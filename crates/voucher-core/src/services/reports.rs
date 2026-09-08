//! 月度报表统计（对应 v4.3 的 `pdf/reports.py`）
//!
//! 近 N 月逐月汇总：张数 / 金额合计 / 状态分布（draft/reviewed/exported/archived 透视）。
//! 空月补零，保证趋势图横轴连续；升序返回（旧 → 新）。

use std::collections::HashMap;

use chrono::Datelike;
use rusqlite::Connection;
use serde::Serialize;

use crate::{CoreContext, CoreError, Result};

/// 单月汇总（金额单位：分）
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthlySummary {
    /// 月份（YYYY-MM）
    pub month: String,
    /// 凭证张数
    pub count: i64,
    /// 金额合计（分；空金额不计入）
    pub amount_cents: i64,
    pub draft: i64,
    pub reviewed: i64,
    pub exported: i64,
    pub archived: i64,
}

/// 近 `months` 个月（含当月）逐月汇总；`months` 为 0 时返回空
pub fn monthly_summary(ctx: &CoreContext, months: u32) -> Result<Vec<MonthlySummary>> {
    let labels = month_window(months)?;
    if labels.is_empty() {
        return Ok(Vec::new());
    }

    let grouped: HashMap<String, [i64; 6]> = ctx.db.with_conn(query_grouped)?;

    Ok(labels
        .into_iter()
        .map(|m| {
            let d = grouped.get(&m).copied().unwrap_or([0; 6]);
            MonthlySummary {
                month: m,
                count: d[0],
                amount_cents: d[1],
                draft: d[2],
                reviewed: d[3],
                exported: d[4],
                archived: d[5],
            }
        })
        .collect())
}

/// 月份窗口（当月往前推 months-1 个月），升序
fn month_window(months: u32) -> Result<Vec<String>> {
    let first = chrono::Local::now()
        .date_naive()
        .with_day(1)
        .and_then(|d| d.checked_sub_months(chrono::Months::new(months.saturating_sub(1))))
        .ok_or_else(|| CoreError::Invalid("月份窗口计算失败".into()))?;
    (0..months)
        .map(|i| {
            first
                .checked_add_months(chrono::Months::new(i))
                .map(|d| d.format("%Y-%m").to_string())
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| CoreError::Invalid("月份窗口计算失败".into()))
}

/// 月份前缀 + 状态透视的分组查询，值为 [张数, 金额合计, draft, reviewed, exported, archived]
fn query_grouped(conn: &Connection) -> Result<HashMap<String, [i64; 6]>> {
    let mut stmt = conn.prepare(
        "SELECT substr(voucher_date, 1, 7) AS m,
                COUNT(*) AS cnt,
                COALESCE(SUM(amount_cents), 0) AS amt,
                COALESCE(SUM(CASE WHEN status = 'draft' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'reviewed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'exported' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'archived' THEN 1 ELSE 0 END), 0)
         FROM vouchers
         GROUP BY m",
    )?;
    let mut rows = stmt.query([])?;
    let mut map = HashMap::new();
    while let Some(r) = rows.next()? {
        let month: String = r.get(0)?;
        map.insert(
            month,
            [
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ],
        );
    }
    Ok(map)
}

// ---------- R-1：Top 对方账户排行（按金额降序） ----------

/// 单个对方账户的聚合行
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CounterpartyRow {
    /// 对方账户名
    pub name: String,
    /// 凭证张数
    pub count: i64,
    /// 金额合计（分；空金额不计入）
    pub amount_cents: i64,
}

/// Top 对方账户：按金额合计降序（同金额按张数降序），取前 `limit` 名。
/// 统计范围近 `months` 个月（含当月；0 = 全部历史）。
pub fn top_counterparties(
    ctx: &CoreContext,
    months: u32,
    limit: u32,
) -> Result<Vec<CounterpartyRow>> {
    let limit = limit.clamp(1, 50);
    ctx.db.with_conn(|conn| {
        let map_row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<CounterpartyRow> {
            Ok(CounterpartyRow {
                name: r.get(0)?,
                count: r.get(1)?,
                amount_cents: r.get(2)?,
            })
        };
        if months == 0 {
            let mut stmt = conn.prepare(
                "SELECT counterparty AS name,
                        COUNT(*) AS cnt,
                        COALESCE(SUM(amount_cents), 0) AS amt
                 FROM vouchers
                 WHERE TRIM(counterparty) <> ''
                 GROUP BY counterparty
                 ORDER BY amt DESC, cnt DESC
                 LIMIT ?1",
            )?;
            let rows = stmt
                .query_map([limit as i64], map_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        } else {
            // 月份窗口起点（含当月往前推 months-1 个月）
            let first = chrono::Local::now()
                .date_naive()
                .with_day(1)
                .and_then(|d| d.checked_sub_months(chrono::Months::new(months.saturating_sub(1))))
                .ok_or_else(|| CoreError::Invalid("月份窗口计算失败".into()))?;
            let prefix_from = first.format("%Y-%m").to_string();
            let mut stmt = conn.prepare(
                "SELECT counterparty AS name,
                        COUNT(*) AS cnt,
                        COALESCE(SUM(amount_cents), 0) AS amt
                 FROM vouchers
                 WHERE TRIM(counterparty) <> ''
                   AND substr(voucher_date, 1, 7) >= ?1
                 GROUP BY counterparty
                 ORDER BY amt DESC, cnt DESC
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![prefix_from, limit as i64], map_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    })
}

// ---------- 功能深化 3：标签分类统计（按金额降序） ----------

/// 单个标签的聚合行（tag 为空串表示「未分类」：没有任何标签的凭证）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagRow {
    /// 标签名（空串 = 未分类）
    pub tag: String,
    /// 凭证张数（一张多标签的凭证在每个标签下各计 1 次）
    pub count: i64,
    /// 金额合计（分；空金额不计入）
    pub amount_cents: i64,
}

/// 标签分类统计：按金额合计降序（同金额按张数降序），取前 `limit` 名。
/// 统计范围近 `months` 个月（含当月；0 = 全部历史）。
/// 「未分类」（无标签凭证）固定排在末尾（不占 limit 名额，便于核对总数）。
pub fn tag_stats(ctx: &CoreContext, months: u32, limit: u32) -> Result<Vec<TagRow>> {
    let limit = limit.clamp(1, 50);
    // 月份窗口起点（months=0 表示全部历史，不加日期过滤）
    let prefix_from = if months == 0 {
        None
    } else {
        let first = chrono::Local::now()
            .date_naive()
            .with_day(1)
            .and_then(|d| d.checked_sub_months(chrono::Months::new(months.saturating_sub(1))))
            .ok_or_else(|| CoreError::Invalid("月份窗口计算失败".into()))?;
        Some(first.format("%Y-%m").to_string())
    };

    ctx.db.with_conn(|conn| {
        let map_row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<TagRow> {
            Ok(TagRow {
                tag: r.get(0)?,
                count: r.get(1)?,
                amount_cents: r.get(2)?,
            })
        };
        // json_each 展开标签数组（json_valid 兜底防脏数据；TRIM 排空标签）
        let mut tagged_stmt = conn.prepare(&format!(
            "SELECT je.value AS tag,
                    COUNT(*) AS cnt,
                    COALESCE(SUM(v.amount_cents), 0) AS amt
             FROM vouchers v, json_each(CASE WHEN json_valid(v.tags) THEN v.tags ELSE '[]' END) je
             WHERE TRIM(je.value) <> ''
               {date_filter}
             GROUP BY je.value
             ORDER BY amt DESC, cnt DESC
             LIMIT ?",
            date_filter = if prefix_from.is_some() {
                "AND substr(v.voucher_date, 1, 7) >= ?"
            } else {
                ""
            }
        ))?;
        let tagged: Vec<TagRow> = if let Some(from) = &prefix_from {
            tagged_stmt
                .query_map(rusqlite::params![from, limit as i64], map_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            tagged_stmt
                .query_map(rusqlite::params![limit as i64], map_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        drop(tagged_stmt);

        // 未分类：没有任何有效（非空白）标签的凭证（不占 limit 名额）
        let mut untagged_stmt = conn.prepare(&format!(
            "SELECT COUNT(*), COALESCE(SUM(amount_cents), 0)
             FROM vouchers
             WHERE NOT EXISTS (
                 SELECT 1 FROM json_each(CASE WHEN json_valid(tags) THEN tags ELSE '[]' END) je
                 WHERE TRIM(je.value) <> ''
             )
               {date_filter}",
            date_filter = if prefix_from.is_some() {
                "AND substr(voucher_date, 1, 7) >= ?"
            } else {
                ""
            }
        ))?;
        let (u_cnt, u_amt): (i64, i64) = if let Some(from) = &prefix_from {
            untagged_stmt.query_row(rusqlite::params![from], |r| Ok((r.get(0)?, r.get(1)?)))?
        } else {
            untagged_stmt.query_row([], |r| Ok((r.get(0)?, r.get(1)?)))?
        };

        let mut rows = tagged;
        if u_cnt > 0 {
            rows.push(TagRow {
                tag: String::new(),
                count: u_cnt,
                amount_cents: u_amt,
            });
        }
        Ok(rows)
    })
}

// ---------- v0.10：常用摘要（编辑页联想） ----------

/// 常用摘要行
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DescriptionRow {
    pub description: String,
    /// 使用次数
    pub count: i64,
}

/// 高频摘要：按历史使用频次降序，取前 `limit` 条。
/// 空摘要（导入后未填写，描述=文件名）不参与——联想的目的是复用「人写过的」摘要。
pub fn frequent_descriptions(ctx: &CoreContext, limit: u32) -> Result<Vec<DescriptionRow>> {
    let limit = limit.clamp(1, 20);
    ctx.db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT description, COUNT(*) AS cnt
             FROM vouchers
             WHERE TRIM(description) <> ''
             GROUP BY description
             ORDER BY cnt DESC, description ASC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], |r| {
                Ok(DescriptionRow {
                    description: r.get(0)?,
                    count: r.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}
