//! 导出批次服务（对应 v4.3 的 `services/export.py`）
//!
//! 一次导出 = 生成一份 PDF + 落一条批次记录 + 明细 + 凭证状态流转为「已导出」。

use serde::{Deserialize, Serialize};

use crate::db::repos::{attachment_repo, export_repo, snapshot_repo, voucher_repo};
use crate::models::{ExportBatch, VoucherStatus};
use crate::services::pdf::{self, PdfCard, PdfImage, PdfInput, PdfOptions};
use crate::{CoreContext, CoreError, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExportOptions {
    pub title: String,
    /// standard | clean | ledger
    pub card_style: String,
    /// 1 | 2 | 3
    pub columns: u8,
    pub include_images: bool,
}

impl ExportOptions {
    fn from_print_config(cfg: &crate::utils::config::PrintConfig) -> Self {
        Self {
            title: cfg.title.clone(),
            card_style: cfg.card_style.clone(),
            columns: cfg.columns,
            include_images: cfg.include_images,
        }
    }
}

/// 按默认打印配置导出
pub fn export_with_defaults(ctx: &CoreContext, voucher_ids: &[i64]) -> Result<ExportBatch> {
    let opts = ExportOptions::from_print_config(&ctx.config().print);
    export_vouchers_pdf(ctx, voucher_ids, &opts)
}

pub fn export_vouchers_pdf(
    ctx: &CoreContext,
    voucher_ids: &[i64],
    opts: &ExportOptions,
) -> Result<ExportBatch> {
    if voucher_ids.is_empty() {
        return Err(CoreError::Invalid("未选择要导出的凭证".into()));
    }

    // 取凭证与附件（按日期+编号排序，保持打印顺序稳定）
    let mut items = Vec::new();
    ctx.db.with_conn(|conn| {
        for id in voucher_ids {
            let v = voucher_repo::get(conn, *id)?
                .ok_or_else(|| CoreError::NotFound(format!("凭证 {id} 不存在")))?;
            let atts = attachment_repo::list_by_voucher(conn, *id)?;
            items.push((v, atts));
        }
        Ok(())
    })?;
    items.sort_by(|a, b| {
        (a.0.voucher_date.clone(), a.0.voucher_no.clone())
            .cmp(&(b.0.voucher_date.clone(), b.0.voucher_no.clone()))
    });

    let input = PdfInput {
        cards: items
            .iter()
            .map(|(v, _)| PdfCard {
                voucher_no: v.voucher_no.clone(),
                voucher_date: v.voucher_date.clone(),
                description: v.description.clone(),
                amount_label: v.amount_label(),
                tags_line: v.tags.join(" · "),
            })
            .collect(),
        images: items
            .iter()
            .flat_map(|(v, atts)| {
                atts.iter().filter(|a| a.is_image()).map(move |a| PdfImage {
                    path: ctx.paths.attachment_abs(&a.file_path),
                    caption: format!("{} {}", v.voucher_no, a.file_name),
                })
            })
            .collect(),
    };

    // 批次号：EXP-YYYYMMDD-HHMMSS（冲突时追加序号）
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut batch_no = format!("EXP-{stamp}");
    ctx.db.with_conn(|conn| {
        let mut n = 2;
        while export_repo::batch_no_exists(conn, &batch_no)? {
            batch_no = format!("EXP-{stamp}-{n}");
            n += 1;
        }
        Ok(())
    })?;

    let pdf_path = ctx.paths.exports.join(format!("{batch_no}.pdf"));
    pdf::generate(
        &pdf_path,
        &input,
        &PdfOptions {
            title: if opts.title.trim().is_empty() {
                "凭证整理单".into()
            } else {
                opts.title.clone()
            },
            card_style: opts.card_style.clone(),
            columns: opts.columns,
            include_images: opts.include_images,
        },
    )?;

    let count = items.len() as i64;
    let final_batch_no = batch_no.clone();
    ctx.db.with_tx(|tx| {
        let batch = export_repo::insert_batch(
            tx,
            &crate::models::NewExportBatch {
                batch_no: final_batch_no,
                voucher_count: count,
                pdf_path: pdf_path.to_string_lossy().to_string(),
                status: crate::models::ExportStatus::Done,
            },
        )?;
        for (i, (v, _)) in items.iter().enumerate() {
            export_repo::insert_item(tx, batch.id, v.id, i as i64)?;
            // 已归档的保持归档态，其余标记为已导出（先落快照，与其他写操作口径一致）
            if v.status != VoucherStatus::Archived {
                snapshot_repo::insert(
                    tx,
                    &crate::models::snapshot::NewSnapshot {
                        voucher_id: v.id,
                        payload: serde_json::to_value(v)?,
                        reason: "export".into(),
                    },
                )?;
                voucher_repo::set_status(tx, v.id, VoucherStatus::Exported)?;
            }
        }
        Ok(batch)
    })
}

pub fn list_batches(ctx: &CoreContext, limit: u32) -> Result<Vec<ExportBatch>> {
    let limit = if limit == 0 { 50 } else { limit };
    ctx.db
        .with_conn(|conn| export_repo::list_batches(conn, limit))
}
