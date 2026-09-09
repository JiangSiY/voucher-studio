//! 凭证域命令：列表 / 详情 / 相邻导航 / 更新 / 批量 / 统计 / 报表 / 搜索。
//!
//! 越层约束（体检 #11）：命令层只经 `services::query` 门面与领域服务取数，
//! 不直连仓储层或裸数据库连接（CI grep 门禁固化）。

use super::*;
use voucher_core::models::VoucherUpdate;
use voucher_core::services::query as qsvc;
use voucher_core::services::query::VoucherQuery;
use voucher_core::services::search;
use voucher_core::services::search::SearchHit;
use voucher_core::services::voucher_mgr;
// ---------- 凭证 ----------

#[tauri::command]
pub fn list_vouchers(
    ctx: Ctx,
    query: VoucherQuery,
) -> std::result::Result<VoucherListResult, String> {
    let (items, total) = err_string(voucher_mgr::list(&ctx, &query))?;
    // D-1：列表缩略图（每张凭证首附件，enhanced 优先）。空列表时跳过 SQL
    let thumbs: HashMap<i64, qsvc::VoucherThumb> = if items.is_empty() {
        HashMap::new()
    } else {
        let ids: Vec<i64> = items.iter().map(|v| v.id).collect();
        err_string(qsvc::voucher_thumbs(&ctx, &ids))?
    };
    Ok(VoucherListResult {
        items,
        total,
        thumbs,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherListResult {
    pub items: Vec<voucher_core::models::Voucher>,
    pub total: u64,
    /// D-1：当前页所有凭证的缩略图信息，key = voucher.id
    pub thumbs: HashMap<i64, qsvc::VoucherThumb>,
}

#[tauri::command]
pub fn get_voucher(ctx: Ctx, id: i64) -> std::result::Result<voucher_mgr::VoucherDetail, String> {
    err_string(voucher_mgr::get_detail(&ctx, id))
}

/// 编辑页上下文导航：同筛选/同排序下的相邻凭证与位置（「保存并下一张」用）
#[tauri::command]
pub fn get_voucher_neighbor(
    ctx: Ctx,
    id: i64,
    query: VoucherQuery,
) -> std::result::Result<qsvc::VoucherNeighbor, String> {
    err_string(voucher_mgr::neighbor(&ctx, id, &query))
}

#[tauri::command]
pub fn update_voucher(
    ctx: Ctx,
    id: i64,
    update: VoucherUpdate,
) -> std::result::Result<voucher_core::models::Voucher, String> {
    err_string(voucher_mgr::update_voucher(&ctx, id, update))
}

/// 批量删除（单事务 + 附件文件清理），返回实际删除数
#[tauri::command]
pub fn delete_vouchers(ctx: Ctx, ids: Vec<i64>) -> std::result::Result<u64, String> {
    err_string(voucher_mgr::delete_vouchers(&ctx, &ids))
}

/// 批量变更状态（单事务逐张快照，reason=batch_status），返回实际变更数
/// （已是目标状态/不存在的跳过不计入）
#[tauri::command]
pub fn batch_update_status(
    ctx: Ctx,
    ids: Vec<i64>,
    status: voucher_core::models::VoucherStatus,
) -> std::result::Result<usize, String> {
    err_string(voucher_mgr::batch_set_status(&ctx, &ids, status))
}

/// 批量编辑（单事务逐张快照，reason=batch_edit）：日期/备注/标签，返回实际变更数
#[tauri::command]
pub fn batch_edit(
    ctx: Ctx,
    ids: Vec<i64>,
    patch: voucher_mgr::BatchEditPatch,
) -> std::result::Result<usize, String> {
    err_string(voucher_mgr::batch_edit(&ctx, &ids, &patch))
}

#[tauri::command]
pub fn voucher_stats(ctx: Ctx) -> std::result::Result<qsvc::VoucherStats, String> {
    err_string(voucher_mgr::stats(&ctx))
}

// ---------- 报表 ----------

/// 近 N 月逐月汇总（默认 6，范围 1–24）
#[tauri::command]
pub fn monthly_summary(
    ctx: Ctx,
    months: Option<u32>,
) -> std::result::Result<Vec<voucher_core::services::reports::MonthlySummary>, String> {
    let n = months.unwrap_or(6).clamp(1, 24);
    err_string(voucher_core::services::reports::monthly_summary(&ctx, n))
}

/// Top 对方账户排行（按金额降序；months=0 表示全部历史，limit 默认 10、范围 1–50）
#[tauri::command]
pub fn top_counterparties(
    ctx: Ctx,
    months: Option<u32>,
    limit: Option<u32>,
) -> std::result::Result<Vec<voucher_core::services::reports::CounterpartyRow>, String> {
    err_string(voucher_core::services::reports::top_counterparties(
        &ctx,
        months.unwrap_or(0),
        limit.unwrap_or(10),
    ))
}

/// 标签分类统计（按金额降序；months=0 表示全部历史，limit 默认 10、范围 1–50；
/// 「未分类」行 tag 为空串、固定排末尾且不占 limit 名额）
#[tauri::command]
pub fn tag_stats(
    ctx: Ctx,
    months: Option<u32>,
    limit: Option<u32>,
) -> std::result::Result<Vec<voucher_core::services::reports::TagRow>, String> {
    err_string(voucher_core::services::reports::tag_stats(
        &ctx,
        months.unwrap_or(0),
        limit.unwrap_or(10),
    ))
}

/// v0.10：高频摘要联想（编辑页摘要输入框；模板在前端与 config 合并）
#[tauri::command]
pub fn frequent_descriptions(
    ctx: Ctx,
    limit: Option<u32>,
) -> std::result::Result<Vec<voucher_core::services::reports::DescriptionRow>, String> {
    err_string(voucher_core::services::reports::frequent_descriptions(
        &ctx,
        limit.unwrap_or(8),
    ))
}

// ---------- v0.10：疑似重复检测（同日期 + 同金额） ----------

/// 单凭证疑似重复（保存后前端提示）：同日期同金额的其他凭证
#[tauri::command]
pub fn check_voucher_duplicates(
    ctx: Ctx,
    id: i64,
) -> std::result::Result<Vec<voucher_core::models::Voucher>, String> {
    err_string(qsvc::suspected_duplicates(&ctx, id))
}

/// 全库疑似重复扫描（仪表盘「疑似重复」面板）：同日期同金额、组内 ≥2 张
#[tauri::command]
pub fn scan_duplicates(ctx: Ctx) -> std::result::Result<Vec<qsvc::DuplicateGroup>, String> {
    err_string(qsvc::duplicate_groups(&ctx))
}

// ---------- v0.11：会计期间锁定（按月封账） ----------

/// 全部已锁期间（设置页期间管理 + 列表/编辑页锁定态判断）
#[tauri::command]
pub fn list_period_locks(ctx: Ctx) -> std::result::Result<Vec<qsvc::PeriodLock>, String> {
    err_string(voucher_core::services::period::list_periods(&ctx))
}

/// 锁定期间（YYYY-MM）：该月凭证禁止增删改
#[tauri::command]
pub fn lock_period(ctx: Ctx, period: String) -> std::result::Result<(), String> {
    err_string(voucher_core::services::period::lock_period(&ctx, &period))
}

/// 解锁期间（设置页需二次确认后调用）
#[tauri::command]
pub fn unlock_period(ctx: Ctx, period: String) -> std::result::Result<(), String> {
    err_string(voucher_core::services::period::unlock_period(&ctx, &period))
}

/// v0.11：年度汇总（该自然年 12 个月，空月补零）
#[tauri::command]
pub fn yearly_summary(
    ctx: Ctx,
    year: i32,
) -> std::result::Result<Vec<voucher_core::services::reports::MonthlySummary>, String> {
    let y = year.clamp(1970, 9999);
    err_string(voucher_core::services::reports::yearly_summary(&ctx, y))
}

// ---------- 搜索 ----------

#[tauri::command]
pub fn search_vouchers(
    ctx: Ctx,
    keyword: String,
    filters: Option<qsvc::SearchFilters>,
    limit: Option<u32>,
) -> std::result::Result<Vec<SearchHit>, String> {
    let f = filters.unwrap_or_default();
    err_string(search::search_vouchers(
        &ctx,
        &keyword,
        &f,
        limit.unwrap_or(50),
    ))
}
