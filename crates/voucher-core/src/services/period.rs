//! 会计期间锁服务（v0.11 按月封账）
//!
//! 锁定语义：已锁月份内的凭证禁止人工增删改（编辑 / 删除 / 批量操作 / 换图）。
//! OCR 自动填单不拦截（只填 draft 阶段空字段，属录入辅助）。
//! 日期从 A 月改到 B 月：两侧月份都需未锁（改出 / 改入都算变动）。

use rusqlite::Connection;

use crate::db::repos::period_repo::{self, PeriodLock};
use crate::{CoreContext, CoreError, Result};

/// 凭证日期 → 期间（YYYY-MM）；非法日期返回 None
pub fn period_of(date: &str) -> Option<String> {
    let d = date.trim();
    if d.len() >= 7 && d.as_bytes().get(4) == Some(&b'-') {
        Some(d[..7].to_string())
    } else {
        None
    }
}

/// 锁定期间（period 须为 YYYY-MM）
pub fn lock_period(ctx: &CoreContext, period: &str) -> Result<()> {
    validate_period(period)?;
    ctx.db.with_conn(|conn| period_repo::lock(conn, period))
}

/// 解锁期间
pub fn unlock_period(ctx: &CoreContext, period: &str) -> Result<()> {
    validate_period(period)?;
    ctx.db.with_conn(|conn| period_repo::unlock(conn, period))
}

/// 全部已锁期间
pub fn list_periods(ctx: &CoreContext) -> Result<Vec<PeriodLock>> {
    ctx.db.with_conn(period_repo::list)
}

/// 守卫：给定日期中任一所在期间已锁 → 报错（中文提示含具体月份）。
/// 供 voucher_mgr / ingestion 的写操作在事务内调用。
pub fn ensure_dates_open(conn: &Connection, dates: &[&str]) -> Result<()> {
    let mut locked: Vec<String> = Vec::new();
    for d in dates {
        let Some(p) = period_of(d) else {
            continue;
        };
        if !locked.contains(&p) && period_repo::is_locked(conn, &p)? {
            locked.push(p);
        }
    }
    if locked.is_empty() {
        return Ok(());
    }
    Err(CoreError::Invalid(format!(
        "{} 已封账，凭证不可变动。如确需修改，请先到「设置 → 凭证参数 → 期间管理」解锁",
        locked.join("、")
    )))
}

fn validate_period(period: &str) -> Result<()> {
    let p = period.trim();
    let valid = p.len() == 7
        && p.as_bytes().get(4) == Some(&b'-')
        && p[..4].bytes().all(|b| b.is_ascii_digit())
        && p[5..].bytes().all(|b| b.is_ascii_digit());
    if valid {
        Ok(())
    } else {
        Err(CoreError::Invalid(format!(
            "期间格式应为 YYYY-MM，收到 {period}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        schema::init(&conn).unwrap();
        conn
    }

    #[test]
    fn period_of_extracts_month() {
        assert_eq!(period_of("2026-09-08"), Some("2026-09".into()));
        assert_eq!(period_of("2026-09"), Some("2026-09".into()));
        assert_eq!(period_of("bad"), None);
        assert_eq!(period_of(""), None);
    }

    #[test]
    fn guard_blocks_locked_dates_only() {
        let conn = conn();
        period_repo::lock(&conn, "2026-08").unwrap();
        // 未锁日期放行
        assert!(ensure_dates_open(&conn, &["2026-09-01", "2026-09-30"]).is_ok());
        // 锁定日期拦截（报错信息含月份）
        let err = ensure_dates_open(&conn, &["2026-09-01", "2026-08-15"]).unwrap_err();
        assert!(err.to_string().contains("2026-08 已封账"));
        // 非法日期跳过（不报错）
        assert!(ensure_dates_open(&conn, &["bad", "2026-09-01"]).is_ok());
    }

    #[test]
    fn period_format_validated() {
        assert!(validate_period("2026-09").is_ok());
        assert!(validate_period("202609").is_err());
        assert!(validate_period("2026-9").is_err());
        assert!(validate_period("abcd-ef").is_err());
    }
}
