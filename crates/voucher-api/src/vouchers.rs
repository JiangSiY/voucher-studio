//! 凭证域命令（与 `src-tauri/src/commands/vouchers.rs` 一一对应）。

use std::collections::HashMap;

use serde::Serialize;
use voucher_core::models::{NewVoucher, Voucher, VoucherStatus, VoucherUpdate};
use voucher_core::services::query as qsvc;
use voucher_core::services::query::VoucherQuery;
use voucher_core::services::search;
use voucher_core::services::search::SearchHit;
use voucher_core::services::{dup_review, period, reports, voucher_mgr};

use crate::{core, ApiError, ApiResult, CoreContext};

pub fn create_memo_voucher(ctx: &CoreContext, new_voucher: NewVoucher) -> ApiResult<Voucher> {
    core(voucher_mgr::create_voucher(ctx, new_voucher))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherListResult {
    pub items: Vec<Voucher>,
    pub total: u64,
    pub thumbs: HashMap<i64, qsvc::VoucherThumb>,
}

pub fn list_vouchers(ctx: &CoreContext, query: VoucherQuery) -> ApiResult<VoucherListResult> {
    let (items, total) = core(voucher_mgr::list(ctx, &query))?;
    let thumbs: HashMap<i64, qsvc::VoucherThumb> = if items.is_empty() {
        HashMap::new()
    } else {
        let ids: Vec<i64> = items.iter().map(|v| v.id).collect();
        core(qsvc::voucher_thumbs(ctx, &ids))?
    };
    Ok(VoucherListResult {
        items,
        total,
        thumbs,
    })
}

pub fn get_voucher(ctx: &CoreContext, id: i64) -> ApiResult<voucher_mgr::VoucherDetail> {
    core(voucher_mgr::get_detail(ctx, id))
}

pub fn get_voucher_neighbor(
    ctx: &CoreContext,
    id: i64,
    query: VoucherQuery,
) -> ApiResult<qsvc::VoucherNeighbor> {
    core(voucher_mgr::neighbor(ctx, id, &query))
}

pub fn update_voucher(ctx: &CoreContext, id: i64, update: VoucherUpdate) -> ApiResult<Voucher> {
    core(voucher_mgr::update_voucher(ctx, id, update))
}

pub fn delete_vouchers(ctx: &CoreContext, ids: Vec<i64>) -> ApiResult<u64> {
    core(voucher_mgr::delete_vouchers(ctx, &ids))
}

pub fn batch_update_status(
    ctx: &CoreContext,
    ids: Vec<i64>,
    status: VoucherStatus,
) -> ApiResult<usize> {
    core(voucher_mgr::batch_set_status(ctx, &ids, status))
}

pub fn batch_edit(
    ctx: &CoreContext,
    ids: Vec<i64>,
    patch: voucher_mgr::BatchEditPatch,
) -> ApiResult<usize> {
    core(voucher_mgr::batch_edit(ctx, &ids, &patch))
}

pub fn voucher_stats(ctx: &CoreContext) -> ApiResult<qsvc::VoucherStats> {
    core(voucher_mgr::stats(ctx))
}

pub fn monthly_summary(
    ctx: &CoreContext,
    months: Option<u32>,
) -> ApiResult<Vec<reports::MonthlySummary>> {
    let n = months.unwrap_or(6).clamp(1, 24);
    core(reports::monthly_summary(ctx, n))
}

pub fn top_counterparties(
    ctx: &CoreContext,
    months: Option<u32>,
    limit: Option<u32>,
    date_from: Option<String>,
    date_to: Option<String>,
) -> ApiResult<Vec<reports::CounterpartyRow>> {
    core(reports::top_counterparties(
        ctx,
        months.unwrap_or(0),
        limit.unwrap_or(10),
        date_from.as_deref(),
        date_to.as_deref(),
    ))
}

pub fn tag_stats(
    ctx: &CoreContext,
    months: Option<u32>,
    limit: Option<u32>,
    date_from: Option<String>,
    date_to: Option<String>,
) -> ApiResult<Vec<reports::TagRow>> {
    core(reports::tag_stats(
        ctx,
        months.unwrap_or(0),
        limit.unwrap_or(10),
        date_from.as_deref(),
        date_to.as_deref(),
    ))
}

pub fn frequent_descriptions(
    ctx: &CoreContext,
    limit: Option<u32>,
) -> ApiResult<Vec<reports::DescriptionRow>> {
    core(reports::frequent_descriptions(ctx, limit.unwrap_or(8)))
}

pub fn check_voucher_duplicates(
    ctx: &CoreContext,
    id: i64,
) -> ApiResult<Vec<qsvc::DuplicateSuspect>> {
    core(qsvc::suspected_duplicates(ctx, id))
}

pub fn scan_duplicate_review(ctx: &CoreContext) -> ApiResult<Vec<qsvc::ReviewGroup>> {
    core(qsvc::duplicate_review_queue(ctx))
}

pub fn duplicate_pending_summary(ctx: &CoreContext) -> ApiResult<qsvc::PendingSummary> {
    core(qsvc::duplicate_pending_summary(ctx))
}

pub fn list_duplicate_decisions(
    ctx: &CoreContext,
    limit: Option<u32>,
) -> ApiResult<Vec<qsvc::DupDecision>> {
    core(qsvc::duplicate_decisions(ctx, limit.unwrap_or(100)))
}

pub fn resolve_duplicate_pair(
    ctx: &CoreContext,
    a_id: i64,
    b_id: i64,
    note: Option<String>,
) -> ApiResult<i64> {
    core(dup_review::resolve_pair(
        ctx,
        a_id,
        b_id,
        note.as_deref().unwrap_or(""),
    ))
}

pub fn remove_duplicate_voucher(
    ctx: &CoreContext,
    keep_id: i64,
    remove_id: i64,
    note: Option<String>,
) -> ApiResult<u64> {
    let (_decision_id, deleted) = core(dup_review::remove_duplicate(
        ctx,
        keep_id,
        remove_id,
        note.as_deref().unwrap_or(""),
    ))?;
    Ok(deleted)
}

pub fn dashboard_overview(
    ctx: &CoreContext,
    date_from: Option<String>,
    date_to: Option<String>,
    status: Option<VoucherStatus>,
) -> ApiResult<qsvc::DashboardOverview> {
    core(qsvc::dashboard_overview(ctx, date_from, date_to, status))
}

pub fn dashboard_todos(ctx: &CoreContext) -> ApiResult<qsvc::DashboardTodos> {
    core(qsvc::dashboard_todos(ctx))
}

pub fn data_health(ctx: &CoreContext) -> ApiResult<qsvc::DataHealth> {
    core(qsvc::data_health(ctx))
}

pub fn duplicate_pending_count(ctx: &CoreContext) -> ApiResult<qsvc::DupPendingCount> {
    core(qsvc::duplicate_pending_count(ctx))
}

pub fn monthly_breakdown(
    ctx: &CoreContext,
    date_from: Option<String>,
    date_to: Option<String>,
    dim: Option<String>,
    per_month: Option<u32>,
) -> ApiResult<Vec<qsvc::BreakdownRow>> {
    core(qsvc::monthly_breakdown(
        ctx,
        date_from,
        date_to,
        dim.unwrap_or_else(|| "counterparty".into()),
        per_month,
    ))
}

pub fn export_coverage(
    ctx: &CoreContext,
    date_from: Option<String>,
    date_to: Option<String>,
) -> ApiResult<qsvc::ExportCoverage> {
    core(qsvc::export_coverage(ctx, date_from, date_to))
}

pub fn storage_usage(
    ctx: &CoreContext,
    with_enhanced_bytes: Option<bool>,
) -> ApiResult<qsvc::StorageUsage> {
    core(qsvc::storage_usage(
        ctx,
        with_enhanced_bytes.unwrap_or(false),
    ))
}

pub fn list_voucher_ids(ctx: &CoreContext, query: VoucherQuery) -> ApiResult<Vec<i64>> {
    core(qsvc::voucher_ids(ctx, &query))
}

pub fn list_period_locks(ctx: &CoreContext) -> ApiResult<Vec<qsvc::PeriodLock>> {
    core(period::list_periods(ctx))
}

pub fn lock_period(ctx: &CoreContext, period: String) -> ApiResult<()> {
    core(period::lock_period(ctx, &period))
}

pub fn unlock_period(ctx: &CoreContext, period: String) -> ApiResult<()> {
    core(period::unlock_period(ctx, &period))
}

pub fn yearly_summary(ctx: &CoreContext, year: i32) -> ApiResult<Vec<reports::MonthlySummary>> {
    core(reports::yearly_summary(ctx, year.clamp(1970, 9999)))
}

pub fn search_vouchers(
    ctx: &CoreContext,
    keyword: String,
    filters: Option<qsvc::SearchFilters>,
    limit: Option<u32>,
) -> ApiResult<Vec<SearchHit>> {
    let f = filters.unwrap_or_default();
    core(search::search_vouchers(
        ctx,
        &keyword,
        &f,
        limit.unwrap_or(50),
    ))
}

/// 应用信息（服务端版：路径取自数据目录，版本取 crate 版本）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub db_path: String,
    pub attachments_dir: String,
    pub backups_dir: String,
    pub exports_dir: String,
    pub logs_dir: String,
}

pub fn app_info(ctx: &CoreContext) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: ctx.paths.base.to_string_lossy().to_string(),
        db_path: ctx.paths.db_file().to_string_lossy().to_string(),
        attachments_dir: ctx.paths.attachments.to_string_lossy().to_string(),
        backups_dir: ctx.paths.backups.to_string_lossy().to_string(),
        exports_dir: ctx.paths.exports.to_string_lossy().to_string(),
        logs_dir: ctx.paths.logs.to_string_lossy().to_string(),
    }
}

/// 供服务端在「附件路径越界」时复用同一套错误文案
pub fn not_found(msg: impl Into<String>) -> ApiError {
    ApiError::plain("not_found", msg)
}
