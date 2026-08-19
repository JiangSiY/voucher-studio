//! OCR 结果缓存仓储（以附件内容哈希为键）

use rusqlite::{params, Connection};

use crate::utils::paths::now_iso;
use crate::Result;

pub fn upsert(conn: &Connection, cache_key: &str, engine: &str, text: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO ocr_cache (cache_key, engine, text, created_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(cache_key) DO UPDATE SET engine=?2, text=?3, created_at=?4",
        params![cache_key, engine, text, now_iso()],
    )?;
    Ok(())
}

/// 返回 (engine, text)
pub fn get(conn: &Connection, cache_key: &str) -> Result<Option<(String, String)>> {
    let mut stmt = conn.prepare("SELECT engine, text FROM ocr_cache WHERE cache_key = ?1")?;
    let mut rows = stmt.query(params![cache_key])?;
    Ok(match rows.next()? {
        Some(r) => Some((r.get(0)?, r.get(1)?)),
        None => None,
    })
}
