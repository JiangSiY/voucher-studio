//! 快照仓储

use rusqlite::{params, Connection};

use crate::models::snapshot::NewSnapshot;
use crate::models::VoucherSnapshot;
use crate::utils::paths::now_iso;
use crate::Result;

pub fn insert(conn: &Connection, s: &NewSnapshot) -> Result<i64> {
    conn.execute(
        "INSERT INTO snapshots (voucher_id, payload, reason, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![s.voucher_id, s.payload.to_string(), s.reason, now_iso()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_by_voucher(
    conn: &Connection,
    voucher_id: i64,
    limit: u32,
) -> Result<Vec<VoucherSnapshot>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, voucher_id, payload, reason, created_at FROM snapshots WHERE voucher_id = ?1 ORDER BY id DESC LIMIT {limit}"
    ))?;
    let mut rows = stmt.query(params![voucher_id])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(VoucherSnapshot {
            id: r.get(0)?,
            voucher_id: r.get(1)?,
            payload: serde_json::from_str(&r.get::<_, String>(2)?)
                .unwrap_or(serde_json::Value::Null),
            reason: r.get(3)?,
            created_at: r.get(4)?,
        });
    }
    Ok(out)
}
