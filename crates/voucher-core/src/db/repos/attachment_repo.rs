//! 附件仓储

use rusqlite::{params, Connection};

use crate::models::{Attachment, EnhanceSource, NewAttachment, OcrStatus};
use crate::utils::paths::now_iso;
use crate::{CoreError, Result};

const COLS: &str =
    "id, voucher_id, file_name, file_path, file_hash, file_size, mime_type, ocr_status, ocr_text, enhanced_path, enhanced_source, created_at";

fn row_to_attachment(row: &rusqlite::Row<'_>) -> rusqlite::Result<Attachment> {
    let status: String = row.get(7)?;
    let source: String = row.get(10)?;
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
        enhanced_path: row.get(9)?,
        enhanced_source: EnhanceSource::parse(&source).unwrap_or(EnhanceSource::Unknown),
        created_at: row.get(11)?,
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

/// v3：写入增强版 JPEG 相对路径
pub fn set_enhanced(conn: &Connection, id: i64, rel_path: &str) -> Result<()> {
    conn.execute(
        "UPDATE attachments SET enhanced_path = ?1 WHERE id = ?2",
        params![rel_path, id],
    )?;
    Ok(())
}

/// v3：写入附件来源分类
pub fn set_enhanced_source(conn: &Connection, id: i64, source: EnhanceSource) -> Result<()> {
    conn.execute(
        "UPDATE attachments SET enhanced_source = ?1 WHERE id = ?2",
        params![source.as_str(), id],
    )?;
    Ok(())
}

/// v3：清除增强版（用于「清除增强缓存」或重新增强）
/// 返回受影响的行数（0/1）
pub fn clear_enhanced(conn: &Connection, id: i64) -> Result<bool> {
    let n = conn.execute(
        "UPDATE attachments SET enhanced_path = NULL WHERE id = ?1",
        params![id],
    )?;
    Ok(n > 0)
}

/// v3：列出所有已增强的附件（用于「清除增强缓存」前的扫描）
pub fn list_enhanced(conn: &Connection) -> Result<Vec<Attachment>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM attachments WHERE enhanced_path IS NOT NULL"
    ))?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(row_to_attachment(r)?);
    }
    Ok(out)
}

/// D-1 列表缩略图行：(voucher_id, file_path, enhanced_path, enhanced_source)
pub type VoucherThumbRow = (i64, String, Option<String>, String);

/// D-1 列表缩略图：每张凭证只取第一张附件的 file_path / enhanced_path / enhanced_source
///
/// 一次性 IN-query 拿到当前页所有凭证的首附件，避免 N+1。
/// 没有附件的凭证不出现在结果里（前端按 missing 显示占位）。
pub fn first_thumb_for_vouchers(
    conn: &Connection,
    voucher_ids: &[i64],
) -> Result<Vec<VoucherThumbRow>> {
    if voucher_ids.is_empty() {
        return Ok(Vec::new());
    }
    // 取每张凭证的最小 id 附件（一图一凭证下即唯一附件）
    let placeholders = vec!["?"; voucher_ids.len()].join(",");
    let sql = format!(
        "SELECT a.voucher_id, a.file_path, a.enhanced_path, a.enhanced_source
         FROM attachments a
         INNER JOIN (
             SELECT voucher_id, MIN(id) AS min_id
             FROM attachments
             WHERE voucher_id IN ({placeholders})
             GROUP BY voucher_id
         ) m ON m.voucher_id = a.voucher_id AND m.min_id = a.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(voucher_ids.iter()))?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, String>(3)?,
        ));
    }
    Ok(out)
}
