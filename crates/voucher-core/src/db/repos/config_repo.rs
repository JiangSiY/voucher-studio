//! KV 状态仓储（运行时状态，如上次备份时间；与用户配置 app.json 分离）

use rusqlite::{params, Connection};

use crate::utils::paths::now_iso;
use crate::Result;

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM config WHERE key = ?1")?;
    let mut rows = stmt.query(params![key])?;
    Ok(match rows.next()? {
        Some(r) => Some(r.get(0)?),
        None => None,
    })
}

pub fn set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO config (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value=?2, updated_at=?3",
        params![key, value, now_iso()],
    )?;
    Ok(())
}
