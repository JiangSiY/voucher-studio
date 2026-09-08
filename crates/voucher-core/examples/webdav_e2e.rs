//! WebDAV 真机联调（v0.8 增量结构版）：对真实服务器（坚果云）跑完整协议链路。
//!
//! 用法：cargo run -p voucher-core --example webdav_e2e [-- --keep]
//!
//! 链路（v0.8 云端增量：snapshot + 内容寻址附件包）：
//!   PROPFIND 测试连接 → 增量上传（snapshot + 附件包，附件已存在则 0 附件流量）
//!   → PROPFIND 列表确认 → 二次上传验证「附件未变 → 0 附件流量」
//!   → 下载快照（自动合成 v2 全量 zip）→ inspect 校验 → （默认）DELETE 清理远端测试文件
//!
//! 安全性：全程只操作 voucher-{backup,snapshot,attachments}-*.zip 白名单文件。

use std::path::PathBuf;

use voucher_core::services::backup;
use voucher_core::CoreContext;

fn main() {
    let keep = std::env::args().any(|a| a == "--keep");
    let target = PathBuf::from(std::env::var("APPDATA").expect("APPDATA"))
        .join("VoucherStudio")
        .join("data");
    let ctx = CoreContext::init(Some(target.clone())).expect("init");

    println!(
        "=== WebDAV 真机联调·增量结构（数据目录 {}）===",
        target.display()
    );
    let cfg = ctx.config();
    println!(
        "目标服务器: {} (user: {})",
        cfg.backup.webdav.url, cfg.backup.webdav.username
    );

    let mut pass = 0usize;
    let mut fail = 0usize;

    // ---- 步骤 1：连接测试（PROPFIND Depth:1）----
    match backup::test_webdav(&ctx) {
        Ok(n) => {
            println!("[1] PROPFIND 连接测试: PASS（远端已有 {n} 份可恢复备份）");
            pass += 1;
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("自动创建") {
                println!("[1] PROPFIND 连接测试: PASS（目录不存在，文案指引自动创建）");
                pass += 1;
            } else {
                println!("[1] PROPFIND 连接测试: FAIL（{e}）");
                fail += 1;
            }
        }
    }

    // 联调前远端全量文件名（三类：backup/snapshot/attachments）——清理时只动新增的
    let before = propfind_names_remote(&ctx);

    // ---- 步骤 2：远端列表（解析 + 倒序 + 附件包不外显）----
    let list_before = match backup::list_webdav_backups(&ctx) {
        Ok(rows) => {
            println!("[2] 远端列表解析: PASS（{} 份可恢复备份）", rows.len());
            pass += 1;
            rows.into_iter().map(|r| r.file_name).collect::<Vec<_>>()
        }
        Err(e) => {
            if e.to_string().contains("自动创建") {
                println!("[2] 远端列表解析: PASS（目录尚未创建，上传时自动建）");
                pass += 1;
            } else {
                println!("[2] 远端列表解析: FAIL（{e}）");
                fail += 1;
            }
            Vec::new()
        }
    };

    // ---- 步骤 3：增量上传（snapshot + 附件包）----
    // v0.9 本地备份产物即快照（list_backups 首项 = 最新，快照/全量统一视图）
    let latest = backup::list_backups(&ctx)
        .expect("list backups")
        .into_iter()
        .next()
        .expect("至少要有一个本地备份");
    let local_name = latest.file_name.clone();
    println!("[3] 增量上传本地备份 {local_name}（snapshot + 附件包直连）…");
    match backup::upload_backup_to_webdav(&ctx, std::path::Path::new(&latest.path)) {
        Ok(()) => {
            println!("[3] 增量上传: PASS");
            pass += 1;
        }
        Err(e) => {
            println!("[3] 增量上传: FAIL（{e}）");
            fail += 1;
        }
    }

    // ---- 步骤 4：列表确认快照出现 ----
    let mut uploaded_snapshot: Option<String> = None;
    match backup::list_webdav_backups(&ctx) {
        Ok(rows) => {
            let names: Vec<String> = rows.into_iter().map(|r| r.file_name).collect();
            let new_snapshot = names
                .iter()
                .find(|n| n.starts_with("voucher-snapshot-") && !list_before.contains(n))
                .cloned();
            match new_snapshot {
                Some(s) => {
                    println!("[4] 上传后列表包含新快照: PASS（{s}）");
                    uploaded_snapshot = Some(s);
                    pass += 1;
                }
                None => {
                    println!("[4] 上传后列表包含新快照: FAIL（未出现新 voucher-snapshot-*）");
                    fail += 1;
                }
            }
            // 排序：新→旧
            let mut sorted = names.clone();
            sorted.sort();
            sorted.reverse();
            if sorted == names {
                println!("[4] 列表新→旧排序: PASS");
                pass += 1;
            } else {
                println!("[4] 列表新→旧排序: FAIL");
                fail += 1;
            }
        }
        Err(e) => {
            println!("[4] 上传后列表复查: FAIL（{e}）");
            fail += 1;
        }
    }

    // ---- 步骤 5：二次上传（附件未变）→ 附件包 0 流量（远端附件包数量不变）----
    let bundles_before = before
        .iter()
        .filter(|n| n.starts_with("voucher-attachments-"))
        .count();
    match backup::upload_backup_to_webdav(&ctx, std::path::Path::new(&latest.path)) {
        Ok(()) => {
            let after = propfind_names_remote(&ctx);
            let bundles_after = after
                .iter()
                .filter(|n| n.starts_with("voucher-attachments-"))
                .count();
            if bundles_after == bundles_before
                || bundles_after == bundles_before + 1 && bundles_before == 0
            {
                // 首次联调（before 无附件包）允许 +1；已有则必须不变 = 附件未重传
                println!("[5] 二次上传·附件包不重传: PASS（{bundles_after} 个附件包）");
                pass += 1;
            } else {
                println!(
                    "[5] 二次上传·附件包不重传: FAIL（before={bundles_before} after={bundles_after}，附件包被重复上传？）"
                );
                fail += 1;
            }
        }
        Err(e) => {
            println!("[5] 二次上传: FAIL（{e}）");
            fail += 1;
        }
    }

    // ---- 步骤 6：下载快照（自动拉附件包 + 合成 v2 全量）→ inspect 校验 ----
    if let Some(snap_name) = uploaded_snapshot.clone() {
        // 先移走可能已存在的同时间戳合成产物，避免幂等早退
        let ts = snap_name.trim_start_matches("voucher-snapshot-");
        let composed_name = format!("voucher-backup-{ts}");
        let dest = ctx.paths.backups.join(&composed_name);
        let parked = if dest.exists() {
            std::fs::rename(
                &dest,
                ctx.paths
                    .temp
                    .join(format!("webdav-e2e-park-{composed_name}")),
            )
            .is_ok()
        } else {
            false
        };
        match backup::download_webdav_backup(&ctx, &snap_name) {
            Ok(path) => match backup::inspect_backup(&ctx, std::path::Path::new(&path)) {
                Ok(summary) if summary.checksums_verified => {
                    println!(
                        "[6] 快照下载合成 + 完整性校验: PASS（{} 附件，{} 条目）",
                        summary.attachment_count,
                        summary.restored_files.len()
                    );
                    pass += 1;
                }
                Ok(_) => {
                    println!("[6] 快照下载合成: FAIL（校验未通过）");
                    fail += 1;
                }
                Err(e) => {
                    println!("[6] 快照下载合成: FAIL（{e}）");
                    fail += 1;
                }
            },
            Err(e) => {
                println!("[6] 快照下载: FAIL（{e}）");
                fail += 1;
            }
        }
        // 现场恢复：合成件是有效备份，留在 backups/；还原被移走的原件（若有）
        if parked {
            let _ = std::fs::rename(
                ctx.paths
                    .temp
                    .join(format!("webdav-e2e-park-{composed_name}")),
                ctx.paths
                    .backups
                    .join(format!("{composed_name}.orig-parked")),
            );
        }
    }

    // ---- 步骤 7：非法文件名防护 ----
    match backup::download_webdav_backup(&ctx, "../voucher-backup-evil.zip") {
        Err(e) if e.to_string().contains("非法") => {
            println!("[7] 路径穿越入参拒绝: PASS");
            pass += 1;
        }
        other => {
            println!("[7] 路径穿越入参拒绝: FAIL（{other:?}）");
            fail += 1;
        }
    }

    // ---- 步骤 8（默认）：清理远端测试上传（只删联调前不存在的白名单文件）----
    if !keep {
        let after = propfind_names_remote(&ctx);
        let mut cleaned = 0usize;
        for name in &after {
            if before.contains(name) {
                continue; // 联调前就在云端的绝不动
            }
            // 附件包直接删除（download 只面向快照/全量，不适用于附件包）
            if let Ok(true) = delete_remote(&ctx, name) {
                cleaned += 1;
            }
        }
        if cleaned > 0 {
            println!("[8] 清理远端测试文件: PASS（{cleaned} 个，远端恢复联调前状态）");
            pass += 1;
        } else {
            println!("[8] 清理: SKIP（无新增文件或均已清理）");
        }
    } else {
        println!("[8] 清理: SKIP（--keep 保留云端文件）");
    }

    println!("\n=== 结果：{pass} PASS / {fail} FAIL ===");
    if fail > 0 {
        std::process::exit(1);
    }
}

/// PROPFIND 远端全部白名单文件名（三类），e2e 前后对比用。
/// 与主程序 parse_backup_hrefs 同构：宽松解析 href 尾段。
fn propfind_names_remote(ctx: &CoreContext) -> Vec<String> {
    let cfg = ctx.config();
    let webdav = &cfg.backup.webdav;
    let password = match ctx
        .secrets
        .get(voucher_core::utils::config::SECRET_WEBDAV_PASSWORD)
    {
        Ok(Some(p)) => p,
        _ => return Vec::new(),
    };
    let base = webdav.url.trim().trim_end_matches('/');
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("client");
    let resp = client
        .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), base)
        .basic_auth(&webdav.username, Some(&password))
        .header("Depth", "1")
        .send()
        .expect("propfind");
    let body = resp.text().unwrap_or_default();
    let lower = body.to_lowercase();
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(start) = lower[pos..]
        .find("<d:href")
        .or_else(|| lower[pos..].find("<href"))
    {
        let abs = pos + start;
        let Some(gt) = lower[abs..].find('>') else {
            break;
        };
        let cs = abs + gt + 1;
        let Some(end) = lower[cs..].find("</") else {
            break;
        };
        let ce = cs + end;
        if let Some(name) = body[cs..ce].rsplit('/').next() {
            let ok = (name.starts_with("voucher-backup-")
                || name.starts_with("voucher-snapshot-")
                || name.starts_with("voucher-attachments-"))
                && name.ends_with(".zip")
                && !name.contains("..");
            if ok {
                out.push(name.to_string());
            }
        }
        pos = ce;
    }
    out
}

/// 删除远端单个白名单文件（仅用于联调清理；复用认证与 URL 构造约定）
fn delete_remote(ctx: &CoreContext, file_name: &str) -> voucher_core::Result<bool> {
    let cfg = ctx.config();
    let webdav = &cfg.backup.webdav;
    let password = ctx
        .secrets
        .get(voucher_core::utils::config::SECRET_WEBDAV_PASSWORD)?
        .ok_or_else(|| voucher_core::CoreError::Invalid("WebDAV 密码未设置".into()))?;
    let base = webdav.url.trim().trim_end_matches('/');
    let url = format!("{base}/{file_name}");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let resp = client
        .delete(&url)
        .basic_auth(&webdav.username, Some(&password))
        .send()?;
    Ok(resp.status().is_success())
}
