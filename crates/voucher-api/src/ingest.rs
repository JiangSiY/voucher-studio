//! 导入 / 附件 / OCR / 台账命令（与 `src-tauri/src/commands/ingest.rs` 对应）。
//!
//! 与桌面版的差异只有「文件从哪来」：桌面用系统对话框，服务端用 `/api/upload` 落到临时目录后
//! 调这里的 [`import_uploaded`]。其余流程（去重、落盘、建卡、触发 OCR）完全共用

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use voucher_core::models::{Attachment, ImportBatch};
use voucher_core::services::query as qsvc;
use voucher_core::services::{enhance, ingestion, ledger as lsvc, ocr, ocr_fields, voucher_mgr};

use crate::{core, ApiError, ApiResult, CoreContext};

/// 允许扩展名：图片 + PDF（与桌面版同口径）
pub const SUPPORTED_EXTS: &[&str] = &["jpg", "jpeg", "png", "bmp", "webp", "pdf"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerImportArgs {
    pub csv_path: String,
    pub kind: ingestion::LedgerKind,
    #[serde(default)]
    pub dry_run: bool,
}

/// 附件绝对路径（桌面版用于 `<img src>`；网页版改为 `/api/attachments/{id}/original`，
/// 但仍保留命令以兼容既有前端调用点）
pub fn attachment_abs_path(ctx: &CoreContext, id: i64) -> ApiResult<String> {
    let att = core(qsvc::attachment(ctx, id))?;
    match att {
        Some(a) => Ok(ctx
            .paths
            .attachment_abs(&a.file_path)
            .to_string_lossy()
            .to_string()),
        None => Err(ApiError::plain("not_found", format!("附件 {id} 不存在"))),
    }
}

pub fn crop_attachment(
    ctx: &CoreContext,
    attachment_id: i64,
    rect: voucher_core::services::crop::CropRect,
    rotation_deg: i32,
) -> ApiResult<Attachment> {
    core(voucher_core::services::crop::crop_attachment(
        ctx,
        attachment_id,
        rect,
        rotation_deg,
    ))
}

/// 重新识别并自动填单（Manual：只填空字段，不覆盖人工录入）
pub fn run_ocr(ctx: &CoreContext, attachment_id: i64) -> ApiResult<voucher_mgr::VoucherDetail> {
    let att = core(ocr::rerun_for_attachment(ctx, attachment_id))?;
    core(ocr_fields::apply_for_attachment(
        ctx,
        att.voucher_id,
        att.id,
        ocr_fields::FillMode::Manual,
    ))?;
    core(voucher_mgr::get_detail(ctx, att.voucher_id))
}

pub fn ocr_preview_fields(
    ctx: &CoreContext,
    attachment_id: i64,
) -> ApiResult<ocr_fields::ExtractedFields> {
    core(qsvc::ocr_preview(ctx, attachment_id))
}

pub fn run_ocr_pending(ctx: &CoreContext, limit: Option<u32>) -> ApiResult<usize> {
    core(ocr::run_pending(ctx, limit.unwrap_or(50), &|_, _| {}))
}

pub fn re_enhance_all(ctx: &CoreContext) -> ApiResult<serde_json::Value> {
    let (ok, failed) = core(enhance::re_enhance_all(ctx, &|_, _| {}))?;
    Ok(serde_json::json!({ "ok": ok, "failed": failed }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOcrStatus {
    pub available: bool,
    pub engine_name: String,
    pub note: String,
}

pub fn local_ocr_status() -> LocalOcrStatus {
    let macos = cfg!(target_os = "macos");
    LocalOcrStatus {
        available: macos,
        engine_name: "Apple Vision".to_string(),
        note: if macos {
            "本地识别使用 macOS 原生 Vision 框架，无需下载模型".to_string()
        } else {
            "本地识别仅 macOS 可用；其它平台请使用云端识别".to_string()
        },
    }
}

pub fn ocr_health(ctx: &CoreContext) -> voucher_core::OcrHealth {
    ctx.ocr_health()
}

pub fn cli_import_ledger(
    ctx: &CoreContext,
    args: LedgerImportArgs,
) -> ApiResult<ingestion::ImportSummary> {
    let path = PathBuf::from(&args.csv_path);
    if !path.exists() {
        return Err(ApiError::plain(
            "not_found",
            format!("CSV 不存在：{}", args.csv_path),
        ));
    }
    core(ingestion::import_ledger_csv(
        ctx,
        &path,
        args.kind,
        args.dry_run,
    ))
}

pub fn cli_list_pending(ctx: &CoreContext) -> ApiResult<Vec<qsvc::PendingLedgerItem>> {
    core(qsvc::list_pending_ledger(ctx))
}

pub fn ledger_counts(ctx: &CoreContext) -> ApiResult<lsvc::LedgerCounts> {
    core(lsvc::counts(ctx))
}

pub fn list_ledger_items(ctx: &CoreContext, stage: String) -> ApiResult<Vec<lsvc::LedgerItem>> {
    let parsed = core(lsvc::LedgerStage::parse(&stage))?;
    core(lsvc::list_items(ctx, parsed))
}

pub fn list_import_batches(ctx: &CoreContext, limit: Option<u32>) -> ApiResult<Vec<ImportBatch>> {
    core(lsvc::list_batches(ctx, limit.unwrap_or(50)))
}

pub fn undo_import_batch(ctx: &CoreContext, batch_id: i64) -> ApiResult<lsvc::UndoBatchResult> {
    core(lsvc::undo_batch(ctx, batch_id))
}

pub fn set_ledger_booked(ctx: &CoreContext, voucher_id: i64, booked: bool) -> ApiResult<()> {
    core(lsvc::set_booked(ctx, voucher_id, booked))
}

/// 服务端上传入口：把 `/api/upload` 落盘的临时文件当成本地文件导入。
///
/// `attach_to`：给已有凭证补附件（网页版「拖一张图到这个凭证上」）。
pub fn import_uploaded(
    ctx: &CoreContext,
    paths: &[PathBuf],
    attach_to: Option<i64>,
    run_ocr: bool,
) -> ApiResult<ingestion::ImportSummary> {
    core(ingestion::import_files(ctx, paths, attach_to, run_ocr))
}

/// 替换某凭证的附件（换图/换 PDF 不换卡）
pub fn replace_attachment(
    ctx: &CoreContext,
    voucher_id: i64,
    path: &Path,
) -> ApiResult<voucher_mgr::VoucherDetail> {
    let owned = path.to_path_buf();
    core(ingestion::replace_attachment(ctx, voucher_id, &owned))?;
    core(voucher_mgr::get_detail(ctx, voucher_id))
}

/// 服务端对账入口：CSV 已由上传接口落盘
pub fn reconcile_csv(
    ctx: &CoreContext,
    path: &Path,
) -> ApiResult<voucher_core::services::reconcile::ReconcileReport> {
    core(voucher_core::services::reconcile::reconcile(ctx, path))
}
