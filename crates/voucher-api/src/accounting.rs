//! 会计投影域命令（M2）：预演、写入、导出金蝶分录。
//!
//! 与 `voucher_core::services::accounting` 一一对应，这里只做「参数 → 服务 → JSON」。
//! 关键约束：**写入前必须预演通过**——`gl_post_period` 内部会重新 build + validate，
//! 前端把 issues 原样展示给用户，不做静默调平。

use std::path::PathBuf;

use voucher_core::services::accounting::{
    self, AccountMap, CategoryMap, GlEntry, PeriodPlan, PeriodRow, PostResult,
};

use crate::{core, ApiResult, CoreContext};

pub fn build_period(ctx: &CoreContext, period: String) -> ApiResult<PeriodPlan> {
    core(ctx.db.with_conn(|c| accounting::build_period(c, &period)))
}

pub fn post_period(ctx: &CoreContext, period: String) -> ApiResult<PostResult> {
    core(accounting::post_period(ctx, &period))
}

pub fn list_entries(ctx: &CoreContext, period: String) -> ApiResult<Vec<GlEntry>> {
    core(accounting::list_entries(ctx, &period))
}

pub fn export_kis(ctx: &CoreContext, period: String) -> ApiResult<String> {
    core(accounting::export_kis_csv(ctx, &period).map(path_string))
}

pub fn export_copy_sheet(ctx: &CoreContext, period: String) -> ApiResult<String> {
    core(accounting::export_copy_sheet(ctx, &period).map(path_string))
}

pub fn export_v5_entries(ctx: &CoreContext, period: String) -> ApiResult<String> {
    core(accounting::export_v5_entries(ctx, &period).map(path_string))
}

pub fn list_category_maps(ctx: &CoreContext) -> ApiResult<Vec<CategoryMap>> {
    core(accounting::list_category_maps(ctx))
}

pub fn upsert_category_map(ctx: &CoreContext, map: CategoryMap) -> ApiResult<serde_json::Value> {
    core(accounting::upsert_category_map(ctx, &map))?;
    Ok(serde_json::json!({ "ok": true }))
}

pub fn list_account_maps(ctx: &CoreContext) -> ApiResult<Vec<AccountMap>> {
    core(accounting::list_account_maps(ctx))
}

pub fn upsert_account_map(ctx: &CoreContext, map: AccountMap) -> ApiResult<serde_json::Value> {
    core(accounting::upsert_account_map(ctx, &map))?;
    Ok(serde_json::json!({ "ok": true }))
}

pub fn list_periods(ctx: &CoreContext) -> ApiResult<Vec<PeriodRow>> {
    core(accounting::list_periods(ctx))
}

pub fn set_period_status(
    ctx: &CoreContext,
    period: String,
    status: String,
) -> ApiResult<serde_json::Value> {
    core(accounting::set_period_status(ctx, &period, &status))?;
    Ok(serde_json::json!({ "ok": true }))
}

fn path_string(p: PathBuf) -> String {
    p.to_string_lossy().to_string()
}
