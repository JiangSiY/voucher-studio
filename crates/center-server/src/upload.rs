//! 上传：`POST /api/upload`（multipart/form-data）→ 落临时目录 → `import_uploaded`。
//!
//! 为什么不用 axum 的 Multipart（axum 未缓存，见 Cargo.toml）：这里手写一个够用的
//! multipart 解析器——只支持本项目会用到的子集（字段 + 文件、无嵌套 multipart），
//! 但严格按 boundary 切分，不做「按名字猜内容」的糊弄实现。
//!
//! 安全边界：整个 body 有硬上限（默认 512 MB），超限直接 413；文件名只取 basename，
//! 路径穿越（`../`）在落盘前被剥掉；临时文件处理完立即删除。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use crate::state::{ApiErr, AppState};

/// 单个请求最大字节数（前后端都按这个值卡；50 张 10MB 的图也够）
pub const MAX_BODY: usize = 512 * 1024 * 1024;

pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub data: Vec<u8>,
}

/// 解析 multipart/form-data。返回 Err 时消息直接给用户看。
pub fn parse_multipart(content_type: &str, body: &[u8]) -> Result<Vec<Part>, String> {
    let boundary = content_type
        .split(';')
        .map(str::trim)
        .find_map(|kv| kv.strip_prefix("boundary="))
        .map(|b| b.trim_matches('"').to_string())
        .ok_or_else(|| "上传缺少 multipart boundary".to_string())?;
    let delim = format!("--{boundary}").into_bytes();
    let mut parts = Vec::new();

    // 先按 boundary 切段：每段形如 "\r\n头\r\n\r\n体\r\n"
    let mut idx = 0usize;
    let mut seg_start: Option<usize> = None;
    while idx + delim.len() <= body.len() {
        if &body[idx..idx + delim.len()] == delim.as_slice() {
            if let Some(s) = seg_start {
                let seg = &body[s..idx];
                if let Some(part) = parse_segment(seg) {
                    parts.push(part);
                }
            }
            // 跳到本行行尾（"--" 表示结束）
            let line_end = find_line_end(body, idx + delim.len());
            if body
                .get(idx + delim.len()..line_end)
                .map(|t| t.starts_with(b"--"))
                .unwrap_or(false)
            {
                break;
            }
            seg_start = Some(line_end);
            idx = line_end;
        } else {
            idx += 1;
        }
    }
    Ok(parts)
}

fn find_line_end(body: &[u8], from: usize) -> usize {
    let mut i = from;
    while i + 1 < body.len() {
        if body[i] == b'\r' && body[i + 1] == b'\n' {
            return i + 2;
        }
        i += 1;
    }
    body.len()
}

fn parse_segment(seg: &[u8]) -> Option<Part> {
    // 头部与体之间隔一个空行
    let split = seg
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)?;
    let head = std::str::from_utf8(&seg[..split]).ok()?;
    let mut name = String::new();
    let mut filename = None;
    for line in head.split("\r\n") {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("content-disposition:") {
            for piece in line.split(';').map(str::trim) {
                if let Some(v) = piece.strip_prefix("name=") {
                    name = v.trim_matches('"').to_string();
                } else if let Some(v) = piece.strip_prefix("filename=") {
                    let v = v.trim_matches('"');
                    if !v.is_empty() {
                        filename = Some(v.to_string());
                    }
                }
            }
        }
    }
    let mut data = seg[split..].to_vec();
    // 段尾是 boundary 前的 "\r\n"，去掉
    if data.ends_with(b"\r\n") {
        data.truncate(data.len() - 2);
    }
    Some(Part {
        name,
        filename,
        data,
    })
}

fn safe_basename(name: &str) -> String {
    Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "upload.bin".into())
        .replace(['/', '\\', '\0'], "_")
}

/// 一次上传的解析结果。`mode` 决定这批文件往哪去（导入 / 补附件 / 换图 / 台账 CSV / 对账）
pub struct Staged {
    pub paths: Vec<PathBuf>,
    pub attach_to: Option<i64>,
    pub run_ocr: bool,
    pub mode: String,
    pub kind: String,
}

/// 保存上传的文件到临时目录，并解析控制字段
pub fn stage_files(st: &Arc<AppState>, parts: &[Part]) -> Result<Staged, ApiErr> {
    let mut paths = Vec::new();
    let mut attach_to: Option<i64> = None;
    let mut run_ocr = true;
    let mut mode = "import".to_string();
    let mut kind = "shoukuan".to_string();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");

    // 先解析控制字段（mode/kind/attachTo/runOcr）：multipart 的段顺序不保证，
    // 文件段在前时若边扫边判断扩展名，会误用默认 mode。
    for p in parts.iter().filter(|p| p.filename.is_none()) {
        let text = String::from_utf8_lossy(&p.data).trim().to_string();
        match p.name.as_str() {
            "attachTo" if !text.is_empty() => attach_to = text.parse().ok(),
            "runOcr" => run_ocr = text != "false" && text != "0",
            "mode" if !text.is_empty() => mode = text,
            "kind" if !text.is_empty() => kind = text,
            _ => {}
        }
    }
    // stage 模式：只落盘拿服务端路径（台账 CSV 先预演再提交，需要两次调用共用同一文件）
    let mut allowed: Vec<&str> = vec!["jpg", "jpeg", "png", "bmp", "webp", "pdf"];
    if mode == "stage" || mode == "ledger_csv" {
        allowed.push("csv");
    }

    for p in parts.iter().filter(|p| p.filename.is_some()) {
        let name = safe_basename(p.filename.as_deref().unwrap_or("upload.bin"));
        let ext = Path::new(&name)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if !allowed.contains(&ext.as_str()) {
            return Err(ApiErr::bad_request(
                "invalid",
                format!("不支持的文件类型：{name}（支持 {}）", allowed.join("/")),
            ));
        }
        let dir = st.ctx.paths.temp.join("uploads");
        std::fs::create_dir_all(&dir)
            .map_err(|e| ApiErr::internal(format!("建临时目录失败：{e}")))?;
        let dest = dir.join(format!("{stamp}-{}-{name}", paths.len() + 1));
        std::fs::write(&dest, &p.data)
            .map_err(|e| ApiErr::internal(format!("写入上传文件失败：{e}")))?;
        paths.push(dest);
    }
    if paths.is_empty() {
        return Err(ApiErr::bad_request("invalid", "没有收到任何文件"));
    }
    Ok(Staged {
        paths,
        attach_to,
        run_ocr,
        mode,
        kind,
    })
}

/// 导入完成后清场（失败也不影响主流程）
pub fn cleanup(paths: &[PathBuf]) {
    for p in paths {
        let _ = std::fs::remove_file(p);
    }
}

/// 同步执行（调用方 spawn_blocking）：按 mode 分派到不同业务入口
pub fn run_staged(st: &Arc<AppState>, staged: &Staged) -> Result<Value, ApiErr> {
    let ctx = &st.ctx;
    let first = || {
        staged
            .paths
            .first()
            .cloned()
            .ok_or_else(|| ApiErr::bad_request("invalid", "没有收到文件"))
    };
    match staged.mode.as_str() {
        // 只落盘、不回执业务结果：调用方随后用返回的 path 调 `/api/cmd/*`
        // （台账 CSV 的「预演 → 写入」两步要共用同一个服务端文件）
        "stage" => Ok(serde_json::json!({
            "paths": staged
                .paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
        })),
        "replace" => {
            let id = staged
                .attach_to
                .ok_or_else(|| ApiErr::bad_request("invalid", "换图必须带 attachTo"))?;
            let detail = voucher_api::ingest::replace_attachment(ctx, id, &first()?)
                .map_err(ApiErr::from)?;
            serde_json::to_value(detail)
                .map_err(|e| ApiErr::internal(format!("结果序列化失败：{e}")))
        }
        "ledger_csv" => {
            let summary = voucher_api::ingest::cli_import_ledger(
                ctx,
                voucher_api::ingest::LedgerImportArgs {
                    csv_path: first()?.to_string_lossy().to_string(),
                    kind: parse_kind(&staged.kind)?,
                    dry_run: false,
                },
            )
            .map_err(ApiErr::from)?;
            serde_json::to_value(summary)
                .map_err(|e| ApiErr::internal(format!("结果序列化失败：{e}")))
        }
        "reconcile" => {
            let report =
                voucher_api::ingest::reconcile_csv(ctx, &first()?).map_err(ApiErr::from)?;
            serde_json::to_value(report)
                .map_err(|e| ApiErr::internal(format!("结果序列化失败：{e}")))
        }
        "attach" => {
            let id = staged
                .attach_to
                .ok_or_else(|| ApiErr::bad_request("invalid", "补附件必须带 attachTo"))?;
            let summary =
                voucher_api::ingest::import_uploaded(ctx, &staged.paths, Some(id), staged.run_ocr)
                    .map_err(ApiErr::from)?;
            serde_json::to_value(summary)
                .map_err(|e| ApiErr::internal(format!("结果序列化失败：{e}")))
        }
        _ => {
            let summary = voucher_api::ingest::import_uploaded(
                ctx,
                &staged.paths,
                staged.attach_to,
                staged.run_ocr,
            )
            .map_err(ApiErr::from)?;
            serde_json::to_value(summary)
                .map_err(|e| ApiErr::internal(format!("结果序列化失败：{e}")))
        }
    }
}

fn parse_kind(k: &str) -> Result<voucher_core::services::ingestion::LedgerKind, ApiErr> {
    use voucher_core::services::ingestion::LedgerKind;
    match k {
        "shoukuan" => Ok(LedgerKind::Shoukuan),
        "fukuan" => Ok(LedgerKind::Fukuan),
        "jiti" => Ok(LedgerKind::Jiti),
        other => Err(ApiErr::bad_request(
            "invalid",
            format!("未知的台账类型：{other}"),
        )),
    }
}
