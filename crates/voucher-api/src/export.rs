//! 导出域命令（与 `src-tauri/src/commands/export.rs` 对应）。

use voucher_core::models::{ExportBatch, ExportFormat, Voucher};
use voucher_core::services::export;
use voucher_core::services::export::ExportOptions;

use crate::{core, ApiResult, CoreContext};

pub fn export_pdf_daily(
    ctx: &CoreContext,
    voucher_ids: Vec<i64>,
    options: Option<ExportOptions>,
) -> ApiResult<Vec<ExportBatch>> {
    match options {
        Some(o) => core(export::export_vouchers_daily(ctx, &voucher_ids, &o)),
        None => core(export::export_daily_with_defaults(ctx, &voucher_ids)),
    }
}

pub fn export_table(
    ctx: &CoreContext,
    voucher_ids: Vec<i64>,
    format: ExportFormat,
) -> ApiResult<ExportBatch> {
    core(export::export_vouchers_table(ctx, &voucher_ids, format))
}

pub fn list_export_batches(ctx: &CoreContext, limit: Option<u32>) -> ApiResult<Vec<ExportBatch>> {
    core(export::list_batches(ctx, limit.unwrap_or(50)))
}

pub fn export_batch_vouchers(ctx: &CoreContext, batch_id: i64) -> ApiResult<Vec<Voucher>> {
    core(export::batch_vouchers(ctx, batch_id))
}
