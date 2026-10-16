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

use crate::db::repos::{
    attachment_repo, enhance_hit_repo, export_repo, snapshot_repo, voucher_repo,
};
use crate::models::{Attachment, EnhanceSource, ExportBatch, ExportFormat, Voucher, VoucherStatus};
use crate::services::pdf::{self, DailyInput, PdfCard, PdfImage, PdfInput, PdfOptions};
use crate::{CoreContext, CoreError, Result};

/// D-6：PDF 附录取图规则——扫描件/拍照件优先增强版（5 步矫正后更清晰可印）；
/// 截图/未识别回退原图（截图增强版仅用于 OCR，不进 PDF）。
/// 增强版文件缺失时同样回退原图，保证导出不中断（与 OCR 回退口径一致）。
/// v0.8：`print.prefer_enhanced_images = false` 时一律用原图（增强版异常时的兜底开关）。
pub fn pdf_image_abs(ctx: &CoreContext, a: &Attachment) -> std::path::PathBuf {
    if ctx.config().print.prefer_enhanced_images && a.enhanced_source == EnhanceSource::Scan {
        if let Some(ep) = &a.enhanced_path {
            // DB 来源 rel 走词法校验（Q-20）；非法路径视同增强版缺失回退原图
            if let Ok(abs) = ctx.paths.attachment_abs_checked(ep) {
                if abs.exists() {
                    return abs;
                }
                log::warn!("增强版文件缺失，PDF 回退原图: {}", abs.display());
            }
        }
    }
    ctx.paths.attachment_abs(&a.file_path)
}

/// D-6：该附件的增强版是否实际进了 PDF（与 pdf_image_abs 同口径）
pub fn uses_enhanced_in_pdf(ctx: &CoreContext, a: &Attachment) -> bool {
    ctx.config().print.prefer_enhanced_images
        && a.enhanced_source == EnhanceSource::Scan
        && a.enhanced_path
            .as_ref()
            .and_then(|ep| ctx.paths.attachment_abs_checked(ep).ok())
            .map(|abs| abs.is_file())
            .unwrap_or(false)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExportOptions {
    pub title: String,
    /// standard | clean | ledger
    pub card_style: String,
    /// 1 | 2 | 3
    pub columns: u8,
    pub include_images: bool,
    /// v0.8 日报模板：compact（省纸）| archive（档案装订）
    pub daily_template: String,
    /// v0.22 批次③：日报是否附带「相关凭证清单」页。
    /// **用户定的默认是"带"**，所以字段级 serde 默认也是 true（老前端不带这个键 = 带）。
    #[serde(default = "include_links_default")]
    pub include_links: bool,
}

/// 字段级默认：相关凭证清单**默认开**（用户选择）
fn include_links_default() -> bool {
    true
}

impl Default for ExportOptions {
    fn default() -> Self {
        // 与历史 `#[derive(Default)]` 的取值保持一致，只把 include_links 定为 true
        Self {
            title: String::new(),
            card_style: String::new(),
            columns: 0,
            include_images: false,
            daily_template: String::new(),
            include_links: true,
        }
    }
}

impl ExportOptions {
    fn from_print_config(cfg: &crate::utils::config::PrintConfig) -> Self {
        Self {
            title: cfg.title.clone(),
            card_style: cfg.card_style.clone(),
            columns: cfg.columns,
            include_images: cfg.include_images,
            daily_template: cfg.daily_template.clone(),
            // 打印默认带佐证（用户选择）；单次导出可在打印中心关掉
            include_links: true,
        }
    }
}

/// v0.22 批次③：把「相关凭证清单」另存一份 CSV（Excel 可核；UTF-8 BOM，与 reconcile 报告同一套写法）。
///
/// 只读产物：**不落 `export_batches` / `export_items`**，所以批次口径与"从未导出"台账都不受影响。
/// 最后一列显式写「是否并入合计」，把"关联 ≠ 抵消"印在文件里。
fn write_link_list_csv(
    ctx: &CoreContext,
    batch_no: &str,
    rows: &[pdf::PdfAppendixRow],
) -> Result<std::path::PathBuf> {
    use std::io::Write;
    if rows.is_empty() {
        return Err(CoreError::Invalid("没有相关凭证可导出".into()));
    }
    let path = ctx.paths.exports.join(format!("{batch_no}-关联清单.csv"));
    let mut out = String::new();
    out.push('\u{FEFF}');
    out.push_str("主凭证,关系类型,相关凭证,说明,参考金额（元）,是否并入合计\n");
    for r in rows {
        let amount = match r.amount_cents {
            Some(c) => format!("{}.{:02}", c / 100, c % 100),
            None => String::new(),
        };
        push_csv_row(
            &mut out,
            &[
                r.main_no.clone(),
                r.kind_label.clone(),
                r.other_no.clone(),
                r.summary.clone(),
                amount,
                "否（仅参考）".into(),
            ],
        );
    }
    let mut f = std::fs::File::create(&path)?;
    f.write_all(out.as_bytes())?;
    Ok(path)
}

/// CSV 单元格转义（含逗号/引号/换行才加引号；内部引号翻倍）
fn push_csv_row(out: &mut String, cells: &[String]) {
    let line = cells
        .iter()
        .map(|c| {
            if c.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", c.replace('"', "\"\""))
            } else {
                c.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    out.push_str(&line);
    out.push('\n');
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
    // 先写临时名再提交（fsync + rename）：生成中途失败不会在最终名留下半截 PDF
    // （旧行为直接写最终名，坏文件留在导出目录里且没有批次记录）。审计 B11。
    let tmp = pdf_path.with_extension("pdf.tmp");
    pdf::generate(&tmp, &input, &pdf_options(opts))?;
    crate::utils::jsonio::commit_file(&tmp, &pdf_path)?;

    record_batch(
        ctx,
        &items,
        batch_no,
        pdf_path.to_string_lossy().to_string(),
        ExportFormat::Pdf,
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
            // v0.22 批次③：默认带佐证 —— 相关凭证清单页（不写 export_items，批次张数不变）
            appendices: if opts.include_links {
                crate::services::links::appendix_rows(
                    ctx,
                    &day_items.iter().map(|(v, _)| v.id).collect::<Vec<_>>(),
                )?
            } else {
                Vec::new()
            },
        };

        let compact_date = date.replace(['-', '/'], "");
        // 日报批次号带 D 标识：EXP-D20260820-153012（与合并单份 EXP-20260820-153012 区分）
        let batch_no = next_batch_no(ctx, &format!("EXP-D{compact_date}"))?;
        let pdf_path = ctx.paths.exports.join(format!("{batch_no}.pdf"));
        // 同 export_vouchers：临时名 + 提交（审计 B11）
        let tmp = pdf_path.with_extension("pdf.tmp");
        pdf::generate_daily(&tmp, &input, &pdf_options(opts))?;
        crate::utils::jsonio::commit_file(&tmp, &pdf_path)?;

        // 相关凭证清单另存一份 CSV（Excel 可核；不落批次表）
        if !input.appendices.is_empty() {
            let csv = write_link_list_csv(ctx, &batch_no, &input.appendices)?;
            log::info!("相关凭证清单 CSV 已写入：{}", csv.display());
        }

        batches.push(record_batch(
            ctx,
            &day_items,
            batch_no,
            pdf_path.to_string_lossy().to_string(),
            ExportFormat::Pdf,
        )?);
    }
    Ok(batches)
}

pub fn list_batches(ctx: &CoreContext, limit: u32) -> Result<Vec<ExportBatch>> {
    let limit = if limit == 0 { 50 } else { limit };
    ctx.db
        .with_conn(|conn| export_repo::list_batches(conn, limit))
}

/// v0.10：表格式导出（Excel / CSV）。选单逻辑与 PDF 一致（同批次表、同状态流转），
/// 批次号前缀 EXP-X（xlsx）/ EXP-C（csv）与 PDF 批次区分。
pub fn export_vouchers_table(
    ctx: &CoreContext,
    voucher_ids: &[i64],
    format: ExportFormat,
) -> Result<ExportBatch> {
    if voucher_ids.is_empty() {
        return Err(CoreError::Invalid("未选择要导出的凭证".into()));
    }
    if format == ExportFormat::Pdf {
        return Err(CoreError::Invalid(
            "表格式导出仅支持 xlsx / csv（PDF 请走 export_pdf）".into(),
        ));
    }

    let items = fetch_items(ctx, voucher_ids)?;
    let vouchers: Vec<Voucher> = items.iter().map(|(v, _)| v.clone()).collect();

    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let prefix = if format == ExportFormat::Xlsx {
        format!("EXP-X{today}")
    } else {
        format!("EXP-C{today}")
    };
    let batch_no = next_batch_no(ctx, &prefix)?;
    let path = ctx
        .paths
        .exports
        .join(format!("{batch_no}.{}", format.ext()));
    if format == ExportFormat::Xlsx {
        crate::services::table::write_xlsx(&path, &vouchers)?;
    } else {
        crate::services::table::write_csv(&path, &vouchers)?;
    }

    record_batch(
        ctx,
        &items,
        batch_no,
        path.to_string_lossy().to_string(),
        format,
    )
}

/// 批次包含的凭证（导出历史反查：批次 → 凭证明细）。按导出时的排序输出；
/// 批次后凭证若已删除则跳过（不报错——历史批次允许凭证先于批次消亡）。
pub fn batch_vouchers(ctx: &CoreContext, batch_id: i64) -> Result<Vec<Voucher>> {
    ctx.db.with_conn(|conn| {
        let items = export_repo::list_items(conn, batch_id)?;
        let ids: Vec<i64> = items.iter().map(|i| i.voucher_id).collect();
        voucher_repo::get_many(conn, &ids)
    })
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
                path: pdf_image_abs(ctx, a),
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
                    path: pdf_image_abs(ctx, a),
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
        daily_template: opts.daily_template.clone(),
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
/// D-5：同步记录每张被导出图附件的 `enhance_hits` 命中（source='export'），
/// 让设置页「累计命中」/「活跃占比」指标能反映导出的实际使用。
/// - 优先 enhanced_path 走增强版（与 OCR/查看器口径一致）；
/// - 写命中走新连接，错误仅 warn 不中断批次（PDF 已生成不撤）。
/// - v0.10：`format` 区分批次格式；仅 PDF 记录增强命中（表格式导出不涉及图片）。
fn record_batch(
    ctx: &CoreContext,
    items: &[(Voucher, Vec<Attachment>)],
    batch_no: String,
    file_path: String,
    format: ExportFormat,
) -> Result<ExportBatch> {
    let count = items.len() as i64;
    let batch = ctx.db.with_tx(|tx| {
        let batch = export_repo::insert_batch(
            tx,
            &crate::models::NewExportBatch {
                batch_no,
                voucher_count: count,
                pdf_path: file_path,
                status: crate::models::ExportStatus::Done,
                format,
            },
        )?;
        for (i, (v, _atts)) in items.iter().enumerate() {
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
    })?;

    // D-5 增强版命中埋点（D-6 收紧口径）：只有增强版实际进了 PDF 的附件
    // （扫描件 + enhanced_path 存在 + 文件在盘）才算一次 Export 命中，
    // 与 pdf_image_abs 的取图规则完全一致，避免命中率虚高。
    if format == ExportFormat::Pdf {
        for (_v, atts) in items {
            for a in atts
                .iter()
                .filter(|a| a.is_image() && uses_enhanced_in_pdf(ctx, a))
            {
                if let Err(e) = ctx.db.with_conn(|conn| {
                    enhance_hit_repo::record(conn, a.id, enhance_hit_repo::HitSource::Export)
                }) {
                    log::warn!("记录 enhance_hits(Export) 失败（不影响导出）: {e}");
                }
            }
        }
    }

    Ok(batch)
}
