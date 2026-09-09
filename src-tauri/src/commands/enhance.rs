//! 增强缓存管理命令（v0.6 文档扫描矫正缓存 + v0.6.5 D-2 命中统计）。
//!
//! 越层约束（体检 #11）：命令层只经领域服务（enhance_cache / services::query）
//! 中转，不直连仓储层或裸数据库连接（CI grep 门禁固化）。

use super::*;
use voucher_core::services::query as qsvc;
// ---------- v0.6 文档扫描矫正（C 步：增强缓存） ----------

/// 增强缓存统计 v1（设置页「N 个增强版，共 X MB」；命中口径见 enhance_status_v2）
#[tauri::command]
pub fn enhance_status(ctx: Ctx) -> voucher_core::services::enhance_cache::EnhanceStatusV1 {
    voucher_core::services::enhance_cache::status_v1(&ctx)
}

/// 清除所有增强版缓存 v1（DB 字段置 NULL + 物理文件删除）。
/// 返回实际删除的文件数（可能 < DB 记录数：文件已被手动删除）
#[tauri::command]
pub fn clear_enhance_cache(ctx: Ctx) -> std::result::Result<u32, ApiError> {
    err_api(voucher_core::services::enhance_cache::clear_v1(&ctx))
}

// ---------- v0.6.5 D-2 增强缓存管理 UI 升级 ----------

/// 增强缓存状态 v2（设置页增强缓存面板：摘要 + 命中率）
#[tauri::command]
pub fn enhance_status_v2(
    ctx: Ctx,
) -> std::result::Result<voucher_core::services::enhance_cache::EnhanceStatusV2, ApiError> {
    err_api(voucher_core::services::enhance_cache::status_v2(&ctx))
}

/// 按策略释放增强缓存
/// strategy: "all" | "older_than_days" | "larger_than_mb" | "smart"
/// threshold: 阈值（天数 / MB；all 忽略；smart = 宽限期天数，默认 30）
#[tauri::command]
pub fn clear_enhance_cache_v2(
    ctx: Ctx,
    strategy: String,
    threshold: u32,
) -> std::result::Result<voucher_core::services::enhance_cache::ClearSummary, ApiError> {
    let s = voucher_core::services::enhance_cache::ClearStrategy::parse(&strategy)
        .ok_or_else(|| ApiError::plain("invalid", format!("未知释放策略: {strategy}")))?;
    err_api(voucher_core::services::enhance_cache::clear_v2(
        &ctx, s, threshold,
    ))
}

/// 单条释放某张附件的增强版
#[tauri::command]
pub fn delete_enhanced_one(ctx: Ctx, attachment_id: i64) -> std::result::Result<u32, ApiError> {
    err_api(voucher_core::services::enhance_cache::delete_one(
        &ctx,
        attachment_id,
    ))
}

/// 最近增强列表（按文件 mtime 倒序）
#[tauri::command]
pub fn recent_enhanced(
    ctx: Ctx,
    limit: u32,
) -> std::result::Result<Vec<voucher_core::services::enhance_cache::RecentEnhancedRow>, ApiError> {
    err_api(voucher_core::services::enhance_cache::recent_enhanced(
        &ctx, limit,
    ))
}

/// D-7：查看器打开了某附件的增强版 → 记一条 viewer 命中。
/// 附件无增强版时返回错误（前端静默忽略，不影响查看）。
#[tauri::command]
pub fn record_viewer_hit(ctx: Ctx, attachment_id: i64) -> std::result::Result<(), ApiError> {
    err_api(voucher_core::services::enhance_cache::record_hit(
        &ctx,
        attachment_id,
        qsvc::HitSource::Viewer,
    ))
}

/// D-10：命中趋势（近 N 天按日聚合，ocr / export / viewer 分列，缺日补零）
#[tauri::command]
pub fn enhance_hit_trend(
    ctx: Ctx,
    days: u32,
) -> std::result::Result<voucher_core::services::enhance_cache::HitTrend, ApiError> {
    err_api(voucher_core::services::enhance_cache::hit_trend(&ctx, days))
}
