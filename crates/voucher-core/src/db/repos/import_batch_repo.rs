//! 台账导入批次仓储（import_batches，v13）
//!
//! 只做"批次行"的读写；撤销时要连带动到凭证，那跨两个聚合，放在服务层
//! （`services::ledger::undo_import_batch`），仓储层不自己开事务。

use rusqlite::{params, Connection};

use crate::models::{ImportBatch, NewImportBatch};
use crate::utils::paths::now_iso;
use crate::{CoreError, Result};

const COLS: &str = "id, created_at, kind, file_name, file_path, rows_total, created, matched, skipped, error_rows, undone_at";

fn row_to_batch(r: &rusqlite::Row<'_>) -> rusqlite::Result<ImportBatch> {
    Ok(ImportBatch {
        id: r.get(0)?,
        created_at: r.get(1)?,
        kind: r.get(2)?,
        file_name: r.get(3)?,
        file_path: r.get(4)?,
        rows_total: r.get(5)?,
        created: r.get(6)?,
        matched: r.get(7)?,
        skipped: r.get(8)?,
        error_rows: r.get(9)?,
        undone_at: r.get(10)?,
    })
}

pub fn insert(conn: &Connection, b: &NewImportBatch) -> Result<ImportBatch> {
    conn.execute(
        "INSERT INTO import_batches
           (created_at, kind, file_name, file_path, rows_total, created, matched, skipped, error_rows)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            now_iso(),
            b.kind,
            b.file_name,
            b.file_path,
            b.rows_total,
            b.created,
            b.matched,
            b.skipped,
            b.error_rows,
        ],
    )?;
    let id = conn.last_insert_rowid();
    get(conn, id)?.ok_or_else(|| CoreError::Invalid("批次插入后读取失败".into()))
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<ImportBatch>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM import_batches WHERE id = ?1"))?;
    let mut rows = stmt.query(params![id])?;
    Ok(match rows.next()? {
        Some(r) => Some(row_to_batch(r)?),
        None => None,
    })
}

/// 批次列表（新的在前）
pub fn list(conn: &Connection, limit: u32) -> Result<Vec<ImportBatch>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM import_batches ORDER BY created_at DESC, id DESC LIMIT ?1"
    ))?;
    let mut rows = stmt.query(params![limit.max(1)])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(row_to_batch(r)?);
    }
    Ok(out)
}

/// 标记撤销（幂等：已撤销的不重复写时间，返回是否本次刚改）
pub fn mark_undone(conn: &Connection, id: i64) -> Result<bool> {
    let n = conn.execute(
        "UPDATE import_batches SET undone_at = ?1 WHERE id = ?2 AND undone_at IS NULL",
        params![now_iso(), id],
    )?;
    Ok(n > 0)
}
