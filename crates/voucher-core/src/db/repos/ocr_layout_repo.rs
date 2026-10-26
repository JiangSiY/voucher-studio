//! OCR 版面数据存取（ocr_layouts 表：attachment_id → lines JSON）
//!
//! v5 起本地引擎识别时保留每行文本 + 置信度 + 坐标框，
//! 供 ocr_layout 版面分析（几何键值配对）使用。
//! 旧附件 / 云端引擎无 layout → None，抽取回退纯文本模式。

use rusqlite::Connection;

use crate::models::ocr_line::OcrLine;
use crate::Result;

/// 写入（UPSERT）：识别完成后调用。`engine` 记录版面由哪个引擎产出
/// （v0.23 起云端/本地共存，几何配对必须只认同源版面，见 `get_layout`）
pub fn set_layout(
    conn: &Connection,
    attachment_id: i64,
    engine: &str,
    lines: &[OcrLine],
) -> Result<()> {
    let json = serde_json::to_string(lines)?;
    conn.execute(
        "INSERT INTO ocr_layouts (attachment_id, engine, lines, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(attachment_id) DO UPDATE SET engine = ?2, lines = ?3, updated_at = ?4",
        rusqlite::params![
            attachment_id,
            engine,
            json,
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
        ],
    )?;
    Ok(())
}

/// 删除版面（云端结构化要素成功后调用：避免旧本地版面在下次填单时压过云端要素）
pub fn delete_layout(conn: &Connection, attachment_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM ocr_layouts WHERE attachment_id = ?1",
        rusqlite::params![attachment_id],
    )?;
    Ok(())
}

/// 读取版面：返回 `(engine, lines)`；无记录返回 None（旧数据 / 云端引擎 / 识别失败）
pub fn get_layout(conn: &Connection, attachment_id: i64) -> Result<Option<(String, Vec<OcrLine>)>> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT engine, lines FROM ocr_layouts WHERE attachment_id = ?1",
            rusqlite::params![attachment_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    match row {
        Some((engine, json)) => Ok(Some((engine, serde_json::from_str(&json)?))),
        None => Ok(None),
    }
}
