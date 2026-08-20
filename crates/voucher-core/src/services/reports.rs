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
