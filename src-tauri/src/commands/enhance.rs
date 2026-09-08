//! 增强缓存管理命令（v0.6 文档扫描矫正缓存 + v0.6.5 D-2 命中统计）。

use super::*;
// ---------- v0.6 文档扫描矫正（C 步：增强缓存） ----------

/// 增强缓存统计（设置页显示「N 个增强版，共 X MB」）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceStatus {
    pub total: u32,
    pub total_size: u64,
    pub screenshot_count: u32,
    pub scan_count: u32,
}

#[tauri::command]
pub fn enhance_status(ctx: Ctx) -> EnhanceStatus {
    use voucher_core::db::repos::attachment_repo;
    let atts = ctx
        .db
        .with_conn(attachment_repo::list_enhanced)
        .unwrap_or_default();
    let mut total_size: u64 = 0;
    let mut screenshot_count: u32 = 0;
    let mut scan_count: u32 = 0;
    for a in &atts {
        if let Some(rel) = &a.enhanced_path {
            let abs = ctx.paths.attachment_abs(rel);
            if let Ok(m) = std::fs::metadata(&abs) {
                total_size += m.len();
            }
        }
        match a.enhanced_source {
            voucher_core::models::EnhanceSource::Screenshot => screenshot_count += 1,
            _ => scan_count += 1,
        }
    }
    EnhanceStatus {
        total: atts.len() as u32,
        total_size,
        screenshot_count,
        scan_count,
    }
}

/// 清除所有增强版缓存（DB 字段置 NULL + 物理文件删除）。
/// 返回实际删除的文件数（可能 < DB 记录数：文件已被手动删除）
#[tauri::command]
pub fn clear_enhance_cache(ctx: Ctx) -> std::result::Result<u32, String> {
    use voucher_core::db::repos::attachment_repo;
    let atts = err_string(ctx.db.with_conn(attachment_repo::list_enhanced))?;
    let mut deleted = 0u32;
    for a in &atts {
        if let Some(rel) = &a.enhanced_path {
            let abs = ctx.paths.attachment_abs(rel);
            if ctx.paths.is_inside_attachments(&abs) {
                if std::fs::remove_file(&abs).is_ok() {
                    deleted += 1;
                } else {
                    // 文件已不在也视为清理（保留 DB 同步）
                    deleted += 1;
                }
            }
        }
        err_string(
            ctx.db
                .with_conn(|conn| attachment_repo::clear_enhanced(conn, a.id)),
        )?;
    }
    Ok(deleted)
}

// ---------- v0.6.5 D-2 增强缓存管理 UI 升级 ----------

/// 增强缓存状态 v2（设置页增强缓存面板：摘要 + 命中率）
#[tauri::command]
pub fn enhance_status_v2(
    ctx: Ctx,
) -> std::result::Result<voucher_core::services::enhance_cache::EnhanceStatusV2, String> {
    err_string(voucher_core::services::enhance_cache::status_v2(&ctx))
}

/// 按策略释放增强缓存
/// strategy: "all" | "older_than_days" | "larger_than_mb" | "smart"
/// threshold: 阈值（天数 / MB；all 忽略；smart = 宽限期天数，默认 30）
#[tauri::command]
pub fn clear_enhance_cache_v2(
    ctx: Ctx,
    strategy: String,
    threshold: u32,
) -> std::result::Result<voucher_core::services::enhance_cache::ClearSummary, String> {
    let s = voucher_core::services::enhance_cache::ClearStrategy::parse(&strategy)
        .ok_or_else(|| format!("未知释放策略: {strategy}"))?;
    err_string(voucher_core::services::enhance_cache::clear_v2(
        &ctx, s, threshold,
    ))
}

/// 单条释放某张附件的增强版
#[tauri::command]
pub fn delete_enhanced_one(ctx: Ctx, attachment_id: i64) -> std::result::Result<u32, String> {
    err_string(voucher_core::services::enhance_cache::delete_one(
        &ctx,
        attachment_id,
    ))
}

/// 最近增强列表（按文件 mtime 倒序）
#[tauri::command]
pub fn recent_enhanced(
    ctx: Ctx,
    limit: u32,
) -> std::result::Result<Vec<voucher_core::services::enhance_cache::RecentEnhancedRow>, String> {
    err_string(voucher_core::services::enhance_cache::recent_enhanced(
        &ctx, limit,
    ))
}

/// D-7：查看器打开了某附件的增强版 → 记一条 viewer 命中。
/// 附件无增强版时返回错误（前端静默忽略，不影响查看）。
#[tauri::command]
pub fn record_viewer_hit(ctx: Ctx, attachment_id: i64) -> std::result::Result<(), String> {
    err_string(voucher_core::services::enhance_cache::record_hit(
        &ctx,
        attachment_id,
        voucher_core::db::repos::enhance_hit_repo::HitSource::Viewer,
    ))
}

/// D-10：命中趋势（近 N 天按日聚合，ocr / export / viewer 分列，缺日补零）
#[tauri::command]
pub fn enhance_hit_trend(
    ctx: Ctx,
    days: u32,
) -> std::result::Result<voucher_core::services::enhance_cache::HitTrend, String> {
    err_string(voucher_core::services::enhance_cache::hit_trend(&ctx, days))
}
