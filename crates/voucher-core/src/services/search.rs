//! 搜索服务（语义对齐 v4.3 的 `services/search.py`：关键词命中凭证字段与附件 OCR 文本）

use crate::db::repos::voucher_repo;
use crate::models::Voucher;
use crate::{CoreContext, Result};

/// 搜索结果：凭证 + 命中位置说明
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub voucher: Voucher,
    /// 命中位置：编号 / 摘要 / 备注 / 标签 / OCR 文本
    pub matched_in: Vec<String>,
}

pub fn search_vouchers(ctx: &CoreContext, keyword: &str, limit: u32) -> Result<Vec<SearchHit>> {
    let limit = if limit == 0 { 50 } else { limit };
    run(ctx, keyword, limit)
}

fn run(ctx: &CoreContext, keyword: &str, limit: u32) -> Result<Vec<SearchHit>> {
    ctx.db.with_conn(|conn| {
        let vouchers = voucher_repo::search(conn, keyword, limit)?;
        if vouchers.is_empty() {
            return Ok(Vec::new());
        }
        let kw = keyword.trim().to_lowercase();

        // OCR 命中的凭证 id 集合：单次查询（避免逐凭证 N+1），LIKE 转义防通配符误匹配
        let like = format!("%{}%", voucher_repo::escape_like(keyword.trim()));
        let ocr_ids: std::collections::HashSet<i64> = {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT voucher_id FROM attachments WHERE ocr_text LIKE ?1 ESCAPE '\\'",
            )?;
            let mut rows = stmt.query(rusqlite::params![like])?;
            let mut set = std::collections::HashSet::new();
            while let Some(r) = rows.next()? {
                set.insert(r.get::<_, i64>(0)?);
            }
            set
        };

        Ok(vouchers
            .into_iter()
            .map(|v| {
                let mut matched_in = Vec::new();
                if v.voucher_no.to_lowercase().contains(&kw) {
                    matched_in.push("编号".into());
                }
                if v.description.to_lowercase().contains(&kw) {
                    matched_in.push("摘要".into());
                }
                if v.note.to_lowercase().contains(&kw) {
                    matched_in.push("备注".into());
                }
                if v.tags.iter().any(|t| t.to_lowercase().contains(&kw)) {
                    matched_in.push("标签".into());
                }
                if ocr_ids.contains(&v.id) {
                    matched_in.push("OCR".into());
                }
                if matched_in.is_empty() {
                    matched_in.push("全文".into());
                }
                SearchHit {
                    voucher: v,
                    matched_in,
                }
            })
            .collect())
    })
}
