//! 增强缓存管理命令（与 `src-tauri/src/commands/enhance.rs` 对应）。

use voucher_core::services::enhance_cache::{
    self as cache, EnhanceStatusV1, HitTrend, RecentEnhancedRow,
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

pub fn enhance_hit_trend(ctx: &CoreContext, days: u32) -> ApiResult<HitTrend> {
    core(cache::hit_trend(ctx, days))
}
