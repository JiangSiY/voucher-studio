//! 凭证仓储

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::models::{NewVoucher, Voucher, VoucherStatus, VoucherUpdate};
use crate::utils::paths::{now_iso, today};
use crate::{CoreError, Result};

/// 列表查询条件
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VoucherQuery {
    pub status: Option<VoucherStatus>,
    /// 编号/摘要/备注/标签 关键词
    pub keyword: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    /// 从 1 开始
    pub page: u32,
    pub page_size: u32,
}

impl VoucherQuery {
    fn page_size(&self) -> u32 {
        if self.page_size == 0 {
            20
        } else {
            self.page_size
        }
    }

    fn page(&self) -> u32 {
        if self.page == 0 {
            1
        } else {
            self.page
        }
    }
}

/// 统计（仪表盘用）
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherStats {
    pub total: i64,
    pub draft: i64,
    pub reviewed: i64,
    pub exported: i64,
    pub archived: i64,
    pub attachments: i64,
    pub this_month: i64,
}

const COLS: &str =
    "id, voucher_no, voucher_date, description, amount_cents, status, tags, note, created_at, updated_at";

fn row_to_voucher(row: &rusqlite::Row<'_>) -> rusqlite::Result<Voucher> {
    let tags_json: String = row.get(6)?;
    let status: String = row.get(5)?;
    Ok(Voucher {
        id: row.get(0)?,
        voucher_no: row.get(1)?,
        voucher_date: row.get(2)?,
        description: row.get(3)?,
        amount_cents: row.get(4)?,
        status: VoucherStatus::parse(&status).unwrap_or(VoucherStatus::Draft),
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        note: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

pub fn insert(conn: &Connection, v: &NewVoucher) -> Result<Voucher> {
    let now = now_iso();
    let date = if v.voucher_date.trim().is_empty() {
        today()
    } else {
        v.voucher_date.clone()
    };
    let status = v.status.unwrap_or(VoucherStatus::Draft);
    conn.execute(
        "INSERT INTO vouchers (voucher_no, voucher_date, description, amount_cents, status, tags, note, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            v.voucher_no,
            date,
            v.description,
            v.amount_cents,
            status.as_str(),
            serde_json::to_string(&v.tags)?,
            v.note,
            now
        ],
    )?;
    get(conn, conn.last_insert_rowid())?
        .ok_or_else(|| CoreError::Invalid("凭证插入后读取失败".into()))
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Voucher>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM vouchers WHERE id = ?1"))?;
    let mut rows = stmt.query(params![id])?;
    Ok(match rows.next()? {
        Some(r) => Some(row_to_voucher(r)?),
        None => None,
    })
}

pub fn get_by_no(conn: &Connection, voucher_no: &str) -> Result<Option<Voucher>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM vouchers WHERE voucher_no = ?1"
    ))?;
    let mut rows = stmt.query(params![voucher_no])?;
    Ok(match rows.next()? {
        Some(r) => Some(row_to_voucher(r)?),
        None => None,
    })
}

/// 返回是否确实更新
pub fn update(conn: &Connection, id: i64, u: &VoucherUpdate) -> Result<bool> {
    let current = match get(conn, id)? {
        Some(v) => v,
        None => return Ok(false),
    };
    let old_no = current.voucher_no.clone();
    let voucher_no = u.voucher_no.clone().unwrap_or_else(|| old_no.clone());
    if let Some(no) = &u.voucher_no {
        if no != &old_no && get_by_no(conn, no)?.is_some() {
            return Err(CoreError::Invalid(format!("凭证编号 {no} 已存在")));
        }
    }
    let amount_cents = if u.clear_amount {
        None
    } else {
        u.amount_cents.or(current.amount_cents)
    };
    let status = u.status.unwrap_or(current.status).as_str();
    let tags = match &u.tags {
        Some(t) => serde_json::to_string(t)?,
        None => serde_json::to_string(&current.tags)?,
    };
    let n = conn.execute(
        "UPDATE vouchers SET voucher_no=?1, voucher_date=?2, description=?3, amount_cents=?4,
         status=?5, tags=?6, note=?7, updated_at=?8 WHERE id=?9",
        params![
            voucher_no,
            u.voucher_date.clone().unwrap_or(current.voucher_date),
            u.description.clone().unwrap_or(current.description),
            amount_cents,
            status,
            tags,
            u.note.clone().unwrap_or(current.note),
            now_iso(),
            id
        ],
    )?;
    Ok(n > 0)
}

pub fn delete(conn: &Connection, id: i64) -> Result<bool> {
    let n = conn.execute("DELETE FROM vouchers WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

pub fn set_status(conn: &Connection, id: i64, status: VoucherStatus) -> Result<()> {
    conn.execute(
        "UPDATE vouchers SET status=?1, updated_at=?2 WHERE id=?3",
        params![status.as_str(), now_iso(), id],
    )?;
    Ok(())
}

/// 分页列表，返回 (当前页数据, 总条数)
pub fn list(conn: &Connection, q: &VoucherQuery) -> Result<(Vec<Voucher>, u64)> {
    let mut where_clauses: Vec<String> = Vec::new();
    let mut bind: Vec<String> = Vec::new();

    if let Some(s) = &q.status {
        where_clauses.push(format!("status = ?{}", bind.len() + 1));
        bind.push(s.as_str().to_string());
    }
    if let Some(k) = q
        .keyword
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        let kw = format!("%{}%", escape_like(k));
        let i = bind.len() + 1;
        where_clauses.push(format!(
            "(voucher_no LIKE ?{i} ESCAPE '\\' OR description LIKE ?{i} ESCAPE '\\' OR note LIKE ?{i} ESCAPE '\\' OR tags LIKE ?{i} ESCAPE '\\')"
        ));
        bind.push(kw);
    }
    if let Some(d) = &q.date_from {
        where_clauses.push(format!("voucher_date >= ?{}", bind.len() + 1));
        bind.push(d.clone());
    }
    if let Some(d) = &q.date_to {
        where_clauses.push(format!("voucher_date <= ?{}", bind.len() + 1));
        bind.push(d.clone());
    }

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    let total: u64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM vouchers {where_sql}"),
        rusqlite::params_from_iter(bind.iter()),
        |r| r.get::<_, i64>(0),
    )? as u64;

    let page_size = q.page_size();
    let page = q.page().max(1);
    let offset = ((page - 1) * page_size) as i64;
    let sql = format!(
        "SELECT {COLS} FROM vouchers {where_sql} ORDER BY voucher_date DESC, id DESC LIMIT {page_size} OFFSET {offset}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(bind.iter()))?;
    let mut items = Vec::new();
    while let Some(r) = rows.next()? {
        items.push(row_to_voucher(r)?);
    }
    Ok((items, total))
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// 跨字段搜索：编号/摘要/备注/标签/附件 OCR 文本（对应 v4.3 的 `services/search.py`）
pub fn search(conn: &Connection, keyword: &str, limit: u32) -> Result<Vec<Voucher>> {
    let kw = keyword.trim();
    if kw.is_empty() {
        return Ok(Vec::new());
    }
    let like = format!("%{}%", escape_like(kw));
    let sql = format!(
        "SELECT DISTINCT v.id, v.voucher_no, v.voucher_date, v.description, v.amount_cents, v.status, v.tags, v.note, v.created_at, v.updated_at
         FROM vouchers v
         LEFT JOIN attachments a ON a.voucher_id = v.id
         WHERE v.voucher_no LIKE ?1 ESCAPE '\\'
            OR v.description LIKE ?1 ESCAPE '\\'
            OR v.note LIKE ?1 ESCAPE '\\'
            OR v.tags LIKE ?1 ESCAPE '\\'
            OR a.ocr_text LIKE ?1 ESCAPE '\\'
         ORDER BY v.voucher_date DESC, v.id DESC
         LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params![like])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(row_to_voucher(r)?);
    }
    Ok(out)
}

/// 前缀计数：命名服务用它算当日序号
pub fn count_with_prefix(conn: &Connection, prefix: &str) -> Result<i64> {
    let like = format!(
        "{}%",
        prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM vouchers WHERE voucher_no LIKE ?1 ESCAPE '\\'",
        params![like],
        |r| r.get(0),
    )?)
}

pub fn stats(conn: &Connection) -> Result<VoucherStats> {
    let mut s = VoucherStats::default();
    let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM vouchers GROUP BY status")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let status: String = r.get(0)?;
        let count: i64 = r.get(1)?;
        s.total += count;
        match VoucherStatus::parse(&status) {
            Some(VoucherStatus::Draft) => s.draft = count,
            Some(VoucherStatus::Reviewed) => s.reviewed = count,
            Some(VoucherStatus::Exported) => s.exported = count,
            Some(VoucherStatus::Archived) => s.archived = count,
            None => {}
        }
    }
    s.attachments = conn.query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0))?;
    let month_prefix = chrono::Local::now().format("%Y-%m").to_string();
    s.this_month = conn.query_row(
        "SELECT COUNT(*) FROM vouchers WHERE voucher_date LIKE ?1",
        params![format!("{month_prefix}%")],
        |r| r.get(0),
    )?;
    Ok(s)
}
