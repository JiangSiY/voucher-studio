//! 台账字段与会计投影的命令壳（v0.26）。
//!
//! 这两组命令的业务实现在 `voucher-api`（网页版与桌面版共用同一份），
//! 桌面端只做参数解包 + 错误映射，避免两套逻辑漂移。
//! 客户端这两个领域都是同步小查询/小写入（分录投影最多几千行、无网络），
//! 故不 spawn_blocking。

use super::{ApiError, Ctx};
use voucher_core::models::BizFields;

use voucher_api::biz as biz_api;

/// 桌面端命令返回的错误类型（与网页端 `ApiError` 字段一致）
type ApiResult<T> = std::result::Result<T, ApiError>;

/// `voucher-api` 的错误 → 桌面端的错误（保留 code 与必填项清单：前端要按 fields 高亮）
fn conv<T>(r: std::result::Result<T, voucher_api::ApiError>) -> ApiResult<T> {
    r.map_err(|e| ApiError {
        code: e.code,
        message: e.message,
        fields: e.fields,
    })
}

// ---------- 台账字段 ----------

#[tauri::command]
pub fn get_biz_fields(ctx: Ctx<'_>, voucher_id: i64) -> ApiResult<biz_api::BizDetail> {
    conv(biz_api::get_biz_fields(&ctx, voucher_id))
}

#[tauri::command]
pub fn save_biz_fields(
    ctx: Ctx<'_>,
    voucher_id: i64,
    fields: BizFields,
) -> ApiResult<biz_api::BizDetail> {
    conv(biz_api::save_biz_fields(&ctx, voucher_id, fields))
}

#[tauri::command]
pub fn apply_archive_name(ctx: Ctx<'_>, voucher_id: i64) -> ApiResult<biz_api::ArchiveApplied> {
    conv(biz_api::apply_archive_name(&ctx, voucher_id))
}

// ---------- 会计投影 ----------

#[tauri::command]
pub fn gl_build_period(
    ctx: Ctx<'_>,
    period: String,
) -> ApiResult<voucher_core::services::accounting::PeriodPlan> {
    conv(voucher_api::accounting::build_period(&ctx, period))
}

#[tauri::command]
pub fn gl_post_period(
    ctx: Ctx<'_>,
    period: String,
) -> ApiResult<voucher_core::services::accounting::PostResult> {
    conv(voucher_api::accounting::post_period(&ctx, period))
}

#[tauri::command]
pub fn gl_list_entries(
    ctx: Ctx<'_>,
    period: String,
) -> ApiResult<Vec<voucher_core::services::accounting::GlEntry>> {
    conv(voucher_api::accounting::list_entries(&ctx, period))
}

#[tauri::command]
pub fn gl_export_kis(ctx: Ctx<'_>, period: String) -> ApiResult<String> {
    conv(voucher_api::accounting::export_kis(&ctx, period))
}

#[tauri::command]
pub fn gl_export_copy_sheet(ctx: Ctx<'_>, period: String) -> ApiResult<String> {
    conv(voucher_api::accounting::export_copy_sheet(&ctx, period))
}

#[tauri::command]
pub fn gl_export_v5_entries(ctx: Ctx<'_>, period: String) -> ApiResult<String> {
    conv(voucher_api::accounting::export_v5_entries(&ctx, period))
}

#[tauri::command]
pub fn gl_list_category_maps(
    ctx: Ctx<'_>,
) -> ApiResult<Vec<voucher_core::services::accounting::CategoryMap>> {
    conv(voucher_api::accounting::list_category_maps(&ctx))
}

#[tauri::command]
pub fn gl_upsert_category_map(
    ctx: Ctx<'_>,
    map: voucher_core::services::accounting::CategoryMap,
) -> ApiResult<()> {
    conv(voucher_api::accounting::upsert_category_map(&ctx, map)).map(|_| ())
}

#[tauri::command]
pub fn gl_list_account_maps(
    ctx: Ctx<'_>,
) -> ApiResult<Vec<voucher_core::services::accounting::AccountMap>> {
    conv(voucher_api::accounting::list_account_maps(&ctx))
}

#[tauri::command]
pub fn gl_upsert_account_map(
    ctx: Ctx<'_>,
    map: voucher_core::services::accounting::AccountMap,
) -> ApiResult<()> {
    conv(voucher_api::accounting::upsert_account_map(&ctx, map)).map(|_| ())
}

#[tauri::command]
pub fn gl_list_periods(
    ctx: Ctx<'_>,
) -> ApiResult<Vec<voucher_core::services::accounting::PeriodRow>> {
    conv(voucher_api::accounting::list_periods(&ctx))
}

#[tauri::command]
pub fn gl_set_period_status(ctx: Ctx<'_>, period: String, status: String) -> ApiResult<()> {
    conv(voucher_api::accounting::set_period_status(
        &ctx, period, status,
    ))
    .map(|_| ())
}
