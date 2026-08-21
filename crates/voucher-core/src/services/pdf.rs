//! PDF 生成（对应 v4.3 的 `pdf/generator.py` + `pdf/fonts.py`）
//!
//! 布局语义对齐 UI 方案：A4 纵向、列数 1/2/3 可选（默认 3 列 × 7 行 = 21 张/页）、
//! 卡片以凭证编号为核心、文字为主省墨设计、可选附件图片页（2×2/页）。

use std::io::BufWriter;
use std::path::{Path, PathBuf};

use printpdf::path::{PaintMode, WindingOrder};
use printpdf::{
    Color, Image, ImageTransform, IndirectFontRef, Line, Mm, PdfDocument, PdfLayerReference, Point,
    Polygon, Rgb,
};
use serde::Serialize;

use crate::{CoreError, Result};

/// 打印选项（与 PrintConfig 字段一致）
#[derive(Debug, Clone, Serialize)]
pub struct PdfOptions {
    pub title: String,
    /// standard | clean | ledger
    pub card_style: String,
    /// 1 | 2 | 3
    pub columns: u8,
    pub include_images: bool,
}

/// 凭证卡片数据
#[derive(Debug, Clone)]
pub struct PdfCard {
    pub voucher_no: String,
    pub voucher_date: String,
    pub description: String,
    /// 对方账户（按天日报表格列）
    pub counterparty: String,
    pub amount_label: String,
    /// 金额（分；日报合计用）
    pub amount_cents: Option<i64>,
    pub tags_line: String,
}

/// 图片页条目
#[derive(Debug, Clone)]
pub struct PdfImage {
    pub path: PathBuf,
    pub caption: String,
}

#[derive(Debug, Clone, Default)]
pub struct PdfInput {
    pub cards: Vec<PdfCard>,
    pub images: Vec<PdfImage>,
}

/// 按天日报输入：某一天的凭证集合（一图一凭证模式下即当日全部图片凭证）
#[derive(Debug, Clone)]
pub struct DailyInput {
    /// YYYY-MM-DD
    pub date: String,
    pub cards: Vec<PdfCard>,
    pub images: Vec<PdfImage>,
}

// ---------- 页面几何（mm，f32：printpdf 的单位） ----------
const PAGE_W: f32 = 210.0;
const PAGE_H: f32 = 297.0;
const MARGIN: f32 = 10.0;
const HEADER_H: f32 = 16.0;
const FOOTER_H: f32 = 8.0;
const GAP: f32 = 3.0;
const ROWS_PER_PAGE: usize = 7;
const IMG_COLS: usize = 2;
const IMG_ROWS: usize = 2;

/// 从页顶往下数的 y 坐标（PDF 原点在左下角，本模块统一「从页顶往下」思考）
fn y(top: f32) -> Mm {
    Mm(PAGE_H - top)
}

fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color::Rgb(Rgb::new(r, g, b, None))
}

const STROKE: (f32, f32, f32) = (0.72, 0.72, 0.72);
const DARK: (f32, f32, f32) = (0.12, 0.12, 0.12);
const ACCENT: (f32, f32, f32) = (0.0, 0.47, 0.83);
const BODY: (f32, f32, f32) = (0.25, 0.25, 0.25);
const GREY: (f32, f32, f32) = (0.5, 0.5, 0.5);

/// 生成 PDF
pub fn generate(path: &Path, input: &PdfInput, opts: &PdfOptions) -> Result<()> {
    if input.cards.is_empty() {
        return Err(CoreError::Invalid("没有可导出的凭证".into()));
    }

    let title = if opts.title.trim().is_empty() {
        "凭证整理单".to_string()
    } else {
        opts.title.clone()
    };
    let columns: usize = match opts.columns {
        1..=3 => opts.columns as usize,
        _ => 3,
    };

    let (doc, first_page, first_layer) =
        PdfDocument::new(&title, Mm(PAGE_W), Mm(PAGE_H), "Content");
    let font = load_cjk_font(&doc)?;

    let usable_w = PAGE_W - MARGIN * 2.0;
    let card_w = (usable_w - GAP * (columns as f32 - 1.0)) / columns as f32;
    let usable_h = PAGE_H - MARGIN * 2.0 - HEADER_H - FOOTER_H;
    let card_h = (usable_h - GAP * (ROWS_PER_PAGE - 1) as f32) / ROWS_PER_PAGE as f32;

    let per_page = columns * ROWS_PER_PAGE;
    let total_cards = input.cards.len();
    let card_pages = total_cards.div_ceil(per_page);

    for page_idx in 0..card_pages {
        let layer = if page_idx == 0 {
            doc.get_page(first_page).get_layer(first_layer)
        } else {
            let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Cards");
            doc.get_page(p).get_layer(l)
        };
        draw_header(&layer, &font, &title);

        for slot in 0..per_page {
            let card_idx = page_idx * per_page + slot;
            if card_idx >= total_cards {
                break;
            }
            let col = (slot % columns) as f32;
            let row = (slot / columns) as f32;
            let x = MARGIN + col * (card_w + GAP);
            let top = MARGIN + HEADER_H + row * (card_h + GAP);
            draw_card(
                &layer,
                &font,
                &input.cards[card_idx],
                x,
                top,
                card_w,
                card_h,
                &opts.card_style,
            );
        }
        draw_footer(&layer, &font, page_idx + 1, card_pages);
    }

    // ===== 图片附录页（每页 2×2） =====
    if opts.include_images && !input.images.is_empty() {
        let per_img_page = IMG_COLS * IMG_ROWS;
        let img_pages = input.images.len().div_ceil(per_img_page);
        for page_idx in 0..img_pages {
            let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Images");
            let layer = doc.get_page(p).get_layer(l);
            draw_header(&layer, &font, &format!("{title} · 附件"));

            let cell_w = (usable_w - GAP) / IMG_COLS as f32;
            let cell_h = (usable_h - GAP) / IMG_ROWS as f32;
            for slot in 0..per_img_page {
                let idx = page_idx * per_img_page + slot;
                if idx >= input.images.len() {
                    break;
                }
                let col = slot % IMG_COLS;
                let row = slot / IMG_ROWS;
                let x = MARGIN + col as f32 * (cell_w + GAP);
                let top = MARGIN + HEADER_H + row as f32 * (cell_h + GAP);
                let caption_h = 5.0;
                let img_area_h = cell_h - caption_h - 1.0;
                place_image(&layer, &input.images[idx], x, top, cell_w, img_area_h);
                layer.set_fill_color(rgb(ACCENT.0, ACCENT.1, ACCENT.2));
                layer.use_text(
                    &input.images[idx].caption,
                    7.0,
                    Mm(x + 1.0),
                    y(top + cell_h - 0.5),
                    &font,
                );
            }
            draw_footer(&layer, &font, page_idx + 1, img_pages);
        }
    }

    doc.save(&mut BufWriter::new(std::fs::File::create(path)?))?;
    Ok(())
}

// ---------- 按天日报版式常量（mm） ----------
/// 汇总表行高
const TROW_H: f32 = 6.2;
/// 表头行高
const THEAD_H: f32 = 5.5;
/// 标题与表头之间的摘要行区域（「当日共 N 张 · 合计 ¥X」）
const TSUM_GAP: f32 = 10.0;
/// 每页最多行数（含合计行预留）
const TROWS_PER_PAGE: usize = 34;
/// 列宽：编号 45 / 摘要 70 / 对方账户 50，金额右对齐至右页边
const TCOL_NO: f32 = 45.0;
const TCOL_DESC: f32 = 70.0;
const TCOL_PARTY: f32 = 50.0;

/// 生成「按天日报」PDF：当日汇总表（自动分页 + 合计行）+ 当日图片页（2×2/页）
pub fn generate_daily(path: &Path, day: &DailyInput, opts: &PdfOptions) -> Result<()> {
    if day.cards.is_empty() {
        return Err(CoreError::Invalid("当天没有可导出的凭证".into()));
    }
    let base_title = if opts.title.trim().is_empty() {
        "凭证日报".to_string()
    } else {
        opts.title.trim().to_string()
    };
    let full_title = format!("{base_title} · {}", day.date);

    let (doc, first_page, first_layer) =
        PdfDocument::new(&full_title, Mm(PAGE_W), Mm(PAGE_H), "Summary");
    let font = load_cjk_font(&doc)?;

    let usable_w = PAGE_W - MARGIN * 2.0;
    let usable_h = PAGE_H - MARGIN * 2.0 - HEADER_H - FOOTER_H;

    let count = day.cards.len();
    let total_cents: i64 = day.cards.iter().filter_map(|c| c.amount_cents).sum();
    let img_count = if opts.include_images {
        day.images.len()
    } else {
        0
    };
    let img_pages = img_count.div_ceil(IMG_COLS * IMG_ROWS);
    let table_pages = count.div_ceil(TROWS_PER_PAGE);
    let total_pages = table_pages + img_pages;

    // ===== 汇总表页 =====
    for t in 0..table_pages {
        let layer = if t == 0 {
            doc.get_page(first_page).get_layer(first_layer)
        } else {
            let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Summary");
            doc.get_page(p).get_layer(l)
        };
        draw_header(&layer, &font, &full_title);

        // 首页摘要行：当日共 N 张 · 合计 ¥X（仅统计已填金额的凭证）
        if t == 0 {
            layer.set_fill_color(rgb(DARK.0, DARK.1, DARK.2));
            layer.use_text(
                format!(
                    "当日共 {count} 张凭证 · 合计 ¥{}",
                    crate::models::voucher::format_money(total_cents)
                ),
                9.5,
                Mm(MARGIN),
                y(MARGIN + HEADER_H + 4.5),
                &font,
            );
        }

        // 表头（每页重复）
        let table_top = MARGIN + HEADER_H + TSUM_GAP;
        draw_table_head(&layer, &font, table_top);

        let start = t * TROWS_PER_PAGE;
        let end = (start + TROWS_PER_PAGE).min(count);
        for (i, card) in day.cards[start..end].iter().enumerate() {
            let top = table_top + THEAD_H + i as f32 * TROW_H;
            draw_table_row(&layer, &font, card, top);
        }

        // 合计行（最后一页表格底部）
        if t == table_pages - 1 {
            let top = table_top + THEAD_H + (end - start) as f32 * TROW_H + 1.0;
            layer.set_outline_thickness(0.8);
            layer.set_outline_color(rgb(DARK.0, DARK.1, DARK.2));
            layer.add_line(Line {
                points: vec![
                    (Point::new(Mm(MARGIN), y(top)), false),
                    (Point::new(Mm(PAGE_W - MARGIN), y(top)), false),
                ],
                is_closed: false,
            });
            layer.set_fill_color(rgb(DARK.0, DARK.1, DARK.2));
            layer.use_text("合计", 9.0, Mm(MARGIN + 1.0), y(top + 4.8), &font);
            let cnt_label = format!("{count} 张");
            layer.use_text(
                &cnt_label,
                9.0,
                Mm(right_x(&cnt_label, 9.0, PAGE_W - MARGIN - 2.0)),
                y(top + 4.8),
                &font,
            );
            let amt_label = format!("¥{}", crate::models::voucher::format_money(total_cents));
            layer.use_text(
                &amt_label,
                9.0,
                Mm(right_x(&amt_label, 9.0, PAGE_W - MARGIN)),
                y(top + 4.8),
                &font,
            );
        }

        draw_footer(&layer, &font, t + 1, total_pages);
    }

    // ===== 图片页（2×2/页，与合并版一致） =====
    for page_idx in 0..img_pages {
        let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Images");
        let layer = doc.get_page(p).get_layer(l);
        draw_header(&layer, &font, &format!("{full_title} · 附件"));

        let per_img_page = IMG_COLS * IMG_ROWS;
        let cell_w = (usable_w - GAP) / IMG_COLS as f32;
        let cell_h = (usable_h - GAP) / IMG_ROWS as f32;
        for slot in 0..per_img_page {
            let idx = page_idx * per_img_page + slot;
            if idx >= day.images.len() {
                break;
            }
            let col = slot % IMG_COLS;
            let row = slot / IMG_ROWS;
            let x = MARGIN + col as f32 * (cell_w + GAP);
            let top = MARGIN + HEADER_H + row as f32 * (cell_h + GAP);
            let caption_h = 5.0;
            let img_area_h = cell_h - caption_h - 1.0;
            place_image(&layer, &day.images[idx], x, top, cell_w, img_area_h);
            layer.set_fill_color(rgb(ACCENT.0, ACCENT.1, ACCENT.2));
            layer.use_text(
                &day.images[idx].caption,
                7.0,
                Mm(x + 1.0),
                y(top + cell_h - 0.5),
                &font,
            );
        }
        draw_footer(&layer, &font, table_pages + page_idx + 1, total_pages);
    }

    doc.save(&mut BufWriter::new(std::fs::File::create(path)?))?;
    Ok(())
}

fn draw_table_head(layer: &PdfLayerReference, font: &IndirectFontRef, top: f32) {
    layer.set_fill_color(rgb(GREY.0, GREY.1, GREY.2));
    layer.use_text("编号", 8.0, Mm(MARGIN + 1.0), y(top + 4.0), font);
    layer.use_text("摘要", 8.0, Mm(MARGIN + TCOL_NO + 1.0), y(top + 4.0), font);
    layer.use_text(
        "对方账户",
        8.0,
        Mm(MARGIN + TCOL_NO + TCOL_DESC + 1.0),
        y(top + 4.0),
        font,
    );
    layer.use_text(
        "金额",
        8.0,
        Mm(right_x("金额", 8.0, PAGE_W - MARGIN)),
        y(top + 4.0),
        font,
    );
    layer.set_outline_thickness(0.6);
    layer.set_outline_color(rgb(STROKE.0, STROKE.1, STROKE.2));
    layer.add_line(Line {
        points: vec![
            (Point::new(Mm(MARGIN), y(top + THEAD_H - 0.5)), false),
            (
                Point::new(Mm(PAGE_W - MARGIN), y(top + THEAD_H - 0.5)),
                false,
            ),
        ],
        is_closed: false,
    });
}

fn draw_table_row(layer: &PdfLayerReference, font: &IndirectFontRef, card: &PdfCard, top: f32) {
    let desc_x = MARGIN + TCOL_NO + 1.0;
    let party_x = MARGIN + TCOL_NO + TCOL_DESC + 1.0;

    // 编号（等宽感：主色）
    layer.set_fill_color(rgb(ACCENT.0, ACCENT.1, ACCENT.2));
    let no = truncate_chars(
        &card.voucher_no,
        chars_for_width(&card.voucher_no, 8.5, TCOL_NO - 2.0),
    );
    layer.use_text(no, 8.5, Mm(MARGIN + 1.0), y(top + 4.3), font);

    // 摘要（空则占位横线，保持行可读）
    layer.set_fill_color(rgb(BODY.0, BODY.1, BODY.2));
    let desc = if card.description.is_empty() {
        "—"
    } else {
        &card.description
    };
    let desc = truncate_chars(desc, chars_for_width(desc, 8.5, TCOL_DESC - 2.0));
    layer.use_text(desc, 8.5, Mm(desc_x), y(top + 4.3), font);

    // 对方账户（小灰字）
    layer.set_fill_color(rgb(GREY.0, GREY.1, GREY.2));
    let party = if card.counterparty.is_empty() {
        "—"
    } else {
        &card.counterparty
    };
    let party = truncate_chars(party, chars_for_width(party, 8.5, TCOL_PARTY - 2.0));
    layer.use_text(party, 8.5, Mm(party_x), y(top + 4.3), font);

    // 金额（右对齐，无金额留空）
    if !card.amount_label.is_empty() {
        layer.set_fill_color(rgb(DARK.0, DARK.1, DARK.2));
        layer.use_text(
            &card.amount_label,
            9.0,
            Mm(right_x(&card.amount_label, 9.0, PAGE_W - MARGIN)),
            y(top + 4.3),
            font,
        );
    }

    // 行底分隔线（省墨：仅横向细线，无竖线、无底色）
    layer.set_outline_thickness(0.25);
    layer.set_outline_color(rgb(STROKE.0, STROKE.1, STROKE.2));
    layer.add_line(Line {
        points: vec![
            (Point::new(Mm(MARGIN), y(top + TROW_H - 0.3)), false),
            (
                Point::new(Mm(PAGE_W - MARGIN), y(top + TROW_H - 0.3)),
                false,
            ),
        ],
        is_closed: false,
    });
}

/// 右对齐 x 坐标：右边界 - 文本估算宽度
fn right_x(s: &str, font_pt: f32, right_edge: f32) -> f32 {
    right_edge - text_width_mm(s, font_pt)
}

/// 文本宽度估算（与 chars_for_width 同一套 CJK/ASCII 宽度假设）
fn text_width_mm(s: &str, font_pt: f32) -> f32 {
    let char_mm = font_pt * 0.3528;
    let units: f32 = s
        .chars()
        .map(|c| if c.is_ascii() { 0.55 } else { 1.0 })
        .sum();
    units * char_mm
}

fn draw_header(layer: &PdfLayerReference, font: &IndirectFontRef, title: &str) {
    layer.set_fill_color(rgb(ACCENT.0, ACCENT.1, ACCENT.2));
    layer.use_text(title, 14.0, Mm(MARGIN), y(MARGIN + 8.0), font);
    // 标题下分隔线
    layer.set_outline_color(rgb(ACCENT.0, ACCENT.1, ACCENT.2));
    layer.set_outline_thickness(0.8);
    layer.add_line(Line {
        points: vec![
            (Point::new(Mm(MARGIN), y(MARGIN + 11.0)), false),
            (Point::new(Mm(PAGE_W - MARGIN), y(MARGIN + 11.0)), false),
        ],
        is_closed: false,
    });
}

fn draw_footer(layer: &PdfLayerReference, font: &IndirectFontRef, page: usize, total: usize) {
    layer.set_fill_color(rgb(GREY.0, GREY.1, GREY.2));
    layer.use_text(
        format!("第 {page} / {total} 页"),
        7.5,
        Mm(MARGIN),
        y(PAGE_H - MARGIN + 3.0),
        font,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_card(
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    card: &PdfCard,
    x: f32,
    top: f32,
    w: f32,
    h: f32,
    style: &str,
) {
    // 卡片边框（描边矩形）
    layer.set_outline_color(rgb(STROKE.0, STROKE.1, STROKE.2));
    layer.set_outline_thickness(0.5);
    layer.add_line(Line {
        points: vec![
            (Point::new(Mm(x), y(top)), false),
            (Point::new(Mm(x + w), y(top)), false),
            (Point::new(Mm(x + w), y(top + h)), false),
            (Point::new(Mm(x), y(top + h)), false),
        ],
        is_closed: true,
    });

    // 风格区分：clean 无顶部强调条；standard 用主色、ledger 用深色
    let bar_h = match style {
        "clean" => 0.0,
        _ => 2.2,
    };
    if bar_h > 0.0 {
        let bar = if style == "ledger" { DARK } else { ACCENT };
        layer.set_fill_color(rgb(bar.0, bar.1, bar.2));
        layer.add_polygon(Polygon {
            rings: vec![vec![
                (Point::new(Mm(x), y(top)), false),
                (Point::new(Mm(x + w), y(top)), false),
                (Point::new(Mm(x + w), y(top + bar_h)), false),
                (Point::new(Mm(x), y(top + bar_h)), false),
            ]],
            mode: PaintMode::Fill,
            winding_order: WindingOrder::NonZero,
        });
    }

    let pad = 2.5;
    let left = x + pad;
    let max_w = w - pad * 2.0;

    // 凭证编号：核心，等宽大字号
    layer.set_fill_color(rgb(DARK.0, DARK.1, DARK.2));
    let no_chars = chars_for_width(&card.voucher_no, 11.0, max_w);
    layer.use_text(
        truncate_chars(&card.voucher_no, no_chars),
        11.0,
        Mm(left),
        y(top + bar_h + 7.0),
        font,
    );

    // 日期 + 金额（同一行）
    let mut meta = card.voucher_date.clone();
    if !card.amount_label.is_empty() {
        meta.push_str("  ¥");
        meta.push_str(&card.amount_label);
    }
    layer.set_fill_color(rgb(ACCENT.0, ACCENT.1, ACCENT.2));
    let meta_chars = chars_for_width(&meta, 8.0, max_w);
    layer.use_text(
        truncate_chars(&meta, meta_chars),
        8.0,
        Mm(left),
        y(top + bar_h + 12.5),
        font,
    );

    // 摘要
    if !card.description.is_empty() {
        layer.set_fill_color(rgb(BODY.0, BODY.1, BODY.2));
        let d_chars = chars_for_width(&card.description, 8.0, max_w);
        layer.use_text(
            truncate_chars(&card.description, d_chars),
            8.0,
            Mm(left),
            y(top + bar_h + 17.5),
            font,
        );
    }

    // 标签行（小灰字）
    if !card.tags_line.is_empty() {
        layer.set_fill_color(rgb(GREY.0, GREY.1, GREY.2));
        let t_chars = chars_for_width(&card.tags_line, 6.5, max_w);
        layer.use_text(
            truncate_chars(&card.tags_line, t_chars),
            6.5,
            Mm(left),
            y(top + h - 3.0),
            font,
        );
    }
}

fn place_image(
    layer: &PdfLayerReference,
    img: &PdfImage,
    x: f32,
    top: f32,
    max_w: f32,
    max_h: f32,
) {
    let decoded = image::io::Reader::open(&img.path)
        .map_err(|e| e.to_string())
        .and_then(|r| r.decode().map_err(|e| e.to_string()));
    let dyn_img = match decoded {
        Ok(i) => i,
        Err(e) => {
            log::warn!("读取附件图片失败 {}: {e}", img.path.display());
            return;
        }
    };
    let (w_px, h_px) = (dyn_img.width() as f32, dyn_img.height() as f32);
    if w_px <= 0.0 || h_px <= 0.0 {
        return;
    }
    let image = Image::from_dynamic_image(&dyn_img);

    // printpdf 默认 300dpi：px * 25.4 / 300 = mm
    let default_w_mm = w_px * 25.4 / 300.0;
    let default_h_mm = h_px * 25.4 / 300.0;
    let ratio = (max_w / default_w_mm).min(max_h / default_h_mm);
    let w = default_w_mm * ratio;
    let h = default_h_mm * ratio;
    let dx = x + (max_w - w) / 2.0;
    let dy_top = top + (max_h - h) / 2.0;
    image.add_to_layer(
        layer.clone(),
        ImageTransform {
            translate_x: Some(Mm(dx)),
            translate_y: Some(Mm(PAGE_H - dy_top - h)),
            scale_x: Some(ratio),
            scale_y: Some(ratio),
            ..Default::default()
        },
    );
}

/// 按目标宽度估算可容纳的字符数（CJK 全宽 ≈ 字号 pt；ASCII ≈ 一半）
fn chars_for_width(s: &str, font_pt: f32, width_mm: f32) -> usize {
    let char_mm = font_pt * 0.3528;
    let width_chars = width_mm / char_mm;
    let units: f32 = s
        .chars()
        .map(|c| if c.is_ascii() { 0.55 } else { 1.0 })
        .sum();
    if units <= width_chars {
        return s.chars().count();
    }
    let mut acc = 0.0;
    let mut n = 0;
    for c in s.chars() {
        acc += if c.is_ascii() { 0.55 } else { 1.0 };
        if acc > width_chars - 1.0 {
            break;
        }
        n += 1;
    }
    n.max(1)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// 常见系统中文字体探测（对应 v4.3 `pdf/fonts.py` 的字体注册逻辑）
pub fn find_cjk_font() -> Option<PathBuf> {
    font_candidates().into_iter().find(|p| p.is_file())
}

/// 逐个候选尝试加载（TTC 集合格式可能解析失败，自动降级到下一个）
fn load_cjk_font(doc: &printpdf::PdfDocumentReference) -> Result<IndirectFontRef> {
    let mut candidates = font_candidates();
    if let Ok(p) = std::env::var("VOUCHER_PDF_FONT") {
        let path = PathBuf::from(p);
        if path.is_file() {
            candidates.insert(0, path);
        }
    }
    let mut last_err = String::from("未找到系统中文字体");
    for c in candidates {
        if !c.is_file() {
            continue;
        }
        match std::fs::File::open(&c) {
            Ok(file) => match doc.add_external_font(file) {
                Ok(f) => return Ok(f),
                Err(e) => {
                    log::info!("字体 {} 加载失败，尝试下一个: {e}", c.display());
                    last_err = format!("{}: {e}", c.display());
                }
            },
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(CoreError::Invalid(format!(
        "未找到可用的系统中文字体（{last_err}）"
    )))
}

fn font_candidates() -> Vec<PathBuf> {
    [
        // Windows（simhei 为单文件 TTF，兼容性最好）
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simsun.ttc",
        // macOS
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/Supplemental/Songti.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    ]
    .iter()
    .map(PathBuf::from)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_keeps_short_strings() {
        assert_eq!(truncate_chars("ABC", 5), "ABC");
        let long = "一二三四五六七八九十";
        let cut = truncate_chars(long, 5);
        assert!(cut.chars().count() == 5 && cut.ends_with('…'));
    }

    #[test]
    fn width_estimation() {
        assert_eq!(chars_for_width("12345", 8.0, 100.0), 5);
        assert!(chars_for_width("一二三四五", 8.0, 5.0) < 5);
    }
}
