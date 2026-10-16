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
    Ok(fill_months(&labels, &grouped))
}

/// 按标签月份展开 + 空月补零（体检 Q-12：monthly / yearly 共用同一映射）
fn fill_months(labels: &[String], grouped: &HashMap<String, [i64; 6]>) -> Vec<MonthlySummary> {
    labels
        .iter()
        .map(|m| {
            let d = grouped.get(m).copied().unwrap_or([0; 6]);
            MonthlySummary {
                month: m.clone(),
                count: d[0],
                amount_cents: d[1],
                draft: d[2],
                reviewed: d[3],
                exported: d[4],
                archived: d[5],
            }
        })
        .collect()
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

/// 排行类查询的统计范围：**日期闭区间优先**，否则近 `months` 个月（0 = 全部历史）。
///
/// 为什么要有它（2026-10-16 用户反馈「洞察排行面板不跟随时间窗」）：洞察页的窗口是
/// **任意区间**（预设 + 自定义可到日），而排行原先只支持「近 N 个月」——前端只好传 0，
/// 于是无论窗口怎么切，排行永远是全历史前 10。这里把两种口径收在一处，
/// 前端把窗口的 `dateFrom`/`dateTo` 传下来即可；两者同时给以**区间为准**（区间更精确）。
struct RangeFilter {
    /// `substr(voucher_date,1,7) >= ?` 用的月份前缀（YYYY-MM）
    prefix_from: Option<String>,
    date_from: Option<String>,
    date_to: Option<String>,
}

impl RangeFilter {
    fn new(months: u32, date_from: Option<&str>, date_to: Option<&str>) -> Result<Self> {
        let has_range = date_from.is_some() || date_to.is_some();
        let prefix_from = if months == 0 || has_range {
            None
        } else {
            let first = chrono::Local::now()
                .date_naive()
                .with_day(1)
                .and_then(|d| d.checked_sub_months(chrono::Months::new(months.saturating_sub(1))))
                .ok_or_else(|| CoreError::Invalid("月份窗口计算失败".into()))?;
            Some(first.format("%Y-%m").to_string())
        };
        Ok(Self {
            prefix_from,
            date_from: date_from.map(str::to_string),
            date_to: date_to.map(str::to_string),
        })
    }

    /// 把过滤条件与绑定值追加到 SQL 上（`date_col` 允许写成 `voucher_date` / `v.voucher_date`）
    fn append(&self, sql: &mut String, binds: &mut Vec<Box<dyn rusqlite::ToSql>>, date_col: &str) {
        if let Some(from) = &self.date_from {
            sql.push_str(&format!(" AND {date_col} >= ?"));
            binds.push(Box::new(from.clone()));
        }
        if let Some(to) = &self.date_to {
            sql.push_str(&format!(" AND {date_col} <= ?"));
            binds.push(Box::new(to.clone()));
        }
        if let Some(prefix) = &self.prefix_from {
            sql.push_str(&format!(" AND substr({date_col}, 1, 7) >= ?"));
            binds.push(Box::new(prefix.clone()));
        }
    }
}

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
/// 统计范围：给了 `date_from`/`date_to` 按**日期闭区间**（`YYYY-MM-DD`，含端点）；
/// 否则近 `months` 个月（含当月；0 = 全部历史）。
pub fn top_counterparties(
    ctx: &CoreContext,
    months: u32,
    limit: u32,
    date_from: Option<&str>,
    date_to: Option<&str>,
) -> Result<Vec<CounterpartyRow>> {
    let limit = limit.clamp(1, 50);
    let range = RangeFilter::new(months, date_from, date_to)?;
    ctx.db.with_conn(|conn| {
        let map_row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<CounterpartyRow> {
            Ok(CounterpartyRow {
                name: r.get(0)?,
                count: r.get(1)?,
                amount_cents: r.get(2)?,
            })
        };
        let mut sql = String::from(
            "SELECT counterparty AS name,
                    COUNT(*) AS cnt,
                    COALESCE(SUM(amount_cents), 0) AS amt
             FROM vouchers
             WHERE TRIM(counterparty) <> ''",
        );
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        range.append(&mut sql, &mut binds, "voucher_date");
        sql.push_str(" GROUP BY counterparty ORDER BY amt DESC, cnt DESC LIMIT ?");
        binds.push(Box::new(limit as i64));
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())),
                map_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
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
/// 统计范围：同 [`top_counterparties`]（日期区间优先，否则近 `months` 个月）。
/// 「未分类」（无标签凭证）固定排在末尾（不占 limit 名额，便于核对总数）。
pub fn tag_stats(
    ctx: &CoreContext,
    months: u32,
    limit: u32,
    date_from: Option<&str>,
    date_to: Option<&str>,
) -> Result<Vec<TagRow>> {
    let limit = limit.clamp(1, 50);
    let range = RangeFilter::new(months, date_from, date_to)?;

    ctx.db.with_conn(|conn| {
        let map_row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<TagRow> {
            Ok(TagRow {
                tag: r.get(0)?,
                count: r.get(1)?,
                amount_cents: r.get(2)?,
            })
        };
        // json_each 展开标签数组（json_valid 兜底防脏数据；TRIM 排空标签）
        let mut tagged_sql = String::from(
            "SELECT je.value AS tag,
                    COUNT(*) AS cnt,
                    COALESCE(SUM(v.amount_cents), 0) AS amt
             FROM vouchers v, json_each(CASE WHEN json_valid(v.tags) THEN v.tags ELSE '[]' END) je
             WHERE TRIM(je.value) <> ''",
        );
        let mut tagged_binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        range.append(&mut tagged_sql, &mut tagged_binds, "v.voucher_date");
        tagged_sql.push_str(" GROUP BY je.value ORDER BY amt DESC, cnt DESC LIMIT ?");
        tagged_binds.push(Box::new(limit as i64));
        let mut tagged_stmt = conn.prepare(&tagged_sql)?;
        let mut rows: Vec<TagRow> = tagged_stmt
            .query_map(
                rusqlite::params_from_iter(tagged_binds.iter().map(|b| b.as_ref())),
                map_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(tagged_stmt);

        // 未分类：没有任何有效（非空白）标签的凭证（不占 limit 名额）
        let mut untagged_sql = String::from(
            "SELECT COUNT(*), COALESCE(SUM(amount_cents), 0)
             FROM vouchers
             WHERE NOT EXISTS (
                 SELECT 1 FROM json_each(CASE WHEN json_valid(tags) THEN tags ELSE '[]' END) je
                 WHERE TRIM(je.value) <> ''
             )",
        );
        let mut untagged_binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        range.append(&mut untagged_sql, &mut untagged_binds, "voucher_date");
        let mut untagged_stmt = conn.prepare(&untagged_sql)?;
        let (u_cnt, u_amt): (i64, i64) = untagged_stmt.query_row(
            rusqlite::params_from_iter(untagged_binds.iter().map(|b| b.as_ref())),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;

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

// ---------- v0.11：年度汇总（12 个月空月补零，供年度报表/图表） ----------

/// 某一自然年的逐月汇总（1–12 月，空月补零；口径与 monthly_summary 一致）
pub fn yearly_summary(ctx: &CoreContext, year: i32) -> Result<Vec<MonthlySummary>> {
    let labels: Vec<String> = (1..=12).map(|m| format!("{year:04}-{m:02}")).collect();
    // 全量分组后按标签取值：单用户数据量下全量 GROUP BY 成本可忽略，
    // 与 monthly_summary 共用同一透视口径（体检 Q-12：内联副本收口）
    let grouped: HashMap<String, [i64; 6]> = ctx.db.with_conn(query_grouped)?;
    Ok(fill_months(&labels, &grouped))
}
