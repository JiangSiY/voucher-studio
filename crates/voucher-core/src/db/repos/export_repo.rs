//! 导出批次仓储（export_batches / export_items）

use rusqlite::{params, Connection};

use crate::models::{ExportBatch, ExportItem, ExportStatus, NewExportBatch};
use crate::utils::paths::now_iso;
use crate::{CoreError, Result};

pub fn insert_batch(conn: &Connection, b: &NewExportBatch) -> Result<ExportBatch> {
    conn.execute(
        "INSERT INTO export_batches (batch_no, voucher_count, pdf_path, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            b.batch_no,
            b.voucher_count,
            b.pdf_path,
            b.status.as_str(),
            now_iso()
        ],
    )?;
    let id = conn.last_insert_rowid();
    get(conn, id)?.ok_or_else(|| CoreError::Invalid("批次插入后读取失败".into()))
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<ExportBatch>> {
    let mut stmt = conn.prepare(
        "SELECT id, batch_no, voucher_count, pdf_path, status, created_at FROM export_batches WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    Ok(match rows.next()? {
        Some(r) => Some(ExportBatch {
            id: r.get(0)?,
            batch_no: r.get(1)?,
            voucher_count: r.get(2)?,
            pdf_path: r.get(3)?,
            status: ExportStatus::parse(&r.get::<_, String>(4)?).unwrap_or(ExportStatus::Done),
            created_at: r.get(5)?,
        }),
        None => None,
    })
}

pub fn batch_no_exists(conn: &Connection, batch_no: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM export_batches WHERE batch_no = ?1",
        params![batch_no],
        |r| r.get::<_, i64>(0),
    )? > 0)
}

pub fn insert_item(
    conn: &Connection,
    batch_id: i64,
    voucher_id: i64,
    sort_order: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO export_items (batch_id, voucher_id, sort_order) VALUES (?1, ?2, ?3)",
        params![batch_id, voucher_id, sort_order],
    )?;
    Ok(())
}

pub fn list_items(conn: &Connection, batch_id: i64) -> Result<Vec<ExportItem>> {
    let mut stmt = conn.prepare(
        "SELECT id, batch_id, voucher_id, sort_order FROM export_items WHERE batch_id = ?1 ORDER BY sort_order",
    )?;
    let mut rows = stmt.query(params![batch_id])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(ExportItem {
            id: r.get(0)?,
            batch_id: r.get(1)?,
            voucher_id: r.get(2)?,
            sort_order: r.get(3)?,
        });
    }
    Ok(out)
}

pub fn list_batches(conn: &Connection, limit: u32) -> Result<Vec<ExportBatch>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, batch_no, voucher_count, pdf_path, status, created_at FROM export_batches ORDER BY id DESC LIMIT {limit}"
    ))?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(ExportBatch {
            id: r.get(0)?,
            batch_no: r.get(1)?,
            voucher_count: r.get(2)?,
            pdf_path: r.get(3)?,
            status: ExportStatus::parse(&r.get::<_, String>(4)?).unwrap_or(ExportStatus::Done),
            created_at: r.get(5)?,
        });
    }
    Ok(out)
}
