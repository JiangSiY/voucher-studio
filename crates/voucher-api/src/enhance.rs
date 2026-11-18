//! 增强缓存管理命令（与 `src-tauri/src/commands/enhance.rs` 对应）。

use voucher_core::services::enhance_cache::{
    self as cache, ClearStrategy, ClearSummary, EnhanceStatusV1, EnhanceStatusV2, HitTrend,
    RecentEnhancedRow,
};
use voucher_core::services::query as qsvc;

use crate::{core, ApiResult, CoreContext};

pub fn enhance_status(ctx: &CoreContext) -> EnhanceStatusV1 {
    cache::status_v1(ctx)
}

pub fn clear_enhance_cache(ctx: &CoreContext) -> ApiResult<u32> {
    core(cache::clear_v1(ctx))
}

pub fn delete_enhanced_one(ctx: &CoreContext, attachment_id: i64) -> ApiResult<u32> {
    core(cache::delete_one(ctx, attachment_id))
}

pub fn recent_enhanced(ctx: &CoreContext, limit: u32) -> ApiResult<Vec<RecentEnhancedRow>> {
    core(cache::recent_enhanced(ctx, limit))
}

pub fn record_viewer_hit(ctx: &CoreContext, attachment_id: i64) -> ApiResult<()> {
    core(cache::record_hit(
        ctx,
        attachment_id,
        qsvc::HitSource::Viewer,
    ))
}

pub fn enhance_status_v2(ctx: &CoreContext) -> ApiResult<EnhanceStatusV2> {
    core(cache::status_v2(ctx))
}

pub fn clear_enhance_cache_v2(
    ctx: &CoreContext,
    strategy: String,
    threshold: u32,
) -> ApiResult<ClearSummary> {
    let s = ClearStrategy::parse(&strategy)
        .ok_or_else(|| crate::ApiError::plain("invalid", format!("未知释放策略: {strategy}")))?;
    core(cache::clear_v2(ctx, s, threshold))
}

pub fn enhance_hit_trend(ctx: &CoreContext, days: u32) -> ApiResult<HitTrend> {
    core(cache::hit_trend(ctx, days))
}
