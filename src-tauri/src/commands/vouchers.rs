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
) -> std::result::Result<VoucherListResult, ApiError> {
    let (items, total) = err_api(voucher_mgr::list(&ctx, &query))?;
    // D-1：列表缩略图（每张凭证首附件，enhanced 优先）。空列表时跳过 SQL
    let thumbs: HashMap<i64, qsvc::VoucherThumb> = if items.is_empty() {
        HashMap::new()
    } else {
        let ids: Vec<i64> = items.iter().map(|v| v.id).collect();
        err_api(qsvc::voucher_thumbs(&ctx, &ids))?
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
pub fn get_voucher(ctx: Ctx, id: i64) -> std::result::Result<voucher_mgr::VoucherDetail, ApiError> {
    err_api(voucher_mgr::get_detail(&ctx, id))
}

/// 编辑页上下文导航：同筛选/同排序下的相邻凭证与位置（「保存并下一张」用）
#[tauri::command]
pub fn get_voucher_neighbor(
    ctx: Ctx,
    id: i64,
    query: VoucherQuery,
) -> std::result::Result<qsvc::VoucherNeighbor, ApiError> {
    err_api(voucher_mgr::neighbor(&ctx, id, &query))
}

#[tauri::command]
pub fn update_voucher(
    ctx: Ctx,
    id: i64,
    update: VoucherUpdate,
) -> std::result::Result<voucher_core::models::Voucher, ApiError> {
    err_api(voucher_mgr::update_voucher(&ctx, id, update))
}

/// 批量删除（单事务 + 附件文件清理），返回实际删除数
#[tauri::command]
pub fn delete_vouchers(ctx: Ctx, ids: Vec<i64>) -> std::result::Result<u64, ApiError> {
    err_api(voucher_mgr::delete_vouchers(&ctx, &ids))
}

/// 批量变更状态（单事务逐张快照，reason=batch_status），返回实际变更数
/// （已是目标状态/不存在的跳过不计入）
#[tauri::command]
pub fn batch_update_status(
    ctx: Ctx,
    ids: Vec<i64>,
    status: voucher_core::models::VoucherStatus,
) -> std::result::Result<usize, ApiError> {
    err_api(voucher_mgr::batch_set_status(&ctx, &ids, status))
}

/// 批量编辑（单事务逐张快照，reason=batch_edit）：日期/备注/标签，返回实际变更数
#[tauri::command]
pub fn batch_edit(
    ctx: Ctx,
    ids: Vec<i64>,
    patch: voucher_mgr::BatchEditPatch,
) -> std::result::Result<usize, ApiError> {
    err_api(voucher_mgr::batch_edit(&ctx, &ids, &patch))
}

#[tauri::command]
pub fn voucher_stats(ctx: Ctx) -> std::result::Result<qsvc::VoucherStats, ApiError> {
    err_api(voucher_mgr::stats(&ctx))
}

// ---------- 报表 ----------

/// 近 N 月逐月汇总（默认 6，范围 1–24）
#[tauri::command]
pub fn monthly_summary(
    ctx: Ctx,
    months: Option<u32>,
) -> std::result::Result<Vec<voucher_core::services::reports::MonthlySummary>, ApiError> {
    let n = months.unwrap_or(6).clamp(1, 24);
    err_api(voucher_core::services::reports::monthly_summary(&ctx, n))
}

/// Top 对方账户排行（按金额降序；months=0 表示全部历史，limit 默认 10、范围 1–50）
#[tauri::command]
pub fn top_counterparties(
    ctx: Ctx,
    months: Option<u32>,
    limit: Option<u32>,
) -> std::result::Result<Vec<voucher_core::services::reports::CounterpartyRow>, ApiError> {
    err_api(voucher_core::services::reports::top_counterparties(
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
) -> std::result::Result<Vec<voucher_core::services::reports::TagRow>, ApiError> {
    err_api(voucher_core::services::reports::tag_stats(
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
) -> std::result::Result<Vec<voucher_core::services::reports::DescriptionRow>, ApiError> {
    err_api(voucher_core::services::reports::frequent_descriptions(
        &ctx,
        limit.unwrap_or(8),
    ))
}

// ---------- v0.10 疑似重复检测（同日期 + 同金额）/ v0.16 处置闭环 ----------

/// 单凭证疑似重复（保存后前端提示）：同日期同金额、**尚未判定**的其他凭证 + 证据
#[tauri::command]
pub fn check_voucher_duplicates(
    ctx: Ctx,
    id: i64,
) -> std::result::Result<Vec<qsvc::DuplicateSuspect>, ApiError> {
    err_api(qsvc::suspected_duplicates(&ctx, id))
}

/// v0.16：待核查队列（只含还有未判定对的组；组内对按置信度排序）
#[tauri::command]
pub fn scan_duplicate_review(ctx: Ctx) -> std::result::Result<Vec<qsvc::ReviewGroup>, ApiError> {
    err_api(qsvc::duplicate_review_queue(&ctx))
}

/// v0.16：待处理统计（侧栏徽标 / 仪表盘面板）
#[tauri::command]
pub fn duplicate_pending_summary(ctx: Ctx) -> std::result::Result<qsvc::PendingSummary, ApiError> {
    err_api(qsvc::duplicate_pending_summary(&ctx))
}

/// v0.16：处置历史（新 → 旧；含被删凭证的留痕快照）
#[tauri::command]
pub fn list_duplicate_decisions(
    ctx: Ctx,
    limit: Option<u32>,
) -> std::result::Result<Vec<qsvc::DupDecision>, ApiError> {
    err_api(qsvc::duplicate_decisions(&ctx, limit.unwrap_or(100)))
}

/// v0.16：判定"不是重复"（只写处置记录，不动凭证 → 不受封账限制）；幂等
#[tauri::command]
pub fn resolve_duplicate_pair(
    ctx: Ctx,
    app: AppHandle,
    a_id: i64,
    b_id: i64,
    note: Option<String>,
) -> std::result::Result<i64, ApiError> {
    let id = err_api(voucher_core::services::dup_review::resolve_pair(
        &ctx,
        a_id,
        b_id,
        note.as_deref().unwrap_or(""),
    ))?;
    emit_dup_changed(&app);
    Ok(id)
}

/// v0.16：确认重复，**保留 keep_id、删除 remove_id**（单事务：留痕 + 删除同成同败）；
/// 已封账月份会被拒绝（与列表页删除同口径）
#[tauri::command]
pub fn remove_duplicate_voucher(
    ctx: Ctx,
    app: AppHandle,
    keep_id: i64,
    remove_id: i64,
    note: Option<String>,
) -> std::result::Result<u64, ApiError> {
    let (_decision_id, deleted) = err_api(voucher_core::services::dup_review::remove_duplicate(
        &ctx,
        keep_id,
        remove_id,
        note.as_deref().unwrap_or(""),
    ))?;
    emit_dup_changed(&app);
    Ok(deleted)
}

/// 处置变更后通知各页刷新（沿用 import:done / backup:done 的跨页刷新惯例）
fn emit_dup_changed(app: &AppHandle) {
    use tauri::Emitter;
    let _ = app.emit("dup:changed", serde_json::json!({}));
}

// ---------- v0.18 工作台（仪表盘）只读聚合 ----------

/// 工作台概览：统一窗口口径（`dateFrom`/`dateTo` 为 `YYYY-MM-DD`，含端点）+ 全库固定项；
/// 两端都给时附上一等长窗口的金额/张数（供前端算环比）
#[tauri::command]
pub fn dashboard_overview(
    ctx: Ctx,
    date_from: Option<String>,
    date_to: Option<String>,
    status: Option<voucher_core::models::VoucherStatus>,
) -> std::result::Result<qsvc::DashboardOverview, ApiError> {
    err_api(qsvc::dashboard_overview(&ctx, date_from, date_to, status))
}

/// 待办与健康度（待识别 / 未填金额 / 陈旧草稿 / 未封账往月 / 备份新鲜度 / 疑似重复）
#[tauri::command]
pub fn dashboard_todos(ctx: Ctx) -> std::result::Result<qsvc::DashboardTodos, ApiError> {
    err_api(qsvc::dashboard_todos(&ctx))
}

/// 疑似重复轻量计数（徽标 / 待办卡）：不构造队列，口径与核查队列一致
#[tauri::command]
pub fn duplicate_pending_count(ctx: Ctx) -> std::result::Result<qsvc::DupPendingCount, ApiError> {
    err_api(qsvc::duplicate_pending_count(&ctx))
}

/// v0.18 批次 5：月度透视（`dim` = counterparty | tag；每月前 N 个键，按金额倒序）
#[tauri::command]
pub fn monthly_breakdown(
    ctx: Ctx,
    date_from: Option<String>,
    date_to: Option<String>,
    dim: Option<String>,
    per_month: Option<u32>,
) -> std::result::Result<Vec<qsvc::BreakdownRow>, ApiError> {
    err_api(qsvc::monthly_breakdown(
        &ctx,
        date_from,
        date_to,
        dim.unwrap_or_else(|| "counterparty".into()),
        per_month,
    ))
}

/// v0.18 批次 5：导出覆盖度（已导出 / 从未导出 + 格式分布）
#[tauri::command]
pub fn export_coverage(
    ctx: Ctx,
    date_from: Option<String>,
    date_to: Option<String>,
) -> std::result::Result<qsvc::ExportCoverage, ApiError> {
    err_api(qsvc::export_coverage(&ctx, date_from, date_to))
}

/// v0.18 批次 5：存储占用（`withEnhancedBytes` 开启时才逐文件 stat，较慢）
#[tauri::command]
pub fn storage_usage(
    ctx: Ctx,
    with_enhanced_bytes: Option<bool>,
) -> std::result::Result<qsvc::StorageUsage, ApiError> {
    err_api(qsvc::storage_usage(
        &ctx,
        with_enhanced_bytes.unwrap_or(false),
    ))
}

/// 按当前筛选只取凭证 id（列表页「按筛选全选 N 条」的跨页选择集）
#[tauri::command]
pub fn list_voucher_ids(ctx: Ctx, query: VoucherQuery) -> std::result::Result<Vec<i64>, ApiError> {
    err_api(qsvc::voucher_ids(&ctx, &query))
}

// ---------- v0.11：会计期间锁定（按月封账） ----------

/// 全部已锁期间（设置页期间管理 + 列表/编辑页锁定态判断）
#[tauri::command]
pub fn list_period_locks(ctx: Ctx) -> std::result::Result<Vec<qsvc::PeriodLock>, ApiError> {
    err_api(voucher_core::services::period::list_periods(&ctx))
}

/// 锁定期间（YYYY-MM）：该月凭证禁止增删改
#[tauri::command]
pub fn lock_period(ctx: Ctx, period: String) -> std::result::Result<(), ApiError> {
    err_api(voucher_core::services::period::lock_period(&ctx, &period))
}

/// 解锁期间（设置页需二次确认后调用）
#[tauri::command]
pub fn unlock_period(ctx: Ctx, period: String) -> std::result::Result<(), ApiError> {
    err_api(voucher_core::services::period::unlock_period(&ctx, &period))
}

/// v0.11：年度汇总（该自然年 12 个月，空月补零）
#[tauri::command]
pub fn yearly_summary(
    ctx: Ctx,
    year: i32,
) -> std::result::Result<Vec<voucher_core::services::reports::MonthlySummary>, ApiError> {
    let y = year.clamp(1970, 9999);
    err_api(voucher_core::services::reports::yearly_summary(&ctx, y))
}

// ---------- 搜索 ----------

#[tauri::command]
pub fn search_vouchers(
    ctx: Ctx,
    keyword: String,
    filters: Option<qsvc::SearchFilters>,
    limit: Option<u32>,
) -> std::result::Result<Vec<SearchHit>, ApiError> {
    let f = filters.unwrap_or_default();
    err_api(search::search_vouchers(
        &ctx,
        &keyword,
        &f,
        limit.unwrap_or(50),
    ))
}
