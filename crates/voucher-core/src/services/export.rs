//! 导出批次服务（对应 v4.3 的 `services/export.py`）
//!
//! 两种导出模式：
//! - 合并单份：全部凭证进一份 PDF（卡片网格 + 图片附录）
//! - 按天分组（一图一凭证的默认形态）：按凭证日期分组，每天一份「凭证日报」PDF，
//!   每天各落一条批次记录。
//!
//! 两种模式的状态流转、快照口径一致：非归档凭证 → 已导出（先落快照）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::db::repos::{attachment_repo, export_repo, snapshot_repo, voucher_repo};
use crate::models::{Attachment, ExportBatch, Voucher, VoucherStatus};
use crate::services::pdf::{self, DailyInput, PdfCard, PdfImage, PdfInput, PdfOptions};
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

/// 按默认打印配置导出（合并单份）
pub fn export_with_defaults(ctx: &CoreContext, voucher_ids: &[i64]) -> Result<ExportBatch> {
    let opts = ExportOptions::from_print_config(&ctx.config().print);
    export_vouchers_pdf(ctx, voucher_ids, &opts)
}

/// 按默认打印配置导出（按天分组，每天一份日报 PDF）
pub fn export_daily_with_defaults(
    ctx: &CoreContext,
    voucher_ids: &[i64],
) -> Result<Vec<ExportBatch>> {
    let opts = ExportOptions::from_print_config(&ctx.config().print);
    export_vouchers_daily(ctx, voucher_ids, &opts)
}

pub fn export_vouchers_pdf(
    ctx: &CoreContext,
    voucher_ids: &[i64],
    opts: &ExportOptions,
) -> Result<ExportBatch> {
    if voucher_ids.is_empty() {
        return Err(CoreError::Invalid("未选择要导出的凭证".into()));
    }

    let items = fetch_items(ctx, voucher_ids)?;

    let input = PdfInput {
        cards: items.iter().map(|(v, _)| card_of(v)).collect(),
        images: images_of(ctx, &items),
    };

    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let batch_no = next_batch_no(ctx, &format!("EXP-{today}"))?;
    let pdf_path = ctx.paths.exports.join(format!("{batch_no}.pdf"));
    pdf::generate(&pdf_path, &input, &pdf_options(opts))?;

    record_batch(
        ctx,
        &items,
        batch_no,
        pdf_path.to_string_lossy().to_string(),
    )
}

/// 按天分组导出：每个日期一份「凭证日报」PDF + 各自一条批次记录。
/// 返回值按日期升序（旧日期在前，便于按日归档）。
pub fn export_vouchers_daily(
    ctx: &CoreContext,
    voucher_ids: &[i64],
    opts: &ExportOptions,
) -> Result<Vec<ExportBatch>> {
    if voucher_ids.is_empty() {
        return Err(CoreError::Invalid("未选择要导出的凭证".into()));
    }

    let items = fetch_items(ctx, voucher_ids)?;

    // 按凭证日期分组（BTreeMap：日期升序且去重）
    let mut by_day: BTreeMap<String, Vec<(Voucher, Vec<Attachment>)>> = BTreeMap::new();
    for (v, atts) in items {
        by_day
            .entry(v.voucher_date.clone())
            .or_default()
            .push((v, atts));
    }

    let mut batches = Vec::new();
    for (date, day_items) in by_day {
        let input = DailyInput {
            date: date.clone(),
            cards: day_items.iter().map(|(v, _)| card_of(v)).collect(),
            images: daily_images_of(ctx, &day_items),
        };

        let compact_date = date.replace(['-', '/'], "");
        // 日报批次号带 D 标识：EXP-D20260820-153012（与合并单份 EXP-20260820-153012 区分）
        let batch_no = next_batch_no(ctx, &format!("EXP-D{compact_date}"))?;
        let pdf_path = ctx.paths.exports.join(format!("{batch_no}.pdf"));
        pdf::generate_daily(&pdf_path, &input, &pdf_options(opts))?;

        batches.push(record_batch(
            ctx,
            &day_items,
            batch_no,
            pdf_path.to_string_lossy().to_string(),
        )?);
    }
    Ok(batches)
}

pub fn list_batches(ctx: &CoreContext, limit: u32) -> Result<Vec<ExportBatch>> {
    let limit = if limit == 0 { 50 } else { limit };
    ctx.db
        .with_conn(|conn| export_repo::list_batches(conn, limit))
}

// ---------- 内部辅助 ----------

/// 取凭证与附件（按日期+编号排序，保持打印顺序稳定）
fn fetch_items(ctx: &CoreContext, voucher_ids: &[i64]) -> Result<Vec<(Voucher, Vec<Attachment>)>> {
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
    Ok(items)
}

fn card_of(v: &Voucher) -> PdfCard {
    PdfCard {
        voucher_no: v.voucher_no.clone(),
        voucher_date: v.voucher_date.clone(),
        description: v.description.clone(),
        counterparty: v.counterparty.clone(),
        amount_label: v.amount_label(),
        amount_cents: v.amount_cents,
        tags_line: v.tags.join(" · "),
    }
}

fn images_of(ctx: &CoreContext, items: &[(Voucher, Vec<Attachment>)]) -> Vec<PdfImage> {
    items
        .iter()
        .flat_map(|(v, atts)| {
            atts.iter().filter(|a| a.is_image()).map(move |a| PdfImage {
                path: ctx.paths.attachment_abs(&a.file_path),
                caption: format!("{} {}", v.voucher_no, a.file_name),
            })
        })
        .collect()
}

/// 按天日报的图片条目：caption 对齐参照档案版式「附件N：摘要_对方_金额元（档号）」，
/// N 为当日序号（1 起）。空字段段跳过。
fn daily_images_of(ctx: &CoreContext, items: &[(Voucher, Vec<Attachment>)]) -> Vec<PdfImage> {
    items
        .iter()
        .enumerate()
        .flat_map(|(idx, (v, atts))| {
            atts.iter().filter(move |a| a.is_image()).map(move |a| {
                let mut label = format!("附件{}：{}", idx + 1, v.description);
                if !v.counterparty.is_empty() {
                    label.push('_');
                    label.push_str(&v.counterparty);
                }
                if let Some(cents) = v.amount_cents {
                    label.push('_');
                    label.push_str(&format!(
                        "{}元",
                        crate::models::voucher::format_money(cents)
                    ));
                }
                label.push_str(&format!("（{}）", v.voucher_no));
                PdfImage {
                    path: ctx.paths.attachment_abs(&a.file_path),
                    caption: label,
                }
            })
        })
        .collect()
}

fn pdf_options(opts: &ExportOptions) -> PdfOptions {
    PdfOptions {
        title: if opts.title.trim().is_empty() {
            "凭证整理单".into()
        } else {
            opts.title.clone()
        },
        card_style: opts.card_style.clone(),
        columns: opts.columns,
        include_images: opts.include_images,
    }
}

/// 批次号：{prefix}-HHMMSS（冲突时追加序号；prefix 已含 EXP- 与日期段）
fn next_batch_no(ctx: &CoreContext, prefix: &str) -> Result<String> {
    let stamp = chrono::Local::now().format("%H%M%S").to_string();
    let mut batch_no = format!("{prefix}-{stamp}");
    ctx.db.with_conn(|conn| {
        let mut n = 2;
        while export_repo::batch_no_exists(conn, &batch_no)? {
            batch_no = format!("{prefix}-{stamp}-{n}");
            n += 1;
        }
        Ok(())
    })?;
    Ok(batch_no)
}

/// 落批次记录 + 明细 + 状态流转（非归档 → 已导出，先落快照）
fn record_batch(
    ctx: &CoreContext,
    items: &[(Voucher, Vec<Attachment>)],
    batch_no: String,
    pdf_path: String,
) -> Result<ExportBatch> {
    let count = items.len() as i64;
    ctx.db.with_tx(|tx| {
        let batch = export_repo::insert_batch(
            tx,
            &crate::models::NewExportBatch {
                batch_no,
                voucher_count: count,
                pdf_path,
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
