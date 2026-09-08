//! 凭证域命令：列表 / 详情 / 相邻导航 / 更新 / 批量 / 统计 / 报表 / 搜索。

use super::*;
use voucher_core::db::repos::attachment_repo;
use voucher_core::db::repos::voucher_repo::VoucherQuery;
use voucher_core::models::VoucherUpdate;
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
    let thumbs: HashMap<i64, VoucherThumb> = if items.is_empty() {
        HashMap::new()
    } else {
        let ids: Vec<i64> = items.iter().map(|v| v.id).collect();
        let rows = err_string(
            ctx.db
                .with_conn(|conn| attachment_repo::first_thumb_for_vouchers(conn, &ids)),
        )?;
        rows.into_iter()
            .map(|(vid, fp, ep, src)| {
                (
                    vid,
                    VoucherThumb {
                        file_path: fp,
                        enhanced_path: ep,
                        enhanced_source: src,
                    },
                )
            })
            .collect()
    };
    Ok(VoucherListResult {
        items,
        total,
        thumbs,
    })
}

/// D-1 列表缩略图信息：相对附件根目录的路径，前端用 convertFileSrc 转换
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct VoucherThumb {
    pub file_path: String,
    pub enhanced_path: Option<String>,
    pub enhanced_source: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherListResult {
    pub items: Vec<voucher_core::models::Voucher>,
    pub total: u64,
    /// D-1：当前页所有凭证的缩略图信息，key = voucher.id
    pub thumbs: HashMap<i64, VoucherThumb>,
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
) -> std::result::Result<voucher_core::db::repos::voucher_repo::VoucherNeighbor, String> {
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
pub fn voucher_stats(
    ctx: Ctx,
) -> std::result::Result<voucher_core::db::repos::voucher_repo::VoucherStats, String> {
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

// ---------- 搜索 ----------

#[tauri::command]
pub fn search_vouchers(
    ctx: Ctx,
    keyword: String,
    limit: Option<u32>,
) -> std::result::Result<Vec<SearchHit>, String> {
    err_string(search::search_vouchers(&ctx, &keyword, limit.unwrap_or(50)))
}
