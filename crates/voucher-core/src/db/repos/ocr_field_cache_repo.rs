//! 云端 OCR 结构化要素缓存仓储（`ocr_field_cache` 表，v0.23 新增）
//!
//! 为什么独立于 `ocr_cache`：后者按图片内容哈希存「文本」，这里存「结构化要素」。
//! 两者失效条件不同（换模型要各自失效），且要素只在云端返回；
//! 缓存命中可让「重新识别同一张图」不再付费调 API。

use rusqlite::{params, Connection};

use crate::utils::paths::now_iso;
use crate::Result;

/// 写入（UPSERT）：`fields_json` 为云端返回的 JSON 对象序列化串
pub fn upsert(
    conn: &Connection,
    cache_key: &str,
    engine: &str,
    model: &str,
    fields_json: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO ocr_field_cache (cache_key, engine, model, fields_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(cache_key) DO UPDATE SET engine=?2, model=?3, fields_json=?4, created_at=?5",
        params![cache_key, engine, model, fields_json, now_iso()],
    )?;
    Ok(())
}

/// 读取：返回 (engine, model, fields_json)
pub fn get(conn: &Connection, cache_key: &str) -> Result<Option<(String, String, String)>> {
    let mut stmt = conn
        .prepare("SELECT engine, model, fields_json FROM ocr_field_cache WHERE cache_key = ?1")?;
    let mut rows = stmt.query(params![cache_key])?;
    Ok(match rows.next()? {
        Some(r) => Some((r.get(0)?, r.get(1)?, r.get(2)?)),
        None => None,
    })
}
