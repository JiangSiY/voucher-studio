//! OCR 版面数据存取（ocr_layouts 表：attachment_id → lines JSON）
//!
//! v5 起本地引擎识别时保留每行文本 + 置信度 + 坐标框，
//! 供 ocr_layout 版面分析（几何键值配对）使用。
//! 旧附件 / 云端引擎无 layout → None，抽取回退纯文本模式。

use rusqlite::Connection;

use crate::models::ocr_line::OcrLine;
use crate::Result;

/// 写入（UPSERT）：识别完成后调用
pub fn set_layout(conn: &Connection, attachment_id: i64, lines: &[OcrLine]) -> Result<()> {
    let json = serde_json::to_string(lines)?;
    conn.execute(
        "INSERT INTO ocr_layouts (attachment_id, lines, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(attachment_id) DO UPDATE SET lines = ?2, updated_at = ?3",
        rusqlite::params![
            attachment_id,
            json,
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
        ],
    )?;
    Ok(())
}

/// 读取版面（无记录返回 None：旧数据 / 云端引擎 / 识别失败）
pub fn get_layout(conn: &Connection, attachment_id: i64) -> Result<Option<Vec<OcrLine>>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT lines FROM ocr_layouts WHERE attachment_id = ?1",
            rusqlite::params![attachment_id],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    match json {
        Some(s) => Ok(Some(serde_json::from_str(&s)?)),
        None => Ok(None),
    }
}
