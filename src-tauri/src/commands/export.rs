//! 导出域命令：PDF 生成与批次记录。

use super::*;
use voucher_core::services::export;
use voucher_core::services::export::ExportOptions;
// ---------- 导出（PDF） ----------

#[tauri::command]
pub async fn export_pdf(
    ctx: Ctx<'_>,
    voucher_ids: Vec<i64>,
    options: Option<ExportOptions>,
) -> std::result::Result<voucher_core::models::ExportBatch, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || match options {
        Some(o) => err_string(export::export_vouchers_pdf(&core, &voucher_ids, &o)),
        None => err_string(export::export_with_defaults(&core, &voucher_ids)),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 按天分组导出：每个日期一份「凭证日报」PDF（一图一凭证的默认导出形态）
#[tauri::command]
pub async fn export_pdf_daily(
    ctx: Ctx<'_>,
    voucher_ids: Vec<i64>,
    options: Option<ExportOptions>,
) -> std::result::Result<Vec<voucher_core::models::ExportBatch>, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || match options {
        Some(o) => err_string(export::export_vouchers_daily(&core, &voucher_ids, &o)),
        None => err_string(export::export_daily_with_defaults(&core, &voucher_ids)),
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn list_export_batches(
    ctx: Ctx,
    limit: Option<u32>,
) -> std::result::Result<Vec<voucher_core::models::ExportBatch>, String> {
    err_string(export::list_batches(&ctx, limit.unwrap_or(50)))
}

/// v0.9 导出历史反查：批次包含的凭证（按导出时排序；已删除的跳过）
#[tauri::command]
pub fn export_batch_vouchers(
    ctx: Ctx,
    batch_id: i64,
) -> std::result::Result<Vec<voucher_core::models::Voucher>, String> {
    err_string(export::batch_vouchers(&ctx, batch_id))
}
