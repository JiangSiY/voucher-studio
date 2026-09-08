//! 去重（对应 v4.3 的 `dedup.py`）+ v0.10 语义疑似重复检测
//!
//! 两层去重：
//! - 文件层：MD5 内容哈希（同一张图片不会入库两次）
//! - 语义层：同日期 + 同金额（分）疑似重复（同一票据不同照片/扫描件）

use std::path::Path;

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::db::repos::attachment_repo;
use crate::db::repos::voucher_repo;
use crate::models::{Attachment, Voucher};
use crate::Result;

/// 文件内容 MD5（十六进制小写）
pub fn md5_file(path: &Path) -> Result<String> {
    let data = std::fs::read(path)?;
    Ok(md5_bytes(&data))
}

/// 内存数据 MD5
pub fn md5_bytes(data: &[u8]) -> String {
    format!("{:x}", md5::compute(data))
}

/// 同内容附件是否已存在（含已删除凭证之外的全部附件）
pub fn find_duplicate(conn: &Connection, hash: &str) -> Result<Option<Attachment>> {
    attachment_repo::find_by_hash(conn, hash)
}

// ---------- v0.10：语义疑似重复（同日期 + 同金额） ----------

/// 疑似重复组（全库扫描结果，组内 ≥2 张）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub voucher_date: String,
    pub amount_cents: i64,
    /// 组内全部凭证（按编号排序）
    pub vouchers: Vec<Voucher>,
}

/// 判定口径：同凭证日期 + 同金额（金额非空）。
/// 对方账户不作为硬条件（OCR 回填不稳定，宁多报不漏报——对面在 UI 里供用户比对）。
const DUP_SQL: &str = "SELECT id, voucher_no, voucher_date, description, counterparty, amount_cents, status, tags, note, created_at, updated_at \
     FROM vouchers WHERE voucher_date = ?1 AND amount_cents = ?2 AND id != ?3";

/// 单凭证的疑似重复：库中同日期同金额的其他凭证（金额为空返回空——空金额无判定意义）
pub fn find_suspected(conn: &Connection, v: &Voucher) -> Result<Vec<Voucher>> {
    let Some(amount) = v.amount_cents else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(DUP_SQL)?;
    let mut rows = stmt.query(params![v.voucher_date, amount, v.id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row_to_voucher(row)?);
    }
    Ok(out)
}

/// 全库扫描疑似重复组（同日期 + 同金额、组内 ≥2 张），按日期倒序（最近的先处理）
pub fn scan_groups(conn: &Connection) -> Result<Vec<DuplicateGroup>> {
    let mut stmt = conn.prepare(
        "SELECT voucher_date, amount_cents, COUNT(*) AS c FROM vouchers \
         WHERE amount_cents IS NOT NULL \
         GROUP BY voucher_date, amount_cents HAVING c >= 2 \
         ORDER BY voucher_date DESC, amount_cents DESC",
    )?;
    let keys: Vec<(String, i64)> = {
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push((r.get(0)?, r.get(1)?));
        }
        out
    };

    let mut groups = Vec::new();
    for (date, amount) in keys {
        let mut stmt = conn.prepare(
            "SELECT id, voucher_no, voucher_date, description, counterparty, amount_cents, status, tags, note, created_at, updated_at \
             FROM vouchers WHERE voucher_date = ?1 AND amount_cents = ?2 ORDER BY voucher_no",
        )?;
        let mut rows = stmt.query(params![date, amount])?;
        let mut vouchers = Vec::new();
        while let Some(r) = rows.next()? {
            vouchers.push(row_to_voucher(r)?);
        }
        groups.push(DuplicateGroup {
            voucher_date: date,
            amount_cents: amount,
            vouchers,
        });
    }
    Ok(groups)
}

/// 行 → Voucher（与 voucher_repo::row_to_voucher 同构；此处独立以避免跨模块私有依赖）
fn row_to_voucher(row: &rusqlite::Row<'_>) -> rusqlite::Result<Voucher> {
    use crate::models::VoucherStatus;
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

/// 按凭证 id 批量查疑似重复（导入/OCR 填单后前端提示用）：
/// 与该凭证同日期同金额的其他凭证。
pub fn suspected_by_id(conn: &Connection, id: i64) -> Result<Vec<Voucher>> {
    match voucher_repo::get(conn, id)? {
        Some(v) => find_suspected(conn, &v),
        None => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"hello voucher").unwrap();
        assert_eq!(md5_file(&p).unwrap(), md5_bytes(b"hello voucher"));
        assert_eq!(md5_bytes(b"hello voucher").len(), 32);
    }
}
