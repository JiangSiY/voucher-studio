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
        let kw = keyword.trim().to_lowercase();
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
                // OCR 命中：凭证没在自身字段命中时也要查
                let ocr_hit: bool = conn
                    .query_row(
                        "SELECT COUNT(*) FROM attachments WHERE voucher_id = ?1 AND ocr_text LIKE ?2",
                        rusqlite::params![v.id, format!("%{kw}%")],
                        |r| r.get::<_, i64>(0),
                    )
                    .map(|n| n > 0)
                    .unwrap_or(false);
                if ocr_hit {
                    matched_in.push("OCR".into());
                }
                if matched_in.is_empty() {
                    matched_in.push("全文".into());
                }
                SearchHit { voucher: v, matched_in }
            })
            .collect())
    })
}
