//! 凭证仓储

use std::collections::HashMap;

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
    /// 关键词同时匹配附件 OCR 文本（搜索页进入编辑页后「上一张/下一张」
    /// 与搜索结果同口径，连续核对不跳过仅 OCR 命中的凭证）
    pub include_ocr: bool,
    /// 从 1 开始
    pub page: u32,
    pub page_size: u32,

    // ---- v0.18 扩维：把"待办卡/图表"精确下钻所需的维度补齐 ----
    // 此前只能按状态/关键词/日期筛，标签与对方只能模糊匹配、金额与 OCR 状态无法筛，
    // 于是"未填金额 3 张"这类待办点进去只能给一个近似列表。
    /// **精确**标签筛选：命中其中**任一**标签（多选为"或"；与关键词的模糊匹配区分开）
    pub tags: Vec<String>,
    /// **精确**对方账户筛选（整串相等）
    pub counterparty: Option<String>,
    /// 金额区间（分）；`amount_empty=true` 时优先按"金额为空"筛，忽略区间
    pub amount_from: Option<i64>,
    pub amount_to: Option<i64>,
    /// 只筛金额为空的凭证（对应"未填金额"待办）
    pub amount_empty: bool,
    /// 只筛对方账户未填的凭证（对应月度透视里的「(未填)」行）
    pub counterparty_empty: bool,
    /// 只筛"陈旧"凭证：`updated_at` 早于「现在 − N 天」（对应收件箱的"陈旧草稿"待办，
    /// 与 `dashboard_todos.stale_drafts` 同口径；2026-09-27 审计 P1-2）
    pub stale_days: Option<u32>,
    /// 附件 OCR 状态：`pending` / `running` / `done` / `failed`（对应"待识别"待办）。
    /// **支持多值**（逗号分隔，任一命中）——收件箱的"待识别"计数含 `pending + failed`，
    /// 下钻必须能带上两者，否则「计数 3 张、点进去 1 张」（2026-09-27 审计 P1-1）。
    pub ocr_status: Option<String>,
    /// 导出覆盖度：`never`（从未导出）| `any`（导出过）
    pub exported: Option<String>,
    /// 排序：`date_desc`（默认，与历史一致）| `date_asc` | `updated_desc` | `amount_desc` | `amount_asc`
    pub sort: Option<String>,
}

impl VoucherQuery {
    /// ORDER BY 子句（默认与历史行为完全一致：日期倒序 + id 倒序作稳定次序）
    fn order_by(&self) -> &'static str {
        match self.sort.as_deref() {
            Some("date_asc") => "ORDER BY voucher_date ASC, id ASC",
            // 金额为空的排在最后（NULL 在 SQLite 的 ASC 里最小，DESC 里最大 → 显式处理）
            Some("amount_desc") => "ORDER BY amount_cents IS NULL, amount_cents DESC, id DESC",
            Some("amount_asc") => "ORDER BY amount_cents IS NULL, amount_cents ASC, id DESC",
            Some("updated_desc") => "ORDER BY updated_at DESC, id DESC",
            _ => "ORDER BY voucher_date DESC, id DESC",
        }
    }
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

/// 全列清单（dedup / reconcile 等服务层查询共用，保证行映射口径唯一）
pub(crate) const COLS: &str =
    "id, voucher_no, voucher_date, description, counterparty, amount_cents, status, tags, note, created_at, updated_at";

/// 行 → Voucher 唯一映射（vouchers 表结构变更只需改这一处）
pub(crate) fn row_to_voucher(row: &rusqlite::Row<'_>) -> rusqlite::Result<Voucher> {
    let tags_json: String = row.get(7)?;
    let status: String = row.get(6)?;
    Ok(Voucher {
        id: row.get(0)?,
        voucher_no: row.get(1)?,
        voucher_date: row.get(2)?,
        description: row.get(3)?,
        counterparty: row.get(4)?,
        amount_cents: row.get(5)?,
        status: VoucherStatus::parse(&status).unwrap_or(VoucherStatus::Draft),
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        note: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
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
        "INSERT INTO vouchers (voucher_no, voucher_date, description, counterparty, amount_cents, status, tags, note, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![
            v.voucher_no,
            date,
            v.description,
            v.counterparty,
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
    // 空编号视为「不修改」（防御：空串会破坏 NOT NULL UNIQUE 语义）
    let voucher_no = match u.voucher_no.as_deref().map(str::trim) {
        Some(no) if !no.is_empty() => {
            if no != old_no && get_by_no(conn, no)?.is_some() {
                return Err(CoreError::Invalid(format!("凭证编号 {no} 已存在")));
            }
            no.to_string()
        }
        _ => old_no.clone(),
    };
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
        "UPDATE vouchers SET voucher_no=?1, voucher_date=?2, description=?3, counterparty=?4, amount_cents=?5,
         status=?6, tags=?7, note=?8, updated_at=?9 WHERE id=?10",
        params![
            voucher_no,
            u.voucher_date.clone().unwrap_or(current.voucher_date),
            u.description.clone().unwrap_or(current.description),
            u.counterparty.clone().unwrap_or(current.counterparty),
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
    let (where_sql, bind) = build_filter_where(q);

    let total: u64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM vouchers {where_sql}"),
        rusqlite::params_from_iter(bind.iter()),
        |r| r.get::<_, i64>(0),
    )? as u64;

    let page_size = q.page_size();
    let page = q.page().max(1);
    let offset = ((page - 1) * page_size) as i64;
    let sql = format!(
        "SELECT {COLS} FROM vouchers {where_sql} {} LIMIT {page_size} OFFSET {offset}",
        q.order_by()
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(bind.iter()))?;
    let mut items = Vec::new();
    while let Some(r) = rows.next()? {
        items.push(row_to_voucher(r)?);
    }
    Ok((items, total))
}

/// 按当前筛选**只取 id**（顺序与列表一致）。
///
/// 用途（v0.18 仪表盘批）：列表页「按筛选全选 N 条」——跨页选择集只需 id，
/// 不必把整页对象拉回前端；批删/批改仍是既有写路径，不新增写命令。
pub fn list_ids(conn: &Connection, q: &VoucherQuery) -> Result<Vec<i64>> {
    let (where_sql, bind) = build_filter_where(q);
    let sql = format!("SELECT id FROM vouchers {where_sql} ORDER BY voucher_date DESC, id DESC");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(bind.iter()))?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(r.get(0)?);
    }
    Ok(out)
}

/// 按 id 集合批量取凭证（导出批次反查用；返回顺序与传入顺序一致，不存在的跳过）
pub fn get_many(conn: &Connection, ids: &[i64]) -> Result<Vec<Voucher>> {
    let mut out = Vec::with_capacity(ids.len());
    // IN 参数分批（SQLite 变量数上限保守取 500）
    for chunk in ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!("SELECT {COLS} FROM vouchers WHERE id IN ({placeholders})");
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(chunk.iter()))?;
        let mut found: HashMap<i64, Voucher> = HashMap::new();
        while let Some(r) = rows.next()? {
            let v = row_to_voucher(r)?;
            found.insert(v.id, v);
        }
        // 保持传入顺序
        for id in chunk {
            if let Some(v) = found.remove(id) {
                out.push(v);
            }
        }
    }
    Ok(out)
}

/// 构造筛选 WHERE 子句（list 与 neighbor 共用同一口径，保证编辑页导航与列表所见一致）。
/// 返回 ("WHERE ..." 或空串, 绑定参数列表)。
fn build_filter_where(q: &VoucherQuery) -> (String, Vec<String>) {
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
        let mut cond = format!(
            "(voucher_no LIKE ?{i} ESCAPE '\\' OR description LIKE ?{i} ESCAPE '\\' OR counterparty LIKE ?{i} ESCAPE '\\' OR note LIKE ?{i} ESCAPE '\\' OR tags LIKE ?{i} ESCAPE '\\'"
        );
        if q.include_ocr {
            cond.push_str(&format!(
                " OR EXISTS (SELECT 1 FROM attachments a WHERE a.voucher_id = vouchers.id AND a.ocr_text LIKE ?{i} ESCAPE '\\')"
            ));
        }
        cond.push(')');
        where_clauses.push(cond);
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
    // 精确标签：tags 是 JSON 数组文本，用 json_each 展开后等值匹配（任一命中即算）
    let tags: Vec<String> = q
        .tags
        .iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if !tags.is_empty() {
        let ph: Vec<String> = tags
            .iter()
            .map(|t| {
                bind.push(t.clone());
                format!("?{}", bind.len())
            })
            .collect();
        where_clauses.push(format!(
            "EXISTS (SELECT 1 FROM json_each(vouchers.tags) je WHERE je.value IN ({}))",
            ph.join(",")
        ));
    }
    if q.counterparty_empty {
        where_clauses.push("TRIM(COALESCE(counterparty,'')) = ''".to_string());
    } else if let Some(cp) = q
        .counterparty
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        where_clauses.push(format!("TRIM(counterparty) = ?{}", bind.len() + 1));
        bind.push(cp.to_string());
    }
    if q.amount_empty {
        where_clauses.push("amount_cents IS NULL".to_string());
    } else {
        if let Some(from) = q.amount_from {
            where_clauses.push(format!("amount_cents >= ?{}", bind.len() + 1));
            bind.push(from.to_string());
        }
        if let Some(to) = q.amount_to {
            where_clauses.push(format!("amount_cents <= ?{}", bind.len() + 1));
            bind.push(to.to_string());
        }
    }
    // OCR 状态：逗号分隔的多值（任一命中即算）——与"待识别"计数的口径对齐
    let statuses: Vec<String> = q
        .ocr_status
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if !statuses.is_empty() {
        let ph: Vec<String> = statuses
            .iter()
            .map(|s| {
                bind.push(s.clone());
                format!("?{}", bind.len())
            })
            .collect();
        where_clauses.push(format!(
            "EXISTS (SELECT 1 FROM attachments a WHERE a.voucher_id = vouchers.id AND a.ocr_status IN ({}))",
            ph.join(",")
        ));
    }

    // 陈旧草稿：`updated_at` 早于"当前时刻 − N 天"（收件箱的"陈旧草稿"计数同口径）
    if let Some(days) = q.stale_days.filter(|d| *d > 0) {
        where_clauses.push(format!("updated_at < ?{}", bind.len() + 1));
        bind.push(
            (chrono::Local::now() - chrono::Duration::days(i64::from(days)))
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string(),
        );
    }
    match q.exported.as_deref() {
        Some("never") => where_clauses.push(
            "NOT EXISTS (SELECT 1 FROM export_items ei WHERE ei.voucher_id = vouchers.id)"
                .to_string(),
        ),
        Some("any") => where_clauses.push(
            "EXISTS (SELECT 1 FROM export_items ei WHERE ei.voucher_id = vouchers.id)".to_string(),
        ),
        _ => {}
    }

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };
    (where_sql, bind)
}

/// 相邻凭证信息（编辑页上下文导航用）
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherNeighbor {
    /// 列表序上一张（更新；无则 None = 已是第一张）
    pub prev_id: Option<i64>,
    /// 列表序下一张（更早；无则 None = 已是最后一张）
    pub next_id: Option<i64>,
    /// 当前凭证在筛选结果中的位置（从 1 开始）
    pub position: u64,
    /// 筛选结果总数
    pub total: u64,
}

/// 同筛选/同排序（voucher_date DESC, id DESC，与 list 一致）下的相邻凭证与位置。
/// 当前凭证自身**不参与**筛选匹配——即使它已被筛除（如改状态后不再匹配 draft 筛选），
/// 仍以其键位为基准找相邻的匹配凭证，支撑「保存并下一张」的连续核对动线。
pub fn neighbor(conn: &Connection, id: i64, q: &VoucherQuery) -> Result<VoucherNeighbor> {
    let cur_date: String = conn
        .query_row(
            "SELECT voucher_date FROM vouchers WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|_| CoreError::NotFound(format!("凭证 {id} 不存在")))?;

    let (where_sql, filter_bind) = build_filter_where(q);
    // 键参数（日期/ID）追加在筛选参数之后；同一条 SQL 内编号参数可复用
    let di = filter_bind.len() + 1;
    let ii = filter_bind.len() + 2;
    let mut bind: Vec<rusqlite::types::Value> = filter_bind
        .iter()
        .map(|s| rusqlite::types::Value::Text(s.clone()))
        .collect();
    bind.push(rusqlite::types::Value::Text(cur_date.clone()));
    bind.push(rusqlite::types::Value::Integer(id));

    // 筛选条件与键位条件拼接（筛选为空时键位条件顶上 WHERE）
    let combined = |key_cond: &str| -> String {
        if where_sql.is_empty() {
            format!("WHERE {key_cond}")
        } else {
            format!("{where_sql} AND {key_cond}")
        }
    };

    // 下一张 = 列表序更早（键更小）的第一条匹配凭证
    let next_sql = format!(
        "SELECT id FROM vouchers {} ORDER BY voucher_date DESC, id DESC LIMIT 1",
        combined(&format!(
            "(voucher_date < ?{di} OR (voucher_date = ?{di} AND id < ?{ii}))"
        ))
    );
    let next_id = query_optional_id(conn, &next_sql, &bind)?;

    // 上一张 = 列表序更新（键更大）的第一条匹配凭证
    let prev_sql = format!(
        "SELECT id FROM vouchers {} ORDER BY voucher_date ASC, id ASC LIMIT 1",
        combined(&format!(
            "(voucher_date > ?{di} OR (voucher_date = ?{di} AND id > ?{ii}))"
        ))
    );
    let prev_id = query_optional_id(conn, &prev_sql, &bind)?;

    // 位置 = 列表序更靠前（键更大）的匹配数 + 1
    let pos_sql = format!(
        "SELECT COUNT(*) FROM vouchers {}",
        combined(&format!(
            "(voucher_date > ?{di} OR (voucher_date = ?{di} AND id > ?{ii}))"
        ))
    );
    let before: i64 = conn.query_row(&pos_sql, rusqlite::params_from_iter(bind.iter()), |r| {
        r.get(0)
    })?;

    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM vouchers {where_sql}"),
        rusqlite::params_from_iter(filter_bind.iter()),
        |r| r.get(0),
    )?;

    Ok(VoucherNeighbor {
        prev_id,
        next_id,
        position: (before + 1).max(0) as u64,
        total: total.max(0) as u64,
    })
}

/// 单值 id 查询：无行返回 None，其余错误透传
fn query_optional_id(
    conn: &Connection,
    sql: &str,
    bind: &[rusqlite::types::Value],
) -> Result<Option<i64>> {
    match conn.query_row(sql, rusqlite::params_from_iter(bind.iter()), |r| r.get(0)) {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// LIKE 通配符转义（配合 `ESCAPE '\'` 使用）
pub fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// 跨字段搜索：编号/摘要/备注/标签/附件 OCR 文本（对应 v4.3 的 `services/search.py`）
/// v0.11 支持组合筛选（日期范围 + 金额范围；None = 不过滤）
pub fn search(
    conn: &Connection,
    keyword: &str,
    filters: &SearchFilters,
    limit: u32,
) -> Result<Vec<Voucher>> {
    let kw = keyword.trim();
    if kw.is_empty() {
        return Ok(Vec::new());
    }
    let like = format!("%{}%", escape_like(kw));

    // v0.11 组合筛选：动态拼接 AND 子句（参数从 ?2 起绑定）
    let mut conds = Vec::new();
    let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(like)];
    if let Some(d) = filters.date_from.as_deref() {
        if !d.is_empty() {
            conds.push(format!("v.voucher_date >= ?{}", binds.len() + 1));
            binds.push(Box::new(d.to_string()));
        }
    }
    if let Some(d) = filters.date_to.as_deref() {
        if !d.is_empty() {
            conds.push(format!("v.voucher_date <= ?{}", binds.len() + 1));
            binds.push(Box::new(d.to_string()));
        }
    }
    if let Some(a) = filters.amount_from {
        conds.push(format!("v.amount_cents >= ?{}", binds.len() + 1));
        binds.push(Box::new(a));
    }
    if let Some(a) = filters.amount_to {
        conds.push(format!("v.amount_cents <= ?{}", binds.len() + 1));
        binds.push(Box::new(a));
    }
    let filter_sql = if conds.is_empty() {
        String::new()
    } else {
        format!(" AND ({})", conds.join(" AND "))
    };

    let sql = format!(
        "SELECT DISTINCT v.id, v.voucher_no, v.voucher_date, v.description, v.counterparty, v.amount_cents, v.status, v.tags, v.note, v.created_at, v.updated_at
         FROM vouchers v
         LEFT JOIN attachments a ON a.voucher_id = v.id
         WHERE (v.voucher_no LIKE ?1 ESCAPE '\\'
            OR v.description LIKE ?1 ESCAPE '\\'
            OR v.counterparty LIKE ?1 ESCAPE '\\'
            OR v.note LIKE ?1 ESCAPE '\\'
            OR v.tags LIKE ?1 ESCAPE '\\'
            OR a.ocr_text LIKE ?1 ESCAPE '\\'){filter_sql}
         ORDER BY v.voucher_date DESC, v.id DESC
         LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())))?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(row_to_voucher(r)?);
    }
    Ok(out)
}

/// v0.11 搜索组合筛选（全部 None = 不过滤；金额单位：分）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchFilters {
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub amount_from: Option<i64>,
    pub amount_to: Option<i64>,
}

/// 前缀计数：命名服务用它算当日序号
pub fn count_with_prefix(conn: &Connection, prefix: &str) -> Result<i64> {
    let like = format!("{}%", escape_like(prefix));
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM vouchers WHERE voucher_no LIKE ?1 ESCAPE '\\'",
        params![like],
        |r| r.get(0),
    )?)
}

/// v0.11：某自然年全部凭证 id（按日期+编号升序，年度报表批次明细用）
pub fn ids_of_year(conn: &Connection, year: i32) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM vouchers WHERE substr(voucher_date, 1, 4) = ?1
         ORDER BY voucher_date, voucher_no",
    )?;
    let rows = stmt
        .query_map(params![format!("{year:04}")], |r| r.get::<_, i64>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
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
