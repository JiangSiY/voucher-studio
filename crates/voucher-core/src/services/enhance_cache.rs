//! 增强缓存管理服务（v0.6.5 / D-2 · v0.6.6 / D-10 D-11）
//!
//! 包含：
//! - 摘要统计：总数 / 总大小 / 截图 / 扫描件 + 总命中 / 30 天命中
//! - 命中趋势（D-10）：按日聚合近 N 天 ocr/export/viewer 命中，缺日补零
//! - 释放策略：all / older_than_days / larger_than_mb / smart（D-11：零命中 + 宽限期）
//! - 单条释放
//! - 最近增强列表（JOIN 命中次数）

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::db::repos::{attachment_repo, enhance_hit_repo};
use crate::models::{Attachment, EnhanceSource};
use crate::utils::paths::now_iso;
use crate::{CoreContext, CoreError, Result};

/// 设置页摘要 + 命中率
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceStatusV2 {
    pub total: u32,
    pub total_size: u64,
    pub screenshot_count: u32,
    pub scan_count: u32,
    /// 累计命中次数（ocr + export + viewer 三类之和）
    pub hit_count: u64,
    /// 近 30 天命中
    pub hit_count_30d: u64,
    /// D-8：按来源分列——OCR 用增强版识别的次数
    pub hit_count_ocr: u64,
    /// D-8：按来源分列——增强版进 PDF 的次数（D-6 起仅扫描件）
    pub hit_count_export: u64,
    /// D-8：按来源分列——查看器打开增强版的次数（D-7 接通）
    pub hit_count_viewer: u64,
    /// 最老一张增强的「创建时间」（ISO；空 = 没有）
    pub oldest_enhanced_at: Option<String>,
    /// 最新一张增强的「创建时间」
    pub newest_enhanced_at: Option<String>,
}

/// v1 增强缓存摘要（历史命令 enhance_status 的返回形状；新 UI 走 status_v2）
///
/// B2 #11：原 DTO 定义在命令层 src-tauri/commands/enhance.rs，命令层直连仓储
/// 统计后自组 DTO——现收敛到领域服务，命令层仅透传，JSON 契约不变。
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceStatusV1 {
    pub total: u32,
    pub total_size: u64,
    pub screenshot_count: u32,
    pub scan_count: u32,
}

/// 释放策略
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearStrategy {
    All,
    OlderThanDays,
    LargerThanMb,
    /// D-11 智能：零命中且超过宽限期（LRU——最旧优先），命中过的永不清理
    Smart,
}

impl ClearStrategy {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "all" => Some(Self::All),
            "older_than_days" => Some(Self::OlderThanDays),
            "larger_than_mb" => Some(Self::LargerThanMb),
            "smart" => Some(Self::Smart),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::OlderThanDays => "older_than_days",
            Self::LargerThanMb => "larger_than_mb",
            Self::Smart => "smart",
        }
    }
}

/// 释放结果
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearSummary {
    pub deleted_files: u32,
    pub freed_bytes: u64,
    pub strategy: String,
    pub threshold: u32,
    pub candidate_count: u32,
}

/// 最近增强（设置页「最近增强」表格行）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentEnhancedRow {
    pub attachment_id: i64,
    pub voucher_id: i64,
    pub voucher_no: String,
    pub file_name: String,
    pub file_size: u64,
    pub enhanced_size: u64,
    pub enhanced_source: String,
    pub enhanced_at: String,
    pub hit_count: u32,
}

/// 取完整状态（设置页首屏）
pub fn status_v2(ctx: &CoreContext) -> Result<EnhanceStatusV2> {
    let atts = ctx.db.with_conn(attachment_repo::list_enhanced)?;
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
            EnhanceSource::Screenshot => screenshot_count += 1,
            _ => scan_count += 1,
        }
    }
    let (hit_count, hit_count_30d, by_source, oldest, newest) = ctx.db.with_conn(|conn| {
        let total = enhance_hit_repo::count_total(conn)?;
        let since = thirty_days_ago_iso();
        let r30 = enhance_hit_repo::count_since(conn, &since)?;
        let by_source = enhance_hit_repo::count_by_source(conn)?;
        let oldest: Option<String> = conn
            .query_row(
                "SELECT MIN(file_path) FROM attachments WHERE enhanced_path IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        // 取文件修改时间：写增强版的 mtime 不可靠（拷贝时可能变），用 attachment created_at 不准，
        // 简单做法：直接用文件 mtime
        let oldest_at = atts.iter().filter_map(|a| enhanced_mtime_iso(ctx, a)).min();
        let newest_at = atts.iter().filter_map(|a| enhanced_mtime_iso(ctx, a)).max();
        let _ = oldest;
        Ok((total, r30, by_source, oldest_at, newest_at))
    })?;

    Ok(EnhanceStatusV2 {
        total: atts.len() as u32,
        total_size,
        screenshot_count,
        scan_count,
        hit_count,
        hit_count_30d,
        hit_count_ocr: by_source.ocr,
        hit_count_export: by_source.export,
        hit_count_viewer: by_source.viewer,
        oldest_enhanced_at: oldest,
        newest_enhanced_at: newest,
    })
}

/// v1 增强缓存摘要（历史命令 enhance_status 语义保持）：
/// 仅统计增强版条数 / 占用字节 / 截图 / 扫描件；DB 不可读时返回空统计
/// （与旧命令 `with_conn(...).unwrap_or_default()` 的吞错行为一致）。
pub fn status_v1(ctx: &CoreContext) -> EnhanceStatusV1 {
    let mut s = EnhanceStatusV1::default();
    let atts = match ctx.db.with_conn(attachment_repo::list_enhanced) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("enhance_status(v1): 读取增强列表失败: {e}");
            return s;
        }
    };
    for a in &atts {
        if let Some(rel) = &a.enhanced_path {
            let abs = ctx.paths.attachment_abs(rel);
            if let Ok(m) = std::fs::metadata(&abs) {
                s.total_size += m.len();
            }
        }
        match a.enhanced_source {
            EnhanceSource::Screenshot => s.screenshot_count += 1,
            _ => s.scan_count += 1,
        }
    }
    s.total = atts.len() as u32;
    s
}

/// v1 清除增强缓存（历史命令 clear_enhance_cache 语义保持）：
/// DB 字段置 NULL + 物理文件删除。返回实际删除的文件数
/// （可能 < DB 记录数：文件已被手动删除；逐条独立写、沿用历史无事务语义）。
pub fn clear_v1(ctx: &CoreContext) -> Result<u32> {
    let atts = ctx.db.with_conn(attachment_repo::list_enhanced)?;
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
        ctx.db
            .with_conn(|conn| attachment_repo::clear_enhanced(conn, a.id))?;
    }
    Ok(deleted)
}

/// D-10：趋势看板单日点（ocr / export / viewer 三类命中 + 合计）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HitTrendPoint {
    /// 本地日期 YYYY-MM-DD
    pub day: String,
    pub ocr: u64,
    pub export: u64,
    pub viewer: u64,
    pub total: u64,
}

/// D-10：命中趋势（近 N 天，缺日补零，含峰值供前端定标柱高）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HitTrend {
    pub days: u32,
    pub points: Vec<HitTrendPoint>,
    /// 区间内单日合计峰值（前端画柱用；全零时为 0）
    pub max_total: u64,
}

/// D-10：近 N 天命中趋势（按本地日期补零对齐）
pub fn hit_trend(ctx: &CoreContext, days: u32) -> Result<HitTrend> {
    use chrono::TimeZone;
    let days = days.clamp(1, 90);
    let today = chrono::Local::now().date_naive();
    let start = today - chrono::Duration::days((days - 1) as i64);
    // since 取「本地 0 点」对应的 UTC 瞬间（hit_at 存 UTC；直接拼 00:00Z 会漏掉东八区 0-8 点的命中）
    let start_local_midnight = start
        .and_hms_opt(0, 0, 0)
        .and_then(|t| chrono::Local.from_local_datetime(&t).single())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);
    let since_iso = start_local_midnight
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    let raw = ctx
        .db
        .with_conn(|conn| enhance_hit_repo::daily_counts(conn, &since_iso))?;
    let by_day: std::collections::HashMap<String, enhance_hit_repo::DailyHitCount> =
        raw.into_iter().map(|d| (d.day.clone(), d)).collect();

    let mut points = Vec::with_capacity(days as usize);
    let mut max_total: u64 = 0;
    for i in 0..days {
        let day = (start + chrono::Duration::days(i as i64))
            .format("%Y-%m-%d")
            .to_string();
        let e = by_day.get(&day);
        let (ocr, export, viewer) = match e {
            Some(d) => (d.ocr, d.export, d.viewer),
            None => (0, 0, 0),
        };
        let total = ocr + export + viewer;
        max_total = max_total.max(total);
        points.push(HitTrendPoint {
            day,
            ocr,
            export,
            viewer,
            total,
        });
    }

    Ok(HitTrend {
        days,
        points,
        max_total,
    })
}

/// D-7：外部（查看器等）使用某附件的增强版时记一条命中。
/// 仅当附件存在且有 enhanced_path 才计入——与「看的是增强版」的口径一致；
/// 无增强版返回 Invalid（前端静默忽略即可，不弹错误）。
pub fn record_hit(
    ctx: &CoreContext,
    attachment_id: i64,
    source: enhance_hit_repo::HitSource,
) -> Result<()> {
    ctx.db.with_conn(|conn| {
        let a = attachment_repo::get(conn, attachment_id)?
            .ok_or_else(|| CoreError::NotFound(format!("附件 {attachment_id} 不存在")))?;
        if a.enhanced_path.is_none() {
            return Err(CoreError::Invalid("该附件无增强版，不计命中".into()));
        }
        enhance_hit_repo::record(conn, attachment_id, source)
    })
}

/// 释放单个附件的增强版（DB 字段 + 物理文件）
pub fn delete_one(ctx: &CoreContext, attachment_id: i64) -> Result<u32> {
    let a = ctx
        .db
        .with_conn(|conn| attachment_repo::get(conn, attachment_id))?
        .ok_or_else(|| crate::CoreError::NotFound(format!("附件 {attachment_id} 不存在")))?;
    let rel = match &a.enhanced_path {
        Some(p) => p.clone(),
        None => return Ok(0), // 没增强过
    };
    let abs = ctx.paths.attachment_abs(&rel);
    let mut deleted = 0u32;
    if ctx.paths.is_inside_attachments(&abs) {
        if std::fs::remove_file(&abs).is_ok() {
            deleted = 1;
        } else if !abs.exists() {
            // 文件已被手动删除，视为清理
            deleted = 1;
        }
    }
    ctx.db
        .with_conn(|conn| attachment_repo::clear_enhanced(conn, attachment_id))?;
    // hits 级联删除由 FK ON DELETE CASCADE 保证
    Ok(deleted)
}

/// 释放：按策略
/// threshold 含义：all 忽略 / older_than_days 阈值天数 / larger_than_mb 阈值 MB /
/// smart 宽限期天数（零命中且早于该天数才清）
pub fn clear_v2(
    ctx: &CoreContext,
    strategy: ClearStrategy,
    threshold: u32,
) -> Result<ClearSummary> {
    // D-11：Smart 需要命中集合——单连接互斥锁不可重入，必须在外层 with_conn 之前取
    let hit_ids: Option<std::collections::HashSet<i64>> = (strategy == ClearStrategy::Smart)
        .then(|| {
            ctx.db
                .with_conn(enhance_hit_repo::count_grouped_by_attachment)
                .map(|rows| rows.into_iter().map(|(id, _n)| id).collect())
        })
        .transpose()?;

    let candidates = ctx.db.with_conn(|conn| match strategy {
        ClearStrategy::All => attachment_repo::list_enhanced(conn),
        ClearStrategy::OlderThanDays => {
            // 「早于 N 天」用文件 mtime 判定（写盘时间比 created_at 准）
            let since_ts = days_ago_ts(threshold);
            list_enhanced_filtered(conn, |a| ctx_for_mtime_filter(a, ctx, since_ts))
        }
        ClearStrategy::LargerThanMb => {
            let min_bytes = (threshold as u64) * 1024 * 1024;
            list_enhanced_filtered(conn, |a| enhanced_size_bytes(a, ctx) >= min_bytes)
        }
        ClearStrategy::Smart => {
            // D-11：零命中且超过宽限期（threshold 天）→ 淘汰；
            // 命中过的（hit_count > 0）证明有真实收益，永不进入候选。
            let since_ts = days_ago_ts(threshold.max(1));
            let hits = hit_ids.unwrap_or_default();
            list_enhanced_filtered(conn, |a| {
                !hits.contains(&a.id) && ctx_for_mtime_filter(a, ctx, since_ts)
            })
        }
    })?;

    let candidate_count = candidates.len() as u32;
    let mut deleted_files: u32 = 0;
    let mut freed_bytes: u64 = 0;
    // Smart 按 mtime 最旧优先（LRU：最久没被生成/使用的先走）
    let mut candidates = candidates;
    if strategy == ClearStrategy::Smart {
        candidates.sort_by_key(|a| {
            a.enhanced_path
                .as_ref()
                .and_then(|rel| std::fs::metadata(ctx.paths.attachment_abs(rel)).ok())
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0)
        });
    }
    for a in &candidates {
        if let Some(rel) = &a.enhanced_path {
            let abs = ctx.paths.attachment_abs(rel);
            if ctx.paths.is_inside_attachments(&abs) {
                if let Ok(m) = std::fs::metadata(&abs) {
                    freed_bytes += m.len();
                }
                if std::fs::remove_file(&abs).is_ok() {
                    deleted_files += 1;
                } else if !abs.exists() {
                    // 文件已不存在也算清理
                    deleted_files += 1;
                }
            }
        }
        ctx.db
            .with_conn(|conn| attachment_repo::clear_enhanced(conn, a.id))?;
    }

    Ok(ClearSummary {
        deleted_files,
        freed_bytes,
        strategy: strategy.as_str().to_string(),
        threshold,
        candidate_count,
    })
}

/// 最近增强列表（按文件 mtime DESC）
pub fn recent_enhanced(ctx: &CoreContext, limit: u32) -> Result<Vec<RecentEnhancedRow>> {
    let atts = ctx.db.with_conn(attachment_repo::list_enhanced)?;
    let grouped: std::collections::HashMap<i64, u32> = ctx
        .db
        .with_conn(enhance_hit_repo::count_grouped_by_attachment)?
        .into_iter()
        .collect();

    // 加载 vouchers（拿 voucher_no）
    let mut rows: Vec<RecentEnhancedRow> = Vec::with_capacity(atts.len());
    for a in atts {
        let voucher = ctx
            .db
            .with_conn(|conn| crate::db::repos::voucher_repo::get(conn, a.voucher_id))?;
        let (voucher_no, voucher_id) = match voucher {
            Some(v) => (v.voucher_no, v.id),
            None => (String::from("(孤儿附件)"), a.voucher_id),
        };
        let enhanced_size = enhanced_size_bytes(&a, ctx);
        let enhanced_at = enhanced_mtime_iso(ctx, &a).unwrap_or_else(|| a.created_at.clone());
        rows.push(RecentEnhancedRow {
            attachment_id: a.id,
            voucher_id,
            voucher_no,
            file_name: a.file_name.clone(),
            file_size: a.file_size,
            enhanced_size,
            enhanced_source: a.enhanced_source.as_str().to_string(),
            enhanced_at,
            hit_count: *grouped.get(&a.id).unwrap_or(&0),
        });
    }
    // 按 enhanced_at 倒序
    rows.sort_by(|x, y| y.enhanced_at.cmp(&x.enhanced_at));
    rows.truncate(limit as usize);
    Ok(rows)
}

// ----- 内部辅助 -----

fn enhanced_size_bytes(a: &Attachment, ctx: &CoreContext) -> u64 {
    if let Some(rel) = &a.enhanced_path {
        let abs = ctx.paths.attachment_abs(rel);
        std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    }
}

fn enhanced_mtime_iso(ctx: &CoreContext, a: &Attachment) -> Option<String> {
    let rel = a.enhanced_path.as_ref()?;
    let abs = ctx.paths.attachment_abs(rel);
    let m = std::fs::metadata(&abs).ok()?;
    let mtime = m.modified().ok()?;
    let dt: chrono::DateTime<chrono::Utc> = mtime.into();
    Some(dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

fn ctx_for_mtime_filter(a: &Attachment, ctx: &CoreContext, since_ts_secs: i64) -> bool {
    let Some(rel) = &a.enhanced_path else {
        return false;
    };
    let abs = ctx.paths.attachment_abs(rel);
    let Ok(m) = std::fs::metadata(&abs) else {
        return false;
    };
    let Ok(mtime) = m.modified() else {
        return false;
    };
    let secs = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    secs < since_ts_secs
}

fn list_enhanced_filtered(
    conn: &Connection,
    pred: impl Fn(&Attachment) -> bool,
) -> Result<Vec<Attachment>> {
    let all = attachment_repo::list_enhanced(conn)?;
    Ok(all.into_iter().filter(pred).collect())
}

fn days_ago_ts(days: u32) -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    now - (days as i64) * 86_400
}

fn thirty_days_ago_iso() -> String {
    use chrono::{Duration, Utc};
    let now = Utc::now() - Duration::days(30);
    now.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

// 仅用于确保 now_iso 的导入（让 chrono 参与编译）
#[allow(dead_code)]
fn _force_now_iso_link() -> String {
    now_iso()
}

// 防止 warning：实际并未用 Path 导入，但保留签名兼容旧版
#[allow(dead_code)]
fn _path_marker(_: &Path) {}
