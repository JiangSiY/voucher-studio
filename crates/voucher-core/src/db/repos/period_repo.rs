//! 会计期间锁仓储（period_locks；v0.11 按月封账）
//!
//! period = "YYYY-MM"（取凭证日期前 7 位）。锁定的月份内凭证禁止增删改，
//! 防止封账后返工导致账实不符。快照/备份天然携带锁状态，恢复后随库走。

use rusqlite::{params, Connection};

use serde::Serialize;

use crate::utils::paths::now_iso;
use crate::Result;

/// 已锁期间行
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodLock {
    /// YYYY-MM
    pub period: String,
    pub locked_at: String,
}

/// 全部已锁期间（按期间倒序）
pub fn list(conn: &Connection) -> Result<Vec<PeriodLock>> {
    let mut stmt =
        conn.prepare("SELECT period, locked_at FROM period_locks ORDER BY period DESC")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(PeriodLock {
            period: r.get(0)?,
            locked_at: r.get(1)?,
        });
    }
    Ok(out)
}

/// 期间是否已锁
pub fn is_locked(conn: &Connection, period: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM period_locks WHERE period = ?1",
        params![period],
        |r| r.get::<_, i64>(0),
    )? > 0)
}

/// 锁定期间（幂等：重复锁刷新锁定时间）
pub fn lock(conn: &Connection, period: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO period_locks (period, locked_at) VALUES (?1, ?2)
         ON CONFLICT(period) DO UPDATE SET locked_at = excluded.locked_at",
        params![period, now_iso()],
    )?;
    Ok(())
}

/// 解锁期间（不存在时幂等无操作）
pub fn unlock(conn: &Connection, period: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM period_locks WHERE period = ?1",
        params![period],
    )?;
    Ok(())
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
    fn lock_unlock_roundtrip() {
        let conn = conn();
        assert!(!is_locked(&conn, "2026-09").unwrap());
        lock(&conn, "2026-09").unwrap();
        assert!(is_locked(&conn, "2026-09").unwrap());
        // 幂等重复锁
        lock(&conn, "2026-09").unwrap();
        assert_eq!(list(&conn).unwrap().len(), 1);
        unlock(&conn, "2026-09").unwrap();
        assert!(!is_locked(&conn, "2026-09").unwrap());
        // 幂等解锁不存在期间
        unlock(&conn, "2099-01").unwrap();
        assert!(list(&conn).unwrap().is_empty());
    }
}
