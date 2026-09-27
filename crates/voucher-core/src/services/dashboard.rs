//! 工作台（仪表盘）只读聚合（v0.18）
//!
//! # 为什么单列一个服务
//!
//! 旧仪表盘的核心病灶是**同一屏三套时间口径**：统计卡走全库（`voucher_stats`）、
//! 状态分布只算「近 6 月窗口」、Top 排行是全历史前 10，而三处都不标注口径——
//! 用户看到"草稿 12 / 分布里草稿 5"会以为数据错了。
//!
//! 本模块给新工作台提供**一个统一的窗口口径**：
//! - `overview(date_from, date_to, status)`：窗口内的张数/金额/状态分布，
//!   并附上一等长窗口的金额用于算环比（不足两个窗口时不编造，交由前端隐藏）；
//! - `todos()`：把**已落库但没有出口**的数据变成可行动的待办（待识别、未填金额、
//!   陈旧草稿、未封账月、备份新鲜度、疑似重复）——每项都能点进对应的筛选列表；
//! - 全库口径的固定项（总数/附件数/本月新增）沿用 `voucher_stats` 的语义，单独成字段，
//!   由界面显式标注「全库」。
//!
//! # 口径纪律（改动前先读）
//!
//! - 所有时间比较都用**字符串前缀比较**（`voucher_date` 是 `YYYY-MM-DD` 定长文本），
//!   与既有查询一致，不引入日期函数（避免索引失效与格式歧义）。
//! - 金额一律以**分**为单位（`i64`），空金额不计入合计（`SUM` 忽略 NULL）。
//! - `todos` 里**不含**库完整性/附件文件缺失——那两项由启动时的健康检查缓存
//!   （`services::recovery`）提供，避免每次打开工作台都全表 stat 一遍。

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::models::enums::VoucherStatus;
use crate::services::{dup_review, reports};
use crate::{CoreContext, Result};

/// 窗口内的状态分布
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatusCount {
    pub status: String,
    pub count: u64,
    pub amount_cents: i64,
}

/// 窗口口径的概览（工作台首屏统计）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    /// 本次口径（回显给界面，用于在标题旁标注"口径：本月 · 全部状态"）
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub status: Option<String>,
    /// 窗口内张数与金额合计（分）
    pub count: u64,
    pub amount_cents: i64,
    /// 上一等长窗口的金额与张数（`window_prev` 为 None 时表示无法计算环比）
    pub prev_amount_cents: Option<i64>,
    pub prev_count: Option<u64>,
    /// 窗口内按状态分布
    ///
    /// **口径**：分布始终反映**窗口内的完整构成**，不随顶部的状态筛选收窄——
    /// 它服务于"堆叠条"这一视觉（选中某一档时高亮该段），若跟着筛选收窄就只剩一根柱子、
    /// 失去构成信息。界面上该块的标题须标注口径（如"本月 · 全部状态"）。
    pub by_status: Vec<StatusCount>,
    /// 全库口径（界面须标注"全库"）
    pub total: u64,
    pub attachments: u64,
    pub this_month: u64,
}

/// 待办与健康度（每项都可点进对应筛选；数值为 0 时界面隐藏该项）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Todos {
    /// 待识别附件数（`ocr_status IN ('pending','failed')`）
    pub ocr_pending: u64,
    /// 金额为空（`amount_cents IS NULL`）
    pub amount_missing: u64,
    /// 陈旧草稿（`status='draft'` 且 `updated_at` 早于 N 天）
    pub stale_drafts: u64,
    /// 陈旧草稿的阈值天数（回显给文案用）
    pub stale_draft_days: u32,
    /// 有凭证但未封账的往月数（不含当月）
    pub unlocked_months: u64,
    /// 最近一份本地备份（文件名 + 时间；仅目录扫描，不读 manifest）
    pub latest_backup: Option<BackupLite>,
    /// 最近备份距今是否超过一天（界面据此提示"该备份了"）
    pub backup_stale: bool,
    /// 疑似重复：未判定对数与其中高置信度对数（轻量计数）
    pub dup_pairs: u32,
    pub dup_high_pairs: u32,
    pub dup_groups: u32,
}

/// 备份摘要（轻量：只看目录里的文件名与时间，不读/不解密 manifest）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupLite {
    pub file_name: String,
    pub modified_at: String,
}

/// 默认"陈旧草稿"阈值（天）：一周没人动过的草稿值得提醒
pub const STALE_DRAFT_DAYS: u32 = 7;

/// 备份新鲜度阈值（小时）：超过一天未备份即视为陈旧
const BACKUP_STALE_HOURS: i64 = 24;

/// 用 LIKE 前缀比较实现窗口过滤（`voucher_date` 为定长 `YYYY-MM-DD`）
fn window_where(
    date_from: Option<&str>,
    date_to: Option<&str>,
    status: Option<VoucherStatus>,
) -> (String, Vec<String>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut bind: Vec<String> = Vec::new();
    if let Some(s) = status {
        clauses.push(format!("status = ?{}", bind.len() + 1));
        bind.push(s.as_str().to_string());
    }
    if let Some(d) = date_from.map(str::trim).filter(|d| !d.is_empty()) {
        clauses.push(format!("voucher_date >= ?{}", bind.len() + 1));
        bind.push(d.to_string());
    }
    if let Some(d) = date_to.map(str::trim).filter(|d| !d.is_empty()) {
        clauses.push(format!("voucher_date <= ?{}", bind.len() + 1));
        bind.push(d.to_string());
    }
    let sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    (sql, bind)
}

fn window_totals(
    conn: &Connection,
    date_from: Option<&str>,
    date_to: Option<&str>,
    status: Option<VoucherStatus>,
) -> Result<(u64, i64)> {
    let (where_sql, bind) = window_where(date_from, date_to, status);
    let (count, amount): (i64, Option<i64>) = conn.query_row(
        &format!("SELECT COUNT(*), SUM(amount_cents) FROM vouchers {where_sql}"),
        rusqlite::params_from_iter(bind.iter()),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok((count as u64, amount.unwrap_or(0)))
}

/// 计算"上一等长窗口"：给出 (from, to) 闭区间，返回上一个等长区间。
///
/// 说明：只做**天数**等长（不按月对齐）——月以来回推一个月的口径在月份长度不齐时
/// 会给出"上上月 30 天 vs 本月 31 天"，反而更像错的。天数等长是可比性最稳的口径。
fn prev_window(from: &str, to: &str) -> Option<(String, String)> {
    let from_d = chrono::NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
    let to_d = chrono::NaiveDate::parse_from_str(to, "%Y-%m-%d").ok()?;
    let span = (to_d - from_d).num_days();
    if span < 0 {
        return None;
    }
    let prev_to = from_d.pred_opt()?;
    let prev_from = prev_to - chrono::Duration::days(span);
    Some((
        prev_from.format("%Y-%m-%d").to_string(),
        prev_to.format("%Y-%m-%d").to_string(),
    ))
}

/// 工作台概览（窗口口径 + 全库固定项）
///
/// `date_from`/`date_to` 为 `YYYY-MM-DD`（含端点，任一为空表示该端不限制）；
/// 两端都给时才会计算环比——否则"上一窗口"无意义。
pub fn overview(
    ctx: &CoreContext,
    date_from: Option<String>,
    date_to: Option<String>,
    status: Option<VoucherStatus>,
) -> Result<Overview> {
    let started = std::time::Instant::now();
    let out = ctx.db.with_conn(|conn| {
        let (count, amount_cents) =
            window_totals(conn, date_from.as_deref(), date_to.as_deref(), status)?;

        let (prev_count, prev_amount_cents) = match (date_from.as_deref(), date_to.as_deref()) {
            (Some(f), Some(t)) => match prev_window(f, t) {
                Some((pf, pt)) => {
                    let (c, a) = window_totals(conn, Some(&pf), Some(&pt), status)?;
                    (Some(c), Some(a))
                }
                None => (None, None),
            },
            _ => (None, None),
        };

        // 窗口内状态分布
        let (where_sql, bind) = window_where(date_from.as_deref(), date_to.as_deref(), None);
        let mut by_status: Vec<StatusCount> = Vec::new();
        {
            let sql = format!(
                "SELECT status, COUNT(*), COALESCE(SUM(amount_cents),0) FROM vouchers {where_sql} \
                 GROUP BY status ORDER BY COUNT(*) DESC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query(rusqlite::params_from_iter(bind.iter()))?;
            while let Some(r) = rows.next()? {
                by_status.push(StatusCount {
                    status: r.get(0)?,
                    count: r.get::<_, i64>(1)? as u64,
                    amount_cents: r.get(2)?,
                });
            }
        }

        // 全库固定项：沿用 voucher_stats 的既有语义（本月新增按当前自然月前缀）
        let total: u64 =
            conn.query_row("SELECT COUNT(*) FROM vouchers", [], |r| r.get::<_, i64>(0))? as u64;
        let attachments: u64 = conn.query_row("SELECT COUNT(*) FROM attachments", [], |r| {
            r.get::<_, i64>(0)
        })? as u64;
        let month_prefix = format!("{}%", &crate::utils::paths::today()[..7]);
        let this_month: u64 = conn.query_row(
            "SELECT COUNT(*) FROM vouchers WHERE voucher_date LIKE ?1",
            params![month_prefix],
            |r| r.get::<_, i64>(0),
        )? as u64;

        Ok(Overview {
            date_from,
            date_to,
            status: status.map(|s| s.as_str().to_string()),
            count,
            amount_cents,
            prev_amount_cents,
            prev_count,
            by_status,
            total,
            attachments,
            this_month,
        })
    })?;
    crate::utils::timing::log_elapsed("dashboard.overview", started);
    Ok(out)
}

/// 待办与健康度（全部为轻量计数；不含需要全表 stat 的项）
pub fn todos(ctx: &CoreContext) -> Result<Todos> {
    let (dup_groups, dup_pairs, dup_high_pairs) = {
        let c = dup_review::pending_count(ctx)?;
        (c.groups, c.pairs, c.high_pairs)
    };

    let (ocr_pending, amount_missing, stale_drafts, unlocked_months) =
        ctx.db.with_conn(|conn| {
            let ocr_pending: u64 = conn.query_row(
                "SELECT COUNT(*) FROM attachments WHERE ocr_status IN ('pending','failed')",
                [],
                |r| r.get::<_, i64>(0),
            )? as u64;

            let amount_missing: u64 = conn.query_row(
                "SELECT COUNT(*) FROM vouchers WHERE amount_cents IS NULL",
                [],
                |r| r.get::<_, i64>(0),
            )? as u64;

            // 陈旧草稿：阈值按"当前时刻 - N 天"与 updated_at（ISO 文本，可比）
            let threshold = (chrono::Local::now()
                - chrono::Duration::days(STALE_DRAFT_DAYS as i64))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
            let stale_drafts: u64 = conn.query_row(
                "SELECT COUNT(*) FROM vouchers WHERE status = 'draft' AND updated_at < ?1",
                params![threshold],
                |r| r.get::<_, i64>(0),
            )? as u64;

            // 未封账的往月：有凭证、月份早于当前月、且没有 period_locks 记录
            let current_month = crate::utils::paths::today()[..7].to_string();
            let unlocked_months: u64 = conn.query_row(
                "SELECT COUNT(*) FROM (
                 SELECT DISTINCT substr(voucher_date,1,7) AS m FROM vouchers
                 WHERE substr(voucher_date,1,7) < ?1
             ) t LEFT JOIN period_locks p ON p.period = t.m WHERE p.period IS NULL",
                params![current_month],
                |r| r.get::<_, i64>(0),
            )? as u64;

            Ok((ocr_pending, amount_missing, stale_drafts, unlocked_months))
        })?;

    // 备份新鲜度：只扫目录取最新文件（不读 manifest——加密备份读 manifest 要解密，太重）
    let latest_backup = latest_backup_lite(&ctx.paths.backups);
    let backup_stale = latest_backup
        .as_ref()
        .and_then(|b| chrono::DateTime::parse_from_rfc3339(&b.modified_at).ok())
        .map(|t| {
            chrono::Local::now().signed_duration_since(t.with_timezone(&chrono::Local))
                > chrono::Duration::hours(BACKUP_STALE_HOURS)
        })
        .unwrap_or(true); // 没有任何备份 → 视为陈旧（界面提示"建议先做一次备份"）

    Ok(Todos {
        ocr_pending,
        amount_missing,
        stale_drafts,
        stale_draft_days: STALE_DRAFT_DAYS,
        unlocked_months,
        latest_backup,
        backup_stale,
        dup_pairs,
        dup_high_pairs,
        dup_groups,
    })
}

/// 扫描备份目录取最新一份（按修改时间）；不读 manifest，故对加密备份同样廉价
fn latest_backup_lite(dir: &std::path::Path) -> Option<BackupLite> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut newest: Option<(std::time::SystemTime, String)> = None;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.ends_with(".zip") {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if newest.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
            newest = Some((modified, name));
        }
    }
    let (t, file_name) = newest?;
    let dt: chrono::DateTime<chrono::Local> = t.into();
    Some(BackupLite {
        file_name,
        modified_at: dt.to_rfc3339(),
    })
}

/// 月度趋势（供工作台主图表；直接复用既有 `reports::monthly_summary`，此处只做转发，
/// 让命令层经单一入口取数，便于将来统一窗口口径）
pub fn monthly(ctx: &CoreContext, months: u32) -> Result<Vec<reports::MonthlySummary>> {
    reports::monthly_summary(ctx, months)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prev_window_is_same_length_and_adjacent() {
        // 1 月 10 日 ~ 1 月 19 日（10 天）→ 上一窗口应为 2025-12-31 ~ 2026-01-09
        let (f, t) = prev_window("2026-01-10", "2026-01-19").unwrap();
        assert_eq!((f.as_str(), t.as_str()), ("2025-12-31", "2026-01-09"));
    }

    #[test]
    fn prev_window_single_day() {
        let (f, t) = prev_window("2026-03-01", "2026-03-01").unwrap();
        assert_eq!((f.as_str(), t.as_str()), ("2026-02-28", "2026-02-28"));
    }

    #[test]
    fn prev_window_rejects_reversed_range() {
        assert!(prev_window("2026-03-10", "2026-03-01").is_none());
        assert!(prev_window("不是日期", "2026-03-01").is_none());
    }

    #[test]
    fn window_where_builds_expected_clauses() {
        let (sql, bind) = window_where(Some("2026-01-01"), Some("2026-01-31"), None);
        assert_eq!(
            sql,
            "WHERE voucher_date >= ?1 AND voucher_date <= ?2".to_string()
        );
        assert_eq!(bind, vec!["2026-01-01", "2026-01-31"]);

        let (sql, bind) = window_where(None, None, Some(VoucherStatus::Draft));
        assert_eq!(sql, "WHERE status = ?1");
        assert_eq!(bind, vec!["draft"]);

        let (sql, _) = window_where(None, None, None);
        assert!(sql.is_empty(), "无口径时不应产生 WHERE");
    }
}

// ---------- v0.18 批次 5：洞察与台账所需的只读聚合 ----------

/// 透视行：某月 × 某个键（对方/标签）的张数与金额
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownRow {
    /// `YYYY-MM`
    pub month: String,
    pub key: String,
    pub count: u64,
    pub amount_cents: i64,
}

/// 月度透视：**月份 × 对方账户**（`dim="counterparty"`）或 **月份 × 标签**（`dim="tag"`）。
///
/// 口径：
/// - 只统计窗口内的凭证；金额为空的不计入金额合计（但计入张数）；
/// - 每月只返回前 `per_month` 个键（按金额倒序）——避免月份多、对方多时把界面撑爆；
/// - 标签用 `json_each` 展开：**一张凭证的多个标签会分别计数**（与 `tag_stats` 同口径）。
pub fn monthly_breakdown(
    ctx: &CoreContext,
    date_from: Option<String>,
    date_to: Option<String>,
    dim: &str,
    per_month: u32,
) -> Result<Vec<BreakdownRow>> {
    let per_month = per_month.clamp(1, 50);
    let started = std::time::Instant::now();
    let out = ctx.db.with_conn(|conn| {
        let (where_sql, bind) = window_where(date_from.as_deref(), date_to.as_deref(), None);
        let sql = match dim {
            "tag" => format!(
                // `json_valid` 兜底与 reports.rs 的 tag_stats 同口径（2026-09-27 审计 A-9）：
                // 同一个坏 tags 值，报告页能降级到"未分类"，这里却会抛 db 错误——
                // 一处静默一处硬报错，比两边都坏更难查
                "WITH per AS (
                     SELECT substr(v.voucher_date,1,7) AS month, je.value AS key,
                            COUNT(*) AS count, COALESCE(SUM(v.amount_cents),0) AS amount
                     FROM vouchers v,
                          json_each(CASE WHEN json_valid(v.tags) THEN v.tags ELSE '[]' END) je
                     {where_sql}
                     GROUP BY month, key
                 )
                 SELECT month, key, count, amount FROM (
                     SELECT *, ROW_NUMBER() OVER (PARTITION BY month ORDER BY amount DESC, count DESC) AS rn
                     FROM per
                 ) WHERE rn <= {per_month}
                 ORDER BY month DESC, amount DESC"
            ),
            // 默认：对方账户（空对方归为"未填"，便于发现数据缺口）
            _ => format!(
                "WITH per AS (
                     SELECT substr(voucher_date,1,7) AS month,
                            CASE WHEN TRIM(COALESCE(counterparty,'')) = '' THEN '(未填)' ELSE TRIM(counterparty) END AS key,
                            COUNT(*) AS count, COALESCE(SUM(amount_cents),0) AS amount
                     FROM vouchers {where_sql}
                     GROUP BY month, key
                 )
                 SELECT month, key, count, amount FROM (
                     SELECT *, ROW_NUMBER() OVER (PARTITION BY month ORDER BY amount DESC, count DESC) AS rn
                     FROM per
                 ) WHERE rn <= {per_month}
                 ORDER BY month DESC, amount DESC"
            ),
        };
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(bind.iter()))?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(BreakdownRow {
                month: r.get(0)?,
                key: r.get(1)?,
                count: r.get::<_, i64>(2)? as u64,
                amount_cents: r.get(3)?,
            });
        }
        Ok(out)
    })?;
    crate::utils::timing::log_elapsed("dashboard.monthly_breakdown", started);
    Ok(out)
}

/// 导出格式分布（一批一行，或一批多格式时按批次计）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatCount {
    pub format: String,
    pub batches: u64,
    pub vouchers: u64,
}

/// 导出覆盖度：窗口内"已导出 / 从未导出"台账。
///
/// 口径（**界面须标注**）：
/// - `never_exported` 指 `export_items` 里**没有任何该凭证的记录**（含已归档凭证；
///   历史遗留的归档件若从未经过导出批次，会计入此项——这正是要暴露的数据缺口）；
/// - `by_format` 按批次统计（`export_batches.format`，v0.12 起落库）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportCoverage {
    pub total: u64,
    pub exported: u64,
    pub never_exported: u64,
    pub by_format: Vec<FormatCount>,
}

pub fn export_coverage(
    ctx: &CoreContext,
    date_from: Option<String>,
    date_to: Option<String>,
) -> Result<ExportCoverage> {
    let started = std::time::Instant::now();
    let out = ctx.db.with_conn(|conn| {
        let (where_sql, bind) = window_where(date_from.as_deref(), date_to.as_deref(), None);
        let total: u64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM vouchers {where_sql}"),
            rusqlite::params_from_iter(bind.iter()),
            |r| r.get::<_, i64>(0),
        )? as u64;
        let never: u64 = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM vouchers {where_sql} {} NOT EXISTS
                   (SELECT 1 FROM export_items ei WHERE ei.voucher_id = vouchers.id)",
                if where_sql.is_empty() { "WHERE" } else { "AND" }
            ),
            rusqlite::params_from_iter(bind.iter()),
            |r| r.get::<_, i64>(0),
        )? as u64;

        let mut by_format = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT COALESCE(format,'pdf') AS f, COUNT(*) AS batches,
                    COALESCE(SUM(voucher_count),0) AS vouchers
             FROM export_batches WHERE status = 'done' GROUP BY f ORDER BY vouchers DESC",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            by_format.push(FormatCount {
                format: r.get(0)?,
                batches: r.get::<_, i64>(1)? as u64,
                vouchers: r.get::<_, i64>(2)? as u64,
            });
        }

        Ok(ExportCoverage {
            total,
            exported: total.saturating_sub(never),
            never_exported: never,
            by_format,
        })
    })?;
    crate::utils::timing::log_elapsed("dashboard.export_coverage", started);
    Ok(out)
}

/// 存储占用：**原图体积走 SQL**（`attachments.file_size` 已落库）；
/// 增强版体积 DB 里没有（可重建缓存，刻意不落列）→ 默认只报条数与来源分布，
/// 需要真实体积时传 `with_enhanced_bytes=true` 才逐文件 `stat`（慢，且仅界面主动点开时用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    pub original_count: u64,
    pub original_bytes: u64,
    /// 已增强的附件数（`enhanced_path` 非空）
    pub enhanced_count: u64,
    /// 增强版来源分布：截图 / 扫描件 / 未识别
    pub enhanced_screenshot: u64,
    pub enhanced_scan: u64,
    pub enhanced_unknown: u64,
    /// 增强版实际体积（仅在 `with_enhanced_bytes` 时计算，否则 None）
    pub enhanced_bytes: Option<u64>,
    /// OCR 缓存条数（缓存是纯加速用途，条数即可说明规模）
    pub ocr_cache_count: u64,
}

pub fn storage_usage(ctx: &CoreContext, with_enhanced_bytes: bool) -> Result<StorageUsage> {
    let started = std::time::Instant::now();
    let (original_count, original_bytes, enhanced_count, screenshot, scan, unknown, cache_rows) =
        ctx.db.with_conn(|conn| {
            let (oc, ob): (i64, Option<i64>) = conn.query_row(
                "SELECT COUNT(*), SUM(file_size) FROM attachments",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            // COALESCE 必需（审计批次 7 由压测发现）：attachments 为空表时 SUM(...) 返回 NULL，
            // 而下面按 i64 读 → `InvalidColumnType(Null)`，即"新装用户打开洞察页就报 db 错误"
            let (ec, ss, sc, uk): (i64, i64, i64, i64) = conn.query_row(
                "SELECT COALESCE(SUM(CASE WHEN enhanced_path IS NOT NULL THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN enhanced_path IS NOT NULL AND enhanced_source='screenshot' THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN enhanced_path IS NOT NULL AND enhanced_source='scan' THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN enhanced_path IS NOT NULL AND enhanced_source='unknown' THEN 1 ELSE 0 END), 0)
                 FROM attachments",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            let cc: i64 = conn.query_row("SELECT COUNT(*) FROM ocr_cache", [], |r| r.get(0))?;
            Ok((oc as u64, ob.unwrap_or(0) as u64, ec as u64, ss as u64, sc as u64, uk as u64, cc as u64))
        })?;

    // 增强版体积：DB 无该列（见规范：增强版是"可重建缓存"，不落体积）
    let enhanced_bytes = if with_enhanced_bytes {
        let mut total = 0u64;
        let rels: Vec<String> = ctx.db.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT enhanced_path FROM attachments WHERE enhanced_path IS NOT NULL")?;
            let mut rows = stmt.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(r.get::<_, String>(0)?);
            }
            Ok(out)
        })?;
        for rel in rels {
            if let Ok(meta) = std::fs::metadata(ctx.paths.attachment_abs(&rel)) {
                total += meta.len();
            }
        }
        Some(total)
    } else {
        None
    };

    crate::utils::timing::log_elapsed("dashboard.storage_usage", started);
    Ok(StorageUsage {
        original_count,
        original_bytes,
        enhanced_count,
        enhanced_screenshot: screenshot,
        enhanced_scan: scan,
        enhanced_unknown: unknown,
        enhanced_bytes,
        ocr_cache_count: cache_rows,
    })
}
