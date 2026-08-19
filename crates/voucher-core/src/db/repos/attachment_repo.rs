//! 附件仓储

use rusqlite::{params, Connection};

use crate::models::{Attachment, NewAttachment, OcrStatus};
use crate::utils::paths::now_iso;
use crate::{CoreError, Result};

const COLS: &str =
    "id, voucher_id, file_name, file_path, file_hash, file_size, mime_type, ocr_status, ocr_text, created_at";

fn row_to_attachment(row: &rusqlite::Row<'_>) -> rusqlite::Result<Attachment> {
    let status: String = row.get(7)?;
    Ok(Attachment {
        id: row.get(0)?,
        voucher_id: row.get(1)?,
        file_name: row.get(2)?,
        file_path: row.get(3)?,
        file_hash: row.get(4)?,
        file_size: row.get::<_, i64>(5)? as u64,
        mime_type: row.get(6)?,
        ocr_status: OcrStatus::parse(&status).unwrap_or(OcrStatus::Pending),
        ocr_text: row.get(8)?,
        created_at: row.get(9)?,
    })
}

pub fn insert(conn: &Connection, a: &NewAttachment) -> Result<Attachment> {
    conn.execute(
        "INSERT INTO attachments (voucher_id, file_name, file_path, file_hash, file_size, mime_type, ocr_status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
        params![a.voucher_id, a.file_name, a.file_path, a.file_hash, a.file_size as i64, a.mime_type, now_iso()],
    )?;
    let id = conn.last_insert_rowid();
    get(conn, id)?.ok_or_else(|| CoreError::Invalid("附件插入后读取失败".into()))
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Attachment>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM attachments WHERE id = ?1"))?;
    let mut rows = stmt.query(params![id])?;
    Ok(match rows.next()? {
        Some(r) => Some(row_to_attachment(r)?),
        None => None,
    })
}

pub fn list_by_voucher(conn: &Connection, voucher_id: i64) -> Result<Vec<Attachment>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM attachments WHERE voucher_id = ?1 ORDER BY id"
    ))?;
    let mut rows = stmt.query(params![voucher_id])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(row_to_attachment(r)?);
    }
    Ok(out)
}

pub fn delete(conn: &Connection, id: i64) -> Result<bool> {
    let n = conn.execute("DELETE FROM attachments WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

/// 按内容哈希找重复（去重键）
pub fn find_by_hash(conn: &Connection, hash: &str) -> Result<Option<Attachment>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM attachments WHERE file_hash = ?1 LIMIT 1"
    ))?;
    let mut rows = stmt.query(params![hash])?;
    Ok(match rows.next()? {
        Some(r) => Some(row_to_attachment(r)?),
        None => None,
    })
}

pub fn set_ocr(conn: &Connection, id: i64, status: OcrStatus, text: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE attachments SET ocr_status = ?1, ocr_text = ?2 WHERE id = ?3",
        params![status.as_str(), text, id],
    )?;
    Ok(())
}

pub fn list_pending_ocr(conn: &Connection, limit: u32) -> Result<Vec<Attachment>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM attachments WHERE ocr_status IN ('pending','failed') ORDER BY id LIMIT {limit}"
    ))?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(row_to_attachment(r)?);
    }
    Ok(out)
}
