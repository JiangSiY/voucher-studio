//! 汇总册（v0.26 M3）：把若干张凭证打成一份可打印的 PDF。
//!
//! 三种册子对应用户的三个真实场景（PRD ⑤）：
//! - 图片册：微信收来的小票/回单，打印出来能一张张对着原图核账；
//! - 发票册：增值税专票/普票单独抽出来，字段在上、票面在下，给会计贴票用；
//! - PDF 合并：合同/回单本身就是 PDF，按顺序并成一份 + 末尾清单。
//!
//! 落盘位置：`exports/packs/`。册子是「导出产物」，不进 `export_batches`
//! （那是「凭证状态被标记为已导出」的凭证级批次，册子不改变任何凭证状态）。

use std::path::PathBuf;

use chrono::Local;
use rusqlite::Connection;

use crate::db::repos::attachment_repo;
use crate::models::attachment::Attachment;
use crate::models::biz::BizFields;
use crate::models::Voucher;
use crate::services::{biz, pdf};
use crate::{CoreContext, CoreError, Result};

/// 册子类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackKind {
    ImageBook,
    InvoiceBook,
    PdfMerge,
}

impl PackKind {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "image" | "image_book" | "imagebook" => Ok(Self::ImageBook),
            "invoice" | "invoice_book" | "invoicebook" => Ok(Self::InvoiceBook),
            "pdf" | "pdf_merge" | "merge" => Ok(Self::PdfMerge),
            other => Err(CoreError::Invalid(format!(
                "未知的册子类型「{other}」（可用：image / invoice / pdf）"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ImageBook => "image",
            Self::InvoiceBook => "invoice",
            Self::PdfMerge => "pdf",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::ImageBook => "原始凭证图片册",
            Self::InvoiceBook => "发票册（打印版）",
            Self::PdfMerge => "PDF 合并册",
        }
    }
}

/// 生成结果
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackResult {
    pub path: String,
    pub kind: String,
    pub vouchers: usize,
    /// 没有可用附件的凭证数（跳过，不报错——一张没图的凭证不该让整册失败）
    pub skipped: usize,
    pub pages: usize,
}

fn stamp() -> String {
    Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn money(cents: Option<i64>) -> String {
    match cents {
        Some(c) => format!("￥{}.{:02}", c / 100, c.abs() % 100),
        None => "￥—".into(),
    }
}

fn load_vouchers(ctx: &CoreContext, ids: &[i64]) -> Result<Vec<Voucher>> {
    if ids.is_empty() {
        return Err(CoreError::Invalid("没有选中凭证".into()));
    }
    ctx.db.with_conn(|c| {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let v = crate::db::repos::voucher_repo::get(c, *id)?
                .ok_or_else(|| CoreError::NotFound(format!("凭证 {id} 不存在")))?;
            out.push(v);
        }
        Ok(out)
    })
}

/// 主附件的磁盘路径：优先增强版（旋转/裁切后的），缺失则回退原图
fn primary_file(ctx: &CoreContext, a: &Attachment) -> Option<PathBuf> {
    if let Some(rel) = &a.enhanced_path {
        if let Ok(p) = ctx.paths.attachment_abs_checked(rel) {
            if p.is_file() {
                return Some(p);
            }
        }
    }
    ctx.paths
        .attachment_abs_checked(&a.file_path)
        .ok()
        .filter(|p| p.is_file())
}

fn attachments_of(ctx: &CoreContext, voucher_id: i64) -> Result<Vec<Attachment>> {
    ctx.db
        .with_conn(|c| attachment_repo::list_by_voucher(c, voucher_id))
}

fn biz_of(ctx: &CoreContext, voucher_id: i64) -> Result<BizFields> {
    ctx.db.with_conn(|c: &Connection| biz::load(c, voucher_id))
}

/// 生成册子。`voucher_ids` 的顺序就是册子里的顺序（调用方拖拽排序说了算）。
pub fn build(ctx: &CoreContext, voucher_ids: &[i64], kind: PackKind) -> Result<PackResult> {
    let vouchers = load_vouchers(ctx, voucher_ids)?;
    let dir = ctx.paths.exports.join("packs");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}_{}.pdf", kind.as_str(), stamp()));

    match kind {
        PackKind::ImageBook => build_book(ctx, &vouchers, &path, kind, false),
        PackKind::InvoiceBook => build_book(ctx, &vouchers, &path, kind, true),
        PackKind::PdfMerge => build_merge(ctx, &vouchers, &path),
    }
}

fn build_book(
    ctx: &CoreContext,
    vouchers: &[Voucher],
    path: &std::path::Path,
    kind: PackKind,
    invoice_mode: bool,
) -> Result<PackResult> {
    let mut items: Vec<pdf::BundleItem> = Vec::new();
    let mut skipped = 0usize;
    for v in vouchers {
        let atts = attachments_of(ctx, v.id)?;
        let b = biz_of(ctx, v.id)?;
        if invoice_mode {
            // 发票册只收「票据类型非空」的凭证：没有票据类型的说明不是发票
            if b.ticket_type.trim().is_empty() {
                skipped += 1;
                continue;
            }
        }
        let mut image = None;
        for a in &atts {
            if a.is_pdf {
                continue; // PDF 附件单独走合并册，贴成图片会丢页
            }
            if let Some(p) = primary_file(ctx, a) {
                image = Some(pdf::PdfImage {
                    path: p,
                    caption: v.voucher_no.clone(),
                });
                break;
            }
        }
        if image.is_none() {
            skipped += 1;
            continue;
        }
        let caption = if invoice_mode && !b.invoice_no.trim().is_empty() {
            format!("{} 发票 {} {}", v.voucher_date, b.ticket_type, b.invoice_no)
        } else {
            format!("{} {}", v.voucher_date, v.counterparty)
        };
        let mut lines = vec![
            format!("凭证号 {}", v.voucher_no),
            format!("日期 {} · 金额 {}", v.voucher_date, money(v.amount_cents)),
        ];
        if !v.description.trim().is_empty() {
            lines.push(format!("摘要 {}", v.description));
        }
        if invoice_mode {
            lines.push(format!(
                "票据 {} · 发票号 {}",
                b.ticket_type,
                blank_or(&b.invoice_no, "—")
            ));
            if !b.project_name.trim().is_empty() {
                lines.push(format!("项目 {}", b.project_name));
            }
        }
        items.push(pdf::BundleItem {
            caption,
            lines,
            image,
        });
    }
    if items.is_empty() {
        return Err(CoreError::Invalid("选中的凭证里没有可用的图片附件".into()));
    }
    pdf::generate_bundle(path, kind.title(), &items)?;
    // 一页一张 + 清单页（28 条/页）
    let list_pages = items.len().div_ceil(28);
    Ok(PackResult {
        path: path.to_string_lossy().into_owned(),
        kind: kind.as_str().into(),
        vouchers: items.len(),
        skipped,
        pages: items.len() + list_pages,
    })
}

fn build_merge(
    ctx: &CoreContext,
    vouchers: &[Voucher],
    path: &std::path::Path,
) -> Result<PackResult> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut list: Vec<pdf::BundleItem> = Vec::new();
    let mut skipped = 0usize;
    for v in vouchers {
        let atts = attachments_of(ctx, v.id)?;
        let mut found = false;
        for a in &atts {
            if !a.is_pdf {
                continue;
            }
            if let Some(p) = primary_file(ctx, a) {
                files.push(p);
                found = true;
            }
        }
        if !found {
            skipped += 1;
            continue;
        }
        list.push(pdf::BundleItem {
            caption: format!("{} {}", v.voucher_no, v.counterparty),
            lines: vec![format!(
                "{} · {} · 附件 {}",
                v.voucher_date,
                money(v.amount_cents),
                atts.iter().filter(|a| a.is_pdf).count()
            )],
            image: None,
        });
    }
    if files.is_empty() {
        return Err(CoreError::Invalid("选中的凭证里没有 PDF 附件".into()));
    }

    // 先单独生成「清单」PDF，再与原件合并：这样清单页和原件是同一种产物，页序可控
    let list_path = path.with_file_name(format!(
        "{}-list.pdf",
        path.file_stem().unwrap_or_default().to_string_lossy()
    ));
    pdf::generate_bundle(&list_path, PackKind::PdfMerge.title(), &list)?;

    let mut all = files.clone();
    all.push(list_path.clone());
    let pages = pdf::merge_pdfs(path, &all)?;
    let _ = std::fs::remove_file(&list_path);

    Ok(PackResult {
        path: path.to_string_lossy().into_owned(),
        kind: PackKind::PdfMerge.as_str().into(),
        vouchers: list.len(),
        skipped,
        pages,
    })
}

fn blank_or<'a>(s: &'a str, fallback: &'a str) -> &'a str {
    if s.trim().is_empty() {
        fallback
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_kind_parses_aliases() {
        assert_eq!(PackKind::parse("image").unwrap(), PackKind::ImageBook);
        assert_eq!(PackKind::parse("pdf_merge").unwrap(), PackKind::PdfMerge);
        assert!(PackKind::parse("nope").is_err());
    }

    #[test]
    fn money_formats_cents() {
        assert_eq!(money(Some(12_850)), "￥128.50");
        assert_eq!(money(None), "￥—");
    }

    #[test]
    fn bundle_and_merge_count_pages() {
        // 只测版式与合并的机械正确性：两条各占一页 + 一页清单 = 3 页；两份合并 = 6 页。
        // 不测图片（那依赖真实附件与字体环境，桌面端在真机走查里覆盖）。
        let dir = std::env::temp_dir().join(format!("vs-pack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.pdf");
        let b = dir.join("b.pdf");
        let items: Vec<pdf::BundleItem> = (1..=2)
            .map(|i| pdf::BundleItem {
                caption: format!("测试 {i}"),
                lines: vec![format!("第 {i} 条")],
                image: None,
            })
            .collect();
        pdf::generate_bundle(&a, "测试册", &items).unwrap();
        pdf::generate_bundle(&b, "测试册", &items).unwrap();

        let doc = lopdf::Document::load(&a).unwrap();
        assert_eq!(doc.get_pages().len(), 3);

        let out = dir.join("merged.pdf");
        let pages = pdf::merge_pdfs(&out, &[a, b]).unwrap();
        assert_eq!(pages, 6);
        let merged = lopdf::Document::load(&out).unwrap();
        assert_eq!(merged.get_pages().len(), 6);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
