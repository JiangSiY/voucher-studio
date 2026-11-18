//! 图片导入流水线（拖放 / 微信监听共用的入库逻辑）
//!
//! 一图一凭证：每张成功入库的图片各建一张凭证（建卡延迟到文件入库成功之后，
//! 重复/失败文件不产生空凭证——根治 G3-3 空卡缺陷）。
//! 流程（每文件独立）：扩展名校验 → MD5 去重 → 复制进附件目录 → 凭证+附件单事务落库 → 可选 OCR 自动填单。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db::repos::attachment_repo;
use crate::models::{Attachment, EnhanceSource, NewAttachment, NewImportBatch, NewVoucher};
use crate::services::{dedup, enhance, imaging, naming, ocr, ocr_fields, voucher_mgr};
use crate::{CoreContext, CoreError, Result};

/// 一次导入产生的一张凭证（一张图片）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedVoucher {
    pub voucher_id: i64,
    pub voucher_no: String,
    pub attachment: Attachment,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    /// 文件导入：本次导入创建的凭证（每张图片一张）
    pub vouchers: Vec<ImportedVoucher>,
    /// 文件导入：内容重复被跳过的文件名
    pub skipped_duplicates: Vec<String>,
    /// 文件导入：失败记录
    pub failed: Vec<ImportFailure>,
    /// 台账导入：已匹配现有凭证数
    #[serde(default)]
    pub matched: usize,
    /// 台账导入：新建凭证数
    #[serde(default)]
    pub created: usize,
    /// 台账导入：跳过/异常行数
    #[serde(default)]
    pub skipped: usize,
    /// 台账导入：错误详情
    #[serde(default)]
    pub errors: Vec<ImportError>,
    /// 台账导入：本次落下的批次 id（dry-run 或整份文件无任何数据行时为 None）
    #[serde(default)]
    pub batch_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFailure {
    pub file_name: String,
    pub reason: String,
}

// 允许扩展名：图片 + PDF
const SUPPORTED_EXTS: &[&str] = &["jpg", "jpeg", "png", "bmp", "webp", "pdf"];

/// 台账清单类型（对应三张 CSV）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LedgerKind {
    /// 收款清单
    Shoukuan,
    /// 付款清单
    Fukuan,
    /// 计提清单
    Jiti,
}

impl LedgerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            LedgerKind::Shoukuan => "shoukuan",
            LedgerKind::Fukuan => "fukuan",
            LedgerKind::Jiti => "jiti",
        }
    }
}

/// 导入图片/PDF。`voucher_id` 为 None 时每张文件各建一张草稿凭证；
/// 为 Some 时附加到指定凭证——仅当该凭证当前无附件（一凭证一附件上限）。
/// `run_ocr`：导入后立即 OCR 并自动填单（失败不影响入库；PDF 不做 OCR）。
fn io_failure_reason(path: &std::path::Path, e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => {
            let p = path.to_string_lossy();
            if p.contains("/Library/Containers/") || p.contains("/Library/Group Containers/") {
                "该文件位于**其他应用的沙盒容器**内（例如微信聊天窗口里的图片/临时文件），\
                 macOS 不允许其它应用读取。请先在来源应用里「另存为」到普通目录（如桌面）再导入；\
                 或为本应用开启「系统设置 → 隐私与安全性 → 完全磁盘访问权限」。"
                    .into()
            } else {
                "无权限读取该文件（macOS 受保护目录需在「系统设置 → 隐私与安全性 → 文件与文件夹」\
                 里授权；或先把文件移到普通目录再导入）"
                    .into()
            }
        }
        std::io::ErrorKind::NotFound => "文件不存在".into(),
        _ => format!("无法读取文件：{e}"),
    }
}

fn is_supported_file(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        SUPPORTED_EXTS.contains(&ext.to_lowercase().as_str())
    } else {
        false
    }
}

fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

/// 估算 PDF 页数：优先读 `/Count N`（Pages 节点），回退统计 `/Type /Page` 出现次数。
/// 仅用于归档元数据展示，不追求精确；解析失败返回 None。
fn parse_pdf_page_count(path: &std::path::Path) -> Option<u32> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    // 1) /Count N —— 取最大的那个（根 Pages 节点的总数通常最大）
    let mut best: Option<u32> = None;
    for (idx, _) in text.match_indices("/Count") {
        let rest = &text[idx + 6..];
        let digits: String = rest
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<u32>() {
            if n > 0 && best.is_none_or(|b| n > b) {
                best = Some(n);
            }
        }
    }
    // 2) 回退：统计 /Type /Page（排除 /Pages）
    if best.is_none() {
        let page_marks = text.matches("/Page").count();
        let pages_marks = text.matches("/Pages").count();
        let n = page_marks.saturating_sub(pages_marks);
        if n > 0 {
            best = Some(n as u32);
        }
    }
    best
}

pub fn import_files(
    ctx: &CoreContext,
    paths: &[PathBuf],
    voucher_id: Option<i64>,
    run_ocr: bool,
) -> Result<ImportSummary> {
    if paths.is_empty() {
        return Err(CoreError::Invalid("未选择任何文件".into()));
    }

    // 附加模式前置校验：目标凭证必须无附件（一凭证一附件上限）
    if let Some(id) = voucher_id {
        let detail = voucher_mgr::get_detail(ctx, id)?;
        if !detail.attachments.is_empty() {
            return Err(CoreError::Invalid(
                "该凭证已有附件，请使用「更换附件」".into(),
            ));
        }
        ctx.db.with_conn(|conn| {
            crate::services::period::ensure_dates_open(
                conn,
                &[detail.voucher.voucher_date.as_str()],
            )
        })?;
    }

    let year_month = crate::utils::paths::current_year_month();
    std::fs::create_dir_all(ctx.paths.attachment_dir_for(&year_month))?;

    let cfg = ctx.config();
    let engine = naming::NamingEngine::new(&cfg.naming.template);

    let mut summary = ImportSummary::default();
    let mut seen_hashes: HashSet<String> = HashSet::new();
    let mut attach_done = false;

    for path in paths {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        if voucher_id.is_some() && attach_done {
            summary.failed.push(ImportFailure {
                reason: "该凭证已有附件（一凭证一附件），仅第一个被导入".into(),
                file_name,
            });
            continue;
        }
        if !is_supported_file(path) {
            summary.failed.push(ImportFailure {
                reason: "不支持的文件类型（仅图片和 PDF）".into(),
                file_name,
            });
            continue;
        }

        match std::fs::metadata(path) {
            Ok(m) if m.is_file() => {}
            Ok(_) => {
                summary.failed.push(ImportFailure {
                    reason: "不是普通文件（目录或特殊文件）".into(),
                    file_name,
                });
                continue;
            }
            Err(e) => {
                summary.failed.push(ImportFailure {
                    reason: io_failure_reason(path, &e),
                    file_name,
                });
                continue;
            }
        }

        let hash = match dedup::md5_file(path) {
            Ok(h) => h,
            Err(e) => {
                let reason = match &e {
                    CoreError::Io(io) => io_failure_reason(path, io),
                    other => other.to_string(),
                };
                summary.failed.push(ImportFailure { reason, file_name });
                continue;
            }
        };

        let dup = seen_hashes.contains(&hash)
            || ctx
                .db
                .with_conn(|conn| dedup::find_duplicate(conn, &hash))?
                .is_some();
        if dup {
            summary.skipped_duplicates.push(file_name);
            continue;
        }
        seen_hashes.insert(hash.clone());

        let ext = ext_of(path).to_lowercase();
        let rel = ctx.paths.attachment_rel_path(&hash, &ext, &year_month);
        let dest = ctx.paths.attachment_abs(&rel);
        if let Err(e) = std::fs::copy(path, &dest) {
            summary.failed.push(ImportFailure {
                reason: format!("复制失败: {e}"),
                file_name,
            });
            continue;
        }
        let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);

        // 判断是否为 PDF
        let is_pdf = ext == "pdf";
        let mut page_count: Option<u32> = None;
        let mut enhanced_rel = None;
        let mut db_source = EnhanceSource::Unknown;

        if is_pdf {
            // PDF：解析页数，不做 OCR、不增强
            page_count = parse_pdf_page_count(&dest);
        } else {
            // 图片：可选增强
            let enhance_cfg = &cfg.enhance;
            let source = enhance::classify_source(path);
            let only_scans = enhance_cfg.only_scans;
            enhanced_rel = if enhance_cfg.enabled
                && !(only_scans && source == enhance::SourceKind::Screenshot)
            {
                let opts = enhance::PipelineOptions::from(enhance_cfg);
                enhance::write_enhanced(&dest, &rel, &opts)
            } else {
                None
            };
            db_source = match source {
                enhance::SourceKind::Screenshot => EnhanceSource::Screenshot,
                enhance::SourceKind::PhotoOrScan => EnhanceSource::Scan,
            };
        }

        // 凭证 + 附件行同一事务
        let created = import_one(
            ctx,
            &engine,
            voucher_id,
            file_name.clone(),
            rel.clone(),
            hash,
            size,
            &ext,
            is_pdf,
            page_count,
        );
        match created {
            Ok(Some((created_voucher_id, voucher_no, mut attachment))) => {
                if voucher_id.is_some() {
                    attach_done = true;
                }
                // 写入增强版路径 + 来源分类（图片才有）
                if !is_pdf {
                    if let Some(rel) = enhanced_rel.as_ref() {
                        if let Err(e) = ctx.db.with_conn(|conn| {
                            attachment_repo::set_enhanced(conn, attachment.id, rel)
                        }) {
                            log::warn!("写入增强版路径失败（附件 {}）: {e}", attachment.id);
                        }
                    }
                    if let Err(e) = ctx.db.with_conn(|conn| {
                        attachment_repo::set_enhanced_source(conn, attachment.id, db_source)
                    }) {
                        log::warn!("写入增强来源失败（附件 {}）: {e}", attachment.id);
                    }
                    attachment.enhanced_path = enhanced_rel.clone();
                    attachment.enhanced_source = db_source;
                }
                attachment.is_pdf = is_pdf;
                attachment.page_count = page_count;

                summary.vouchers.push(ImportedVoucher {
                    voucher_id: created_voucher_id,
                    voucher_no,
                    attachment,
                });
            }
            Ok(None) => {
                summary.skipped_duplicates.push(file_name);
            }
            Err(e) => {
                let _ = std::fs::remove_file(&dest);
                if let Some(er) = &enhanced_rel {
                    let _ = std::fs::remove_file(ctx.paths.attachment_abs(er));
                }
                summary.failed.push(ImportFailure {
                    reason: e.to_string(),
                    file_name,
                });
            }
        }
    }

    // 可选 OCR（仅图片，PDF 跳过）
    if run_ocr {
        let workers = ocr::import_ocr_workers(ctx);
        for chunk in summary.vouchers.chunks(workers) {
            let results: Vec<Result<Attachment>> = std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .filter(|v| !v.attachment.is_pdf)
                    .map(|v| {
                        let id = v.attachment.id;
                        scope.spawn(move || ocr::run_for_attachment(ctx, id))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join().unwrap_or_else(|_| {
                            Err(CoreError::Invalid("OCR 识别线程异常退出".into()))
                        })
                    })
                    .collect()
            });
            for (v, r) in chunk.iter().filter(|v| !v.attachment.is_pdf).zip(results) {
                match r {
                    Ok(att) => {
                        if let Err(e) = ocr_fields::apply_for_attachment(
                            ctx,
                            v.voucher_id,
                            att.id,
                            ocr_fields::FillMode::Import,
                        ) {
                            log::warn!("OCR 自动填单失败（凭证 {}）: {e}", v.voucher_id);
                        }
                    }
                    Err(e) => log::warn!("导入后 OCR 失败（附件 {}）: {e}", v.attachment.id),
                }
            }
        }
    }

    Ok(summary)
}

/// 台账 CSV 导入：按 ledger_ref 幂等建卡/匹配
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerRow {
    // V5 改版后「4 资金流水」的主键是 AutoNumber 的「流水序号」，
    // 旧导出脚本写的是「流水号」——两个名字都认，避免历史 CSV 报废。
    #[serde(alias = "流水号", alias = "流水序号")]
    flow_no: String,
    #[serde(alias = "单据号")]
    doc_no: String,
    #[serde(alias = "日期")]
    date: String,
    #[serde(alias = "金额")]
    amount: String,
    #[serde(alias = "账户")]
    account: String,
    #[serde(alias = "对方")]
    counterparty: String,
    #[serde(alias = "项目")]
    project: String,
    #[serde(alias = "分类")]
    category: String,
    #[serde(alias = "摘要", alias = "备注")]
    summary: String,
    #[serde(alias = "票据类型")]
    invoice_type: String,
    #[serde(alias = "发票号")]
    invoice_no: String,
    #[serde(alias = "类型")]
    ltype: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportError {
    pub line: usize,
    pub ledger_ref: String,
    pub reason: String,
}

pub fn import_ledger_csv(
    ctx: &CoreContext,
    csv_path: &std::path::Path,
    kind: LedgerKind,
    dry_run: bool,
) -> Result<ImportSummary> {
    // 手写 RFC4180 解析（不引入 csv crate：本机沙箱无法下载新依赖）
    let bytes = std::fs::read(csv_path)?;
    let text = decode_csv_text(&bytes);
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.is_empty() {
        return Err(CoreError::Invalid("CSV 为空".into()));
    }
    let header = parse_csv_line(lines.remove(0));
    let idx = |name: &str| header.iter().position(|h| h.trim() == name);

    // 每项是一组可接受的列名（V5 改版后「流水号」变成 AutoNumber「流水序号」，
    // 两种导出并存，任认其一即可）。
    let required: Vec<&[&str]> = match kind {
        LedgerKind::Shoukuan | LedgerKind::Fukuan => vec![
            &["流水号", "流水序号"][..],
            &["日期"],
            &["金额"],
            &["账户"],
            &["对方"],
            &["项目"],
        ],
        LedgerKind::Jiti => vec![
            &["单据号"],
            &["日期"],
            &["金额"],
            &["类型"],
            &["分类"],
            &["对方"],
            &["项目"],
        ],
    };
    for group in &required {
        if !group.iter().any(|col| idx(col).is_some()) {
            return Err(CoreError::Invalid(format!(
                "CSV 缺必需列：{}（表头：{}）",
                group.join("/"),
                header.join(",")
            )));
        }
    }

    let get = |cells: &[String], name: &str| -> String {
        idx(name)
            .and_then(|i| cells.get(i))
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    };

    let mut summary = ImportSummary::default();
    let mut line_no = 0usize;
    let mut rows_total = 0usize;
    // 新建凭证的 id：导入完要一并挂到本批次上（撤销按批进行）
    let mut created_ids: Vec<i64> = Vec::new();

    for raw in lines {
        if raw.trim().is_empty() {
            continue;
        }
        line_no += 1;
        rows_total += 1;
        let cells = parse_csv_line(raw);
        let row = LedgerRow {
            flow_no: {
                let v = get(&cells, "流水序号");
                if v.is_empty() {
                    get(&cells, "流水号")
                } else {
                    v
                }
            },
            doc_no: get(&cells, "单据号"),
            date: get(&cells, "日期"),
            amount: get(&cells, "金额"),
            account: get(&cells, "账户"),
            counterparty: get(&cells, "对方"),
            project: get(&cells, "项目"),
            category: get(&cells, "分类"),
            summary: get(&cells, "摘要"),
            invoice_type: get(&cells, "票据类型"),
            invoice_no: get(&cells, "发票号"),
            ltype: get(&cells, "类型"),
        };

        let ledger_ref = match kind {
            LedgerKind::Shoukuan | LedgerKind::Fukuan => row.flow_no.clone(),
            LedgerKind::Jiti => row.doc_no.clone(),
        };
        if ledger_ref.is_empty() {
            summary.errors.push(ImportError {
                line: line_no,
                ledger_ref: String::new(),
                reason: "幂等键为空".into(),
            });
            summary.skipped += 1;
            continue;
        }

        let existing = ctx.db.with_conn(|conn| {
            crate::db::repos::voucher_repo::find_by_ledger_ref(conn, &ledger_ref)
        })?;

        match existing {
            Some(voucher) => {
                if !dry_run {
                    update_voucher_ledger_tags(ctx, voucher.id, kind)?;
                }
                summary.matched += 1;
            }
            None => {
                if !dry_run {
                    match create_voucher_from_ledger(ctx, &row, kind, &ledger_ref) {
                        Ok(id) => created_ids.push(id),
                        Err(e) => {
                            summary.errors.push(ImportError {
                                line: line_no,
                                ledger_ref: ledger_ref.clone(),
                                reason: e.to_string(),
                            });
                            summary.skipped += 1;
                            continue;
                        }
                    }
                }
                summary.created += 1;
            }
        }
    }

    // 批次只在真正写库时落；dry-run 不留痕（预演不改变任何状态）
    if !dry_run {
        let batch = crate::services::ledger::record_batch(
            ctx,
            &NewImportBatch {
                kind: kind.as_str().to_string(),
                file_name: csv_path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                file_path: csv_path.to_string_lossy().to_string(),
                rows_total: i64::try_from(rows_total).unwrap_or(i64::MAX),
                created: i64::try_from(summary.created).unwrap_or(i64::MAX),
                matched: i64::try_from(summary.matched).unwrap_or(i64::MAX),
                skipped: i64::try_from(summary.skipped).unwrap_or(i64::MAX),
                error_rows: i64::try_from(summary.errors.len()).unwrap_or(i64::MAX),
            },
            &created_ids,
        )?;
        summary.batch_id = Some(batch.id);
    }

    if !summary.errors.is_empty() {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let err_path = ctx.paths.exports.join(format!("import-errors-{stamp}.csv"));
        write_errors_csv(&err_path, &summary.errors)?;
        log::warn!(
            "台账导入异常 {} 行，已落盘：{}",
            summary.errors.len(),
            err_path.display()
        );
    }

    Ok(summary)
}

/// 读 CSV 文本：优先 UTF-8（去 BOM）；失败按 GB18030 解码（微信/网银导出常见）。
pub(crate) fn decode_csv_text(bytes: &[u8]) -> String {
    let trimmed = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(trimmed) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::GB18030.decode(trimmed);
            cow.into_owned()
        }
    }
}

/// 极简 RFC4180 单行解析：支持双引号包裹与 "" 转义。
pub(crate) fn parse_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            }
            '"' if !in_quotes => in_quotes = true,
            ',' if !in_quotes => out.push(std::mem::take(&mut cur)),
            '\r' => {}
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// 解析台账日期：容忍 `2026-10-01` / `2026/10/1` / `2026-10-01 12:00:00` 等写法，
/// 返回（规范化为 `YYYY-MM-DD`、年月 `YYYY-MM`）。
///
/// 空值或无法识别 → 行级错误。**不要**用字符串切片取年月：空日期会让 `[..7]` panic，
/// 整批导入随之中断，而不是记一条错误继续下一行（2026-11-14 实测复现）。
pub(crate) fn parse_ledger_date(raw: &str) -> Result<(String, String)> {
    let invalid = || {
        let shown = raw.trim();
        CoreError::Invalid(if shown.is_empty() {
            "日期为空".to_string()
        } else {
            format!("日期不合法（需 YYYY-MM-DD）：{shown}")
        })
    };
    let head = raw.split_whitespace().next().unwrap_or("");
    let normalized = head.replace('/', "-");
    let parts: Vec<&str> = normalized.split('-').collect();
    if parts.len() != 3 {
        return Err(invalid());
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        parts[0].trim().parse::<i32>(),
        parts[1].trim().parse::<u32>(),
        parts[2].trim().parse::<u32>(),
    ) else {
        return Err(invalid());
    };
    if !(1900..=2200).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(invalid());
    }
    Ok((
        format!("{year:04}-{month:02}-{day:02}"),
        format!("{year:04}-{month:02}"),
    ))
}

fn create_voucher_from_ledger(
    ctx: &CoreContext,
    row: &LedgerRow,
    kind: LedgerKind,
    ledger_ref: &str,
) -> Result<i64> {
    let cfg = ctx.config();
    let engine = crate::services::naming::NamingEngine::new(&cfg.naming.template);
    // 日期先校验：坏行返回行级错误（调用方记入 errors 后继续），既不进事务也不 panic
    let (voucher_date, month) = parse_ledger_date(&row.date)?;

    ctx.db.with_tx(|tx| {
        let today = crate::utils::paths::today();
        let voucher_no = engine.next_no(tx, &today)?;

        let (description, counterparty, amount_cents, tags) = match kind {
            LedgerKind::Shoukuan | LedgerKind::Fukuan => {
                let amt = parse_amount_cents(&row.amount)?;
                let desc = format!("{}-{}-{}", row.counterparty, row.project, ledger_ref);
                let mut tags = vec!["ledger:auto".to_string(), format!("kind:{}", kind.as_str())];
                tags.push(format!("account:{}", row.account));
                tags.push(format!("project:{}", row.project));
                if !row.invoice_type.is_empty() {
                    tags.push(format!("invoice_type:{}", row.invoice_type));
                }
                if !row.invoice_no.is_empty() {
                    tags.push(format!("invoice_no:{}", row.invoice_no));
                }
                (desc, row.counterparty.clone(), Some(amt), tags)
            }
            LedgerKind::Jiti => {
                let amt = parse_amount_cents(&row.amount)?;
                let desc = format!("计提{}-{}-{}", row.ltype, month, ledger_ref);
                let mut tags = vec!["ledger:auto".to_string(), "kind:计提".to_string()];
                tags.push(format!("project:{}", row.project));
                tags.push(format!("category:{}", row.category));
                (desc, row.counterparty.clone(), Some(amt), tags)
            }
        };

        let voucher = crate::db::repos::voucher_repo::insert(
            tx,
            &crate::models::NewVoucher {
                voucher_no,
                voucher_date,
                description,
                counterparty,
                amount_cents,
                status: Some(crate::models::VoucherStatus::Draft),
                tags,
                note: String::new(),
                ledger_ref: Some(ledger_ref.to_string()),
            },
        )?;

        // 返回 id：调用方把新建的凭证挂到本批次（撤销按批进行）
        Ok(voucher.id)
    })
}

fn update_voucher_ledger_tags(ctx: &CoreContext, voucher_id: i64, kind: LedgerKind) -> Result<()> {
    ctx.db.with_tx(|tx| {
        let mut voucher = crate::db::repos::voucher_repo::get(tx, voucher_id)?
            .ok_or(CoreError::NotFound("凭证不存在".into()))?;
        let tag_prefix = format!("kind:{}", kind.as_str());
        voucher.tags.retain(|t| !t.starts_with("kind:"));
        voucher.tags.push(tag_prefix);
        crate::db::repos::voucher_repo::update(
            tx,
            voucher_id,
            &crate::models::VoucherUpdate {
                tags: Some(voucher.tags),
                ..Default::default()
            },
        )?;
        Ok(())
    })
}

pub(crate) fn parse_amount_cents(s: &str) -> Result<i64> {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    let yuan: f64 = cleaned
        .parse()
        .map_err(|_| CoreError::Invalid(format!("金额解析失败：{}", s)))?;
    Ok((yuan * 100.0).round() as i64)
}

fn write_errors_csv(path: &std::path::Path, errors: &[ImportError]) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    f.write_all(b"line,ledger_ref,reason\n")?;
    for e in errors {
        writeln!(
            f,
            "{},{},{}",
            e.line,
            e.ledger_ref,
            e.reason.replace(',', "，")
        )?;
    }
    Ok(())
}

/// 单文件落库：新建凭证或挂到既有凭证
#[allow(clippy::too_many_arguments)]
fn import_one(
    ctx: &CoreContext,
    engine: &naming::NamingEngine,
    attach_to: Option<i64>,
    file_name: String,
    rel: String,
    hash: String,
    size: u64,
    ext: &str,
    is_pdf: bool,
    page_count: Option<u32>,
) -> Result<Option<(i64, String, Attachment)>> {
    let today = crate::utils::paths::today();
    let file_stem = file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_else(|| file_name.clone());

    ctx.db.with_tx(|tx| {
        if dedup::find_duplicate(tx, &hash)?.is_some() {
            return Ok(None);
        }
        let (voucher_id, voucher_no) = match attach_to {
            Some(id) => {
                let v = crate::db::repos::voucher_repo::get(tx, id)?
                    .ok_or_else(|| CoreError::NotFound(format!("凭证 {id} 不存在")))?;
                (id, v.voucher_no)
            }
            None => {
                let voucher_no = engine.next_no(tx, &today)?;
                let v = crate::db::repos::voucher_repo::insert(
                    tx,
                    &NewVoucher {
                        voucher_no,
                        voucher_date: today.clone(),
                        description: file_stem.clone(),
                        counterparty: String::new(),
                        amount_cents: None,
                        status: None,
                        tags: Vec::new(),
                        note: String::new(),
                        ledger_ref: None,
                    },
                )?;
                (v.id, v.voucher_no)
            }
        };
        let mime = if is_pdf {
            "application/pdf".to_string()
        } else {
            imaging::mime_from_ext(ext).to_string()
        };
        let attachment = attachment_repo::insert(
            tx,
            &NewAttachment {
                voucher_id,
                file_name,
                file_path: rel,
                file_hash: hash,
                file_size: size,
                mime_type: mime,
                is_pdf: Some(is_pdf),
                page_count,
            },
        )?;
        // PDF 字段在事务外更新（attachment_repo 需扩展支持 is_pdf/page_count）
        Ok(Some((
            voucher_id,
            voucher_no,
            Attachment {
                id: attachment.id,
                voucher_id: attachment.voucher_id,
                file_name: attachment.file_name,
                file_path: attachment.file_path,
                file_hash: attachment.file_hash,
                file_size: attachment.file_size,
                mime_type: attachment.mime_type,
                ocr_status: attachment.ocr_status,
                ocr_text: attachment.ocr_text,
                enhanced_path: None,
                enhanced_source: EnhanceSource::Unknown,
                is_pdf,
                page_count,
                created_at: attachment.created_at,
                ocr_engine: None,
                file_abs: None,
                enhanced_abs: None,
            },
        )))
    })
}

// 兼容旧接口
pub fn import_images(
    ctx: &CoreContext,
    paths: &[PathBuf],
    voucher_id: Option<i64>,
    run_ocr: bool,
) -> Result<ImportSummary> {
    import_files(ctx, paths, voucher_id, run_ocr)
}

/// 更换凭证图片：删旧图挂新图（单事务 + 快照），旧物理文件随后清理。
/// 新图内容与库中其他凭证图片重复时拒绝；与当前图片内容相同视为幂等成功（不换）。
/// 返回新附件行。
pub fn replace_attachment(
    ctx: &CoreContext,
    voucher_id: i64,
    path: &PathBuf,
) -> Result<Attachment> {
    if !imaging::is_supported(path) {
        return Err(CoreError::Invalid("不支持的文件类型（仅图片）".into()));
    }
    if !path.is_file() {
        return Err(CoreError::Invalid("文件不存在".into()));
    }

    let old = ctx.db.with_conn(|conn| {
        attachment_repo::list_by_voucher(conn, voucher_id)?
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {voucher_id} 无图片可更换")))
    })?;

    let hash = dedup::md5_file(path)?;
    if hash == old.file_hash {
        // 同内容：幂等，不换
        return Ok(old);
    }
    let dup_other = ctx.db.with_conn(|conn| {
        let hit = dedup::find_duplicate(conn, &hash)?;
        Ok(hit.filter(|a| a.id != old.id))
    })?;
    if dup_other.is_some() {
        return Err(CoreError::Invalid("这张图片已存在于其他凭证".into()));
    }

    let year_month = crate::utils::paths::current_year_month();
    std::fs::create_dir_all(ctx.paths.attachment_dir_for(&year_month))?;
    let ext = imaging::ext_of(path);
    let rel = ctx.paths.attachment_rel_path(&hash, &ext, &year_month);
    let dest = ctx.paths.attachment_abs(&rel);
    std::fs::copy(path, &dest)?;
    let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    // 事务：快照 → 删旧附件行 → 插新附件行
    let new_attachment = ctx.db.with_tx(|tx| {
        let current = crate::db::repos::voucher_repo::get(tx, voucher_id)?
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {voucher_id} 不存在")))?;
        // v0.11 期间锁：已封账月份的凭证不可换图
        crate::services::period::ensure_dates_open(tx, &[current.voucher_date.as_str()])?;
        crate::db::repos::snapshot_repo::insert(
            tx,
            &crate::models::snapshot::NewSnapshot {
                voucher_id,
                payload: serde_json::to_value(&current)?,
                reason: "replace_image".into(),
            },
        )?;
        attachment_repo::delete(tx, old.id)?;
        attachment_repo::insert(
            tx,
            &NewAttachment {
                voucher_id,
                file_name,
                file_path: rel,
                file_hash: hash,
                file_size: size,
                mime_type: imaging::mime_from_ext(&ext).to_string(),
                is_pdf: Some(false),
                page_count: None,
            },
        )
    })?;

    // 事务外：清理旧物理文件（含增强版，尽力而为）
    let old_abs = ctx.paths.attachment_abs(&old.file_path);
    if ctx.paths.is_inside_attachments(&old_abs) {
        let _ = std::fs::remove_file(&old_abs);
    }
    if let Some(old_enh) = &old.enhanced_path {
        let enh_abs = ctx.paths.attachment_abs(old_enh);
        if ctx.paths.is_inside_attachments(&enh_abs) {
            let _ = std::fs::remove_file(&enh_abs);
        }
    }
    Ok(new_attachment)
}
