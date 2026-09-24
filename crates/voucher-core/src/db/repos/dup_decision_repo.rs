//! 疑似重复处置记录仓储（v0.16）
//!
//! 归一化约定：`a_id < b_id` 后再落库，因此"同一对"只可能有一行（配合唯一索引），
//! 判定幂等（重复判定同一对 = 覆盖旧的结论与备注）。

use std::collections::HashSet;

use rusqlite::{params, Connection};

use crate::models::dup_decision::{DupDecision, NewDupDecision};
use crate::models::enums::DupDecisionKind;
use crate::utils::paths::now_iso;
use crate::Result;

const COLS: &str =
    "id, a_id, b_id, decision, keep_id, removed_id, removed_payload, note, created_at";

fn row_to_decision(row: &rusqlite::Row<'_>) -> rusqlite::Result<DupDecision> {
    let kind: String = row.get(3)?;
    let payload: Option<String> = row.get(6)?;
    Ok(DupDecision {
        id: row.get(0)?,
        a_id: row.get(1)?,
        b_id: row.get(2)?,
        decision: DupDecisionKind::parse(&kind).unwrap_or(DupDecisionKind::NotDuplicate),
        keep_id: row.get(4)?,
        removed_id: row.get(5)?,
        removed_payload: payload.and_then(|s| serde_json::from_str(&s).ok()),
        note: row.get(7)?,
        created_at: row.get(8)?,
    })
}

/// 归一化一对 id（小在前）
pub fn normalize_pair(x: i64, y: i64) -> (i64, i64) {
    if x <= y {
        (x, y)
    } else {
        (y, x)
    }
}

/// 写入/覆盖一条处置记录（同一对幂等：`ON CONFLICT` 覆盖结论与备注，保留原 id）
pub fn upsert(conn: &Connection, d: &NewDupDecision) -> Result<i64> {
    let (a, b) = normalize_pair(d.a_id, d.b_id);
    let payload: Option<String> = d.removed_payload.as_ref().map(|v| v.to_string());
    conn.execute(
        "INSERT INTO dup_decisions (a_id, b_id, decision, keep_id, removed_id, removed_payload, note, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(a_id, b_id) DO UPDATE SET
             decision = excluded.decision,
             keep_id = excluded.keep_id,
             removed_id = excluded.removed_id,
             removed_payload = excluded.removed_payload,
             note = excluded.note,
             created_at = excluded.created_at",
        params![
            a,
            b,
            d.decision.as_str(),
            d.keep_id,
            d.removed_id,
            payload,
            d.note,
            now_iso()
        ],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM dup_decisions WHERE a_id = ?1 AND b_id = ?2",
        params![a, b],
        |r| r.get(0),
    )?;
    Ok(id)
}

/// 查某一对的处置记录（顺序无关）
pub fn find_by_pair(conn: &Connection, x: i64, y: i64) -> Result<Option<DupDecision>> {
    let (a, b) = normalize_pair(x, y);
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM dup_decisions WHERE a_id = ?1 AND b_id = ?2"
    ))?;
    let mut rows = stmt.query(params![a, b])?;
    match rows.next()? {
        Some(r) => Ok(Some(row_to_decision(r)?)),
        None => Ok(None),
    }
}

/// 全部已判定过的对（归一化），供"待核查队列"排除与保存提示过滤
pub fn decided_pairs(conn: &Connection) -> Result<HashSet<(i64, i64)>> {
    let mut stmt = conn.prepare("SELECT a_id, b_id FROM dup_decisions")?;
    let mut rows = stmt.query([])?;
    let mut out = HashSet::new();
    while let Some(r) = rows.next()? {
        out.insert((r.get(0)?, r.get(1)?));
    }
    Ok(out)
}

/// 处置历史（新 → 旧）
pub fn list_recent(conn: &Connection, limit: u32) -> Result<Vec<DupDecision>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM dup_decisions ORDER BY id DESC LIMIT {limit}"
    ))?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(row_to_decision(r)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_pair_orders_ascending() {
        assert_eq!(normalize_pair(3, 7), (3, 7));
        assert_eq!(normalize_pair(7, 3), (3, 7));
        assert_eq!(normalize_pair(5, 5), (5, 5));
    }
}
