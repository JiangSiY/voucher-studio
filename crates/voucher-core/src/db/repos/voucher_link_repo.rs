//! 凭证关联 / 事项组 / 关联审计仓储（v0.22 关联批）
//!
//! 归一化约定：两两关联一律存 `a_id < b_id`（复用 `dup_decision_repo::normalize_pair`），
//! 配合 `(a_id, b_id, kind)` 唯一索引——"同一对同一种关系"只可能有一行，
//! 重复建立＝**覆盖备注**（保留原 `created_at`，判定幂等）。
//!
//! 审计表 `voucher_link_events` **刻意不加外键**（见 `db/schema.rs::SCHEMA_V9`）。

use rusqlite::{params, Connection, OptionalExtension};

use super::dup_decision_repo::normalize_pair;
use std::collections::HashMap;

use crate::models::enums::VoucherLinkKind;
use crate::models::{
    NewVoucherGroupEvent, NewVoucherLink, NewVoucherLinkEvent, VoucherGroupEvent, VoucherLink,
    VoucherLinkEvent,
};
use crate::utils::paths::now_iso;
use crate::Result;

const COLS: &str = "id, a_id, b_id, kind, note, created_at";

fn row_to_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<VoucherLink> {
    let kind: String = row.get(3)?;
    Ok(VoucherLink {
        id: row.get(0)?,
        a_id: row.get(1)?,
        b_id: row.get(2)?,
        kind: VoucherLinkKind::parse(&kind).unwrap_or(VoucherLinkKind::Explains),
        note: row.get(4)?,
        created_at: row.get(5)?,
    })
}

/// 建立/覆盖一条关联，返回其 id（同一对同 kind 幂等）
pub fn upsert(conn: &Connection, l: &NewVoucherLink) -> Result<i64> {
    let (a, b) = normalize_pair(l.a_id, l.b_id);
    conn.execute(
        "INSERT INTO voucher_links (a_id, b_id, kind, note, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(a_id, b_id, kind) DO UPDATE SET note = excluded.note",
        params![a, b, l.kind.as_str(), l.note, now_iso()],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM voucher_links WHERE a_id = ?1 AND b_id = ?2 AND kind = ?3",
        params![a, b, l.kind.as_str()],
        |r| r.get(0),
    )?;
    Ok(id)
}

/// 读一条关联（入参自动归一化）
pub fn get_pair(
    conn: &Connection,
    x: i64,
    y: i64,
    kind: VoucherLinkKind,
) -> Result<Option<VoucherLink>> {
    let (a, b) = normalize_pair(x, y);
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLS} FROM voucher_links WHERE a_id = ?1 AND b_id = ?2 AND kind = ?3"
            ),
            params![a, b, kind.as_str()],
            row_to_link,
        )
        .optional()?)
}

/// 删除一条关联，返回被删的那条（供审计留痕）
pub fn delete(
    conn: &Connection,
    x: i64,
    y: i64,
    kind: VoucherLinkKind,
) -> Result<Option<VoucherLink>> {
    let found = get_pair(conn, x, y, kind)?;
    if let Some(ref l) = found {
        conn.execute("DELETE FROM voucher_links WHERE id = ?1", params![l.id])?;
    }
    Ok(found)
}

/// 某张凭证的全部关联（两个方向都算；新的在前）
pub fn list_of(conn: &Connection, voucher_id: i64) -> Result<Vec<VoucherLink>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM voucher_links WHERE a_id = ?1 OR b_id = ?1
         ORDER BY created_at DESC, id DESC"
    ))?;
    let rows = stmt.query_map(params![voucher_id], row_to_link)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ---------- 审计（append-only） ----------

/// 追加一条审计事件，返回事件 id
pub fn insert_event(conn: &Connection, e: &NewVoucherLinkEvent) -> Result<i64> {
    let (a, b) = normalize_pair(e.a_id, e.b_id);
    conn.execute(
        "INSERT INTO voucher_link_events (link_id, a_id, b_id, kind, action, note, at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            e.link_id,
            a,
            b,
            e.kind.as_str(),
            e.action.as_str(),
            e.note,
            now_iso()
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 与某张凭证相关的事件（任一方向；新→旧）——测试与排查用
pub fn events_of(conn: &Connection, voucher_id: i64) -> Result<Vec<VoucherLinkEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, link_id, a_id, b_id, kind, action, note, at FROM voucher_link_events
         WHERE a_id = ?1 OR b_id = ?1 ORDER BY id DESC",
    )?;
    let rows = stmt.query_map(params![voucher_id], |row| {
        Ok(VoucherLinkEvent {
            id: row.get(0)?,
            link_id: row.get(1)?,
            a_id: row.get(2)?,
            b_id: row.get(3)?,
            kind: row.get(4)?,
            action: row.get(5)?,
            note: row.get(6)?,
            at: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 审计表总行数（测试断言 append-only 用）
pub fn count_events(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM voucher_link_events", [], |r| r.get(0))?)
}

// ---------- 事项组审计（append-only，v10） ----------

/// 追加一条组审计事件，返回事件 id
pub fn insert_group_event(conn: &Connection, e: &NewVoucherGroupEvent) -> Result<i64> {
    conn.execute(
        "INSERT INTO voucher_group_events (group_id, action, voucher_id, note, at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            e.group_id,
            e.action.as_str(),
            e.voucher_id,
            e.note,
            now_iso()
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 某个组的事件（新→旧）——测试与排查用
pub fn group_events_of(conn: &Connection, group_id: i64) -> Result<Vec<VoucherGroupEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, group_id, action, voucher_id, note, at FROM voucher_group_events
         WHERE group_id = ?1 ORDER BY id DESC",
    )?;
    let rows = stmt.query_map(params![group_id], |row| {
        Ok(VoucherGroupEvent {
            id: row.get(0)?,
            group_id: row.get(1)?,
            action: row.get(2)?,
            voucher_id: row.get(3)?,
            note: row.get(4)?,
            at: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 某张凭证相关的组事件（建组/删组这类无 voucher_id 事件不会命中）
pub fn group_events_of_voucher(
    conn: &Connection,
    voucher_id: i64,
) -> Result<Vec<VoucherGroupEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, group_id, action, voucher_id, note, at FROM voucher_group_events
         WHERE voucher_id = ?1 ORDER BY id DESC",
    )?;
    let rows = stmt.query_map(params![voucher_id], |row| {
        Ok(VoucherGroupEvent {
            id: row.get(0)?,
            group_id: row.get(1)?,
            action: row.get(2)?,
            voucher_id: row.get(3)?,
            note: row.get(4)?,
            at: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 组审计表总行数（测试断言）
pub fn count_group_events(conn: &Connection) -> Result<i64> {
    Ok(
        conn.query_row("SELECT COUNT(*) FROM voucher_group_events", [], |r| {
            r.get(0)
        })?,
    )
}

// ---------- 列表徽标用的批量计数 ----------

/// 一批凭证各自的关系条数：`voucher_id -> (explains 条数, detail 条数)`。
///
/// 一次 `IN` 查询覆盖整页（不做 N+1）；两个方向都算——a 端与 b 端各记一次，
/// 与 `list_of` 的口径一致。
pub fn link_counts_in(conn: &Connection, voucher_ids: &[i64]) -> Result<HashMap<i64, (i64, i64)>> {
    let mut out: HashMap<i64, (i64, i64)> = HashMap::new();
    if voucher_ids.is_empty() {
        return Ok(out);
    }
    let wanted: std::collections::HashSet<i64> = voucher_ids.iter().copied().collect();
    let placeholders = vec!["?"; voucher_ids.len()].join(",");
    let sql = format!(
        "SELECT a_id, b_id, kind FROM voucher_links
         WHERE a_id IN ({placeholders}) OR b_id IN ({placeholders})"
    );
    // 两段占位符各要一份绑定值
    let mut args: Vec<i64> = Vec::with_capacity(voucher_ids.len() * 2);
    args.extend_from_slice(voucher_ids);
    args.extend_from_slice(voucher_ids);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for r in rows {
        let (a_id, b_id, kind) = r?;
        for vid in [a_id, b_id] {
            if !wanted.contains(&vid) {
                continue;
            }
            let e = out.entry(vid).or_insert((0, 0));
            if kind == VoucherLinkKind::Explains.as_str() {
                e.0 += 1;
            } else if kind == VoucherLinkKind::Detail.as_str() {
                e.1 += 1;
            }
        }
    }
    Ok(out)
}

/// 一批凭证各自所属的事项组个数：`voucher_id -> 组数`
pub fn group_counts_in(conn: &Connection, voucher_ids: &[i64]) -> Result<HashMap<i64, i64>> {
    let mut out: HashMap<i64, i64> = HashMap::new();
    if voucher_ids.is_empty() {
        return Ok(out);
    }
    let placeholders = vec!["?"; voucher_ids.len()].join(",");
    let sql = format!(
        "SELECT voucher_id, COUNT(DISTINCT group_id) FROM voucher_group_items
         WHERE voucher_id IN ({placeholders}) GROUP BY voucher_id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(voucher_ids.iter()), |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;
    for r in rows {
        let (vid, n) = r?;
        out.insert(vid, n);
    }
    Ok(out)
}

// ---------- 事项组 ----------

/// 新建组，返回 id
pub fn create_group(conn: &Connection, title: &str, note: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO voucher_groups (title, note, created_at) VALUES (?1, ?2, ?3)",
        params![title, note, now_iso()],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 组是否存在
pub fn group_exists(conn: &Connection, id: i64) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM voucher_groups WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// 删除组（成员行随 `ON DELETE CASCADE` 一起走）
pub fn delete_group(conn: &Connection, id: i64) -> Result<usize> {
    Ok(conn.execute("DELETE FROM voucher_groups WHERE id = ?1", params![id])?)
}

/// 组头信息：`(id, title, note, created_at)`
pub fn group_head(conn: &Connection, id: i64) -> Result<Option<(i64, String, String, String)>> {
    Ok(conn
        .query_row(
            "SELECT id, title, note, created_at FROM voucher_groups WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?)
}

/// 全部组头（按创建时间倒序）
pub fn list_group_heads(conn: &Connection) -> Result<Vec<(i64, String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, note, created_at FROM voucher_groups ORDER BY created_at DESC, id DESC",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 组内成员 id（按 `sort_order`）
pub fn member_ids(conn: &Connection, group_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT voucher_id FROM voucher_group_items WHERE group_id = ?1
         ORDER BY sort_order ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![group_id], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 加入成员（已存在则跳过；`sort_order` 追加到末尾），返回**真正新增**的凭证 id。
///
/// 返回 id 而不是行数：服务层要按"真的加进去了"逐张写审计（幂等重复调用不应重复留痕）。
pub fn add_members(conn: &Connection, group_id: i64, voucher_ids: &[i64]) -> Result<Vec<i64>> {
    let mut next: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM voucher_group_items WHERE group_id = ?1",
            params![group_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let mut added: Vec<i64> = Vec::new();
    for vid in voucher_ids {
        let n = conn.execute(
            "INSERT OR IGNORE INTO voucher_group_items (group_id, voucher_id, sort_order)
             VALUES (?1, ?2, ?3)",
            params![group_id, vid, next],
        )?;
        if n > 0 {
            added.push(*vid);
            next += 1;
        }
    }
    Ok(added)
}

/// 移出成员，返回删除行数
pub fn remove_member(conn: &Connection, group_id: i64, voucher_id: i64) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM voucher_group_items WHERE group_id = ?1 AND voucher_id = ?2",
        params![group_id, voucher_id],
    )?)
}

/// 某张凭证所属的组 id 列表
pub fn group_ids_of_voucher(conn: &Connection, voucher_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT group_id FROM voucher_group_items WHERE voucher_id = ?1 ORDER BY group_id ASC",
    )?;
    let rows = stmt.query_map(params![voucher_id], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
