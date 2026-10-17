//! PDF 生成（对应 v4.3 的 `pdf/generator.py` + `pdf/fonts.py`）
//!
//! 布局语义对齐 UI 方案：A4 纵向、列数 1/2/3 可选（默认 3 列 × 7 行 = 21 张/页）、
//! 卡片以凭证编号为核心、文字为主省墨设计、可选附件图片页（2×2/页）。

use std::io::BufWriter;
use std::path::{Path, PathBuf};

use printpdf::{
    Color, Image, ImageTransform, IndirectFontRef, Line, Mm, PdfDocument, PdfLayerReference, Point,
    Rgb,
};
use serde::Serialize;

use crate::{CoreError, Result};

/// 打印选项（与 PrintConfig 字段一致）
#[derive(Debug, Clone, Serialize)]
pub struct PdfOptions {
    pub title: String,
    pub include_images: bool,
    /// v0.8 日报模板：compact（省纸）| archive（档案装订）
    pub daily_template: String,
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

/// 日报「相关凭证清单」（附件清单页，v0.22 批次③）的一行。
///
/// 语义纪律：**关联 ≠ 抵消**——这里的金额只是"相关凭证自己的金额"（参考值），
/// 页面上显式标注「不并入本日报合计」，且绝不影响 `export_items` 的批次张数。
#[derive(Debug, Clone)]
pub struct PdfAppendixRow {
    /// 主凭证编号（本日报里的那张）
    pub main_no: String,
    /// 关系 key：explains | detail
    pub kind_key: String,
    /// 关系中文：附件说明 | 明细
    pub kind_label: String,
    /// 相关凭证编号
    pub other_no: String,
    /// 说明列（相关凭证摘要 / 关系备注）
    pub summary: String,
    /// 相关凭证金额（分；参考值，可空）
    pub amount_cents: Option<i64>,
}

/// 按天日报输入：某一天的凭证集合（一图一凭证模式下即当日全部图片凭证）
#[derive(Debug, Clone)]
pub struct DailyInput {
    /// YYYY-MM-DD
    pub date: String,
    pub cards: Vec<PdfCard>,
    pub images: Vec<PdfImage>,
    /// 相关凭证清单（空 = 不生成附件清单页；由 `ExportOptions::include_links` 控制是否填充）
    pub appendices: Vec<PdfAppendixRow>,
}

// ---------- 页面几何（mm，f32：printpdf 的单位） ----------
const PAGE_W: f32 = 210.0;
const PAGE_H: f32 = 297.0;
const MARGIN: f32 = 10.0;
const HEADER_H: f32 = 16.0;
const FOOTER_H: f32 = 8.0;
const GAP: f32 = 3.0;
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
// ---------- 按天日报版式常量（mm / pt）----------
// 2026-10-16 打印改版（用户要求「字号 ≥ 小四、排版不重叠」）：
//   - 正文/表头/金额/合计一律 12pt（小四），页脚 9pt，图片 caption 10pt，标题 14pt；
//   - **金额列独立预留 40mm**：此前三列吃满 165mm、金额只剩 25mm 且靠右对齐贴页边，
//     金额位数一多（或字号一大）就压进「对方账户」列——这正是「表格右下角重叠」的根因；
//   - 行高 8mm（单行）/ 12mm（摘要换行两行），分页按每行实际高度**贪心装填**；
//   - 表格内容（含合计行）不得越过 TABLE_BOTTOM，页脚基线固定在 290mm。
const TABLE_BOTTOM: f32 = 275.0;
/// 表格每页都预留合计行高度（只有末页真画；换来「合计行永不越界」）
const TOTAL_LINE_H: f32 = 8.0;
/// 表头行高
const THEAD_H: f32 = 6.0;
/// 标题与表头之间的摘要行区域（「当日共 N 张 · 合计 ￥X」）
const TSUM_GAP: f32 = 10.0;

/// 字号（pt）
const TITLE_PT: f32 = 14.0;
const CAPTION_PT: f32 = 10.0;
const HEAD_PT: f32 = 12.0;
const BODY_PT: f32 = 12.0;
const AMOUNT_PT: f32 = 12.0;
/// 金额超宽时的退让字号（仍 ≥ 五号 10.5pt 的下限附近，绝不截断金额）
const AMOUNT_PT_MIN: f32 = 10.5;
const TOTAL_PT: f32 = 12.0;
const FOOTER_PT: f32 = 9.0;

/// 列宽：编号 40 / 摘要 68 / 对方账户 42 / 金额预留 40（合计 190 = 可用宽）
const TCOL_NO: f32 = 40.0;
const TCOL_DESC: f32 = 68.0;
const TCOL_PARTY: f32 = 42.0;
const TCOL_AMOUNT: f32 = 40.0;

/// 行高：单行 8mm（12pt × 1.4 = 5.9mm，留足净空）/ 摘要两行 12mm
const TROW_H: f32 = 8.0;
const TROW_H_2LINE: f32 = 12.0;

// ---------- 日报「相关凭证清单」页（附件清单页，v0.22 批次③）----------
/// 列宽：主凭证 40 / 类型 26 / 相关凭证 38 / 说明 58 / 参考金额 28（合计 190 = 可用宽）
const ACOL_MAIN: f32 = 40.0;
const ACOL_KIND: f32 = 26.0;
const ACOL_OTHER: f32 = 38.0;
const ACOL_DESC: f32 = 58.0;
const ACOL_AMOUNT: f32 = 28.0;
/// 清单页行高与每页行数（等高简单表，不做折行；首页还要放一行说明）
const AROW_H: f32 = 8.0;
const AROWS_PER_PAGE: usize = 24;

/// 清单页的一行：普通关系行，或某关系类型的小计行
enum AppendixLine {
    Row(PdfAppendixRow),
    Subtotal {
        kind_label: String,
        count: usize,
        amount_cents: i64,
    },
}

/// 按关系类型分组（附件说明在前、明细在后），每组末尾插一条小计
fn appendix_lines(rows: &[PdfAppendixRow]) -> Vec<AppendixLine> {
    let mut out = Vec::new();
    for key in ["explains", "detail"] {
        let group: Vec<&PdfAppendixRow> = rows.iter().filter(|r| r.kind_key == key).collect();
        if group.is_empty() {
            continue;
        }
        for r in &group {
            out.push(AppendixLine::Row((*r).clone()));
        }
        out.push(AppendixLine::Subtotal {
            kind_label: group[0].kind_label.clone(),
            count: group.len(),
            amount_cents: group.iter().filter_map(|r| r.amount_cents).sum(),
        });
    }
    // 兜底：未知 kind 的行也要出现在清单里（不能静默丢）
    let known = |k: &str| k == "explains" || k == "detail";
    let others: Vec<&PdfAppendixRow> = rows.iter().filter(|r| !known(&r.kind_key)).collect();
    if !others.is_empty() {
        for r in &others {
            out.push(AppendixLine::Row((*r).clone()));
        }
        out.push(AppendixLine::Subtotal {
            kind_label: "其它".into(),
            count: others.len(),
            amount_cents: others.iter().filter_map(|r| r.amount_cents).sum(),
        });
    }
    out
}

/// 清单页分页（等高行，按每页 `AROWS_PER_PAGE` 行切）
fn appendix_page_slices(lines: &[AppendixLine]) -> Vec<(usize, usize)> {
    if lines.is_empty() {
        return Vec::new();
    }
    (0..lines.len())
        .step_by(AROWS_PER_PAGE)
        .map(|start| (start, (start + AROWS_PER_PAGE).min(lines.len())))
        .collect()
}

/// 画一页「相关凭证清单」：首行说明 + 表头 + 等高的关系行与小计行
fn draw_appendix_page(
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    lines: &[AppendixLine],
    range: (usize, usize),
) {
    let top = MARGIN + HEADER_H + TSUM_GAP;
    layer.set_fill_color(rgb(GREY.0, GREY.1, GREY.2));
    layer.use_text(
        "本页仅为佐证索引：金额是相关凭证自己的金额，不并入本日报合计",
        FOOTER_PT,
        Mm(MARGIN),
        y(top + 3.0),
        font,
    );
    let head_top = top + 5.0;
    draw_head_cells(
        layer,
        font,
        head_top,
        &[
            HeadCol {
                label: "主凭证",
                x: HeadColX::Left(MARGIN + 1.0),
            },
            HeadCol {
                label: "类型",
                x: HeadColX::Left(MARGIN + ACOL_MAIN + 1.0),
            },
            HeadCol {
                label: "相关凭证",
                x: HeadColX::Left(MARGIN + ACOL_MAIN + ACOL_KIND + 1.0),
            },
            HeadCol {
                label: "说明",
                x: HeadColX::Left(MARGIN + ACOL_MAIN + ACOL_KIND + ACOL_OTHER + 1.0),
            },
            HeadCol {
                label: "参考金额",
                x: HeadColX::Right,
            },
        ],
    );

    let mut yy = head_top + THEAD_H;
    for line in &lines[range.0..range.1] {
        let baseline = yy + AROW_H - 1.9;
        match line {
            AppendixLine::Row(r) => {
                cell_text(
                    layer,
                    font,
                    &r.main_no,
                    BODY_PT,
                    MARGIN + 1.0,
                    baseline,
                    Some(ACOL_MAIN - 2.0),
                    ACCENT,
                );
                cell_text(
                    layer,
                    font,
                    &r.kind_label,
                    BODY_PT,
                    MARGIN + ACOL_MAIN + 1.0,
                    baseline,
                    Some(ACOL_KIND - 2.0),
                    BODY,
                );
                cell_text(
                    layer,
                    font,
                    &r.other_no,
                    BODY_PT,
                    MARGIN + ACOL_MAIN + ACOL_KIND + 1.0,
                    baseline,
                    Some(ACOL_OTHER - 2.0),
                    ACCENT,
                );
                cell_text(
                    layer,
                    font,
                    &r.summary,
                    BODY_PT,
                    MARGIN + ACOL_MAIN + ACOL_KIND + ACOL_OTHER + 1.0,
                    baseline,
                    Some(ACOL_DESC - 2.0),
                    GREY,
                );
                if let Some(cents) = r.amount_cents {
                    let label = format!("￥{}", crate::models::voucher::format_money(cents));
                    let (pt, x) = amount_pt_and_x(&label);
                    cell_text(
                        layer,
                        font,
                        &label,
                        pt.min(BODY_PT),
                        x,
                        baseline,
                        Some(ACOL_AMOUNT - 2.0),
                        DARK,
                    );
                }
            }
            AppendixLine::Subtotal {
                kind_label,
                count,
                amount_cents,
            } => {
                let label = format!(
                    "{kind_label} 小计：{count} 条 · 相关凭证金额 ￥{}（参考，不并入合计）",
                    crate::models::voucher::format_money(*amount_cents)
                );
                cell_text(
                    layer,
                    font,
                    &label,
                    BODY_PT,
                    MARGIN + 1.0,
                    baseline,
                    Some(PAGE_W - MARGIN * 2.0 - 2.0),
                    DARK,
                );
            }
        }
        hline(layer, 0.25, yy + AROW_H - 0.3, STROKE);
        yy += AROW_H;
    }
}

/// 一行表格的版式（摘要最多两行）
struct RowLayout {
    no: String,
    desc1: String,
    desc2: Option<String>,
    party: String,
    amount: String,
    h: f32,
}

/// 摘要按列宽折成最多两行；第二行仍放不下的部分截断加「…」
fn wrap_two_lines(s: &str, pt: f32, cap_mm: f32) -> (String, Option<String>) {
    let chars = s.chars().count();
    let fits = chars_for_width(s, pt, cap_mm);
    if fits >= chars {
        return (s.to_string(), None);
    }
    let cut: String = s.chars().take(fits).collect();
    let rest: String = s.chars().skip(fits).collect();
    let rest_fits = chars_for_width(&rest, pt, cap_mm);
    if rest_fits >= rest.chars().count() {
        (cut, Some(rest))
    } else {
        let keep = rest_fits.saturating_sub(1);
        let head: String = rest.chars().take(keep).collect();
        (cut, Some(format!("{head}…")))
    }
}

/// 把凭证排成行版式（摘要折行、行高随之变化）
fn layout_rows(cards: &[PdfCard]) -> Vec<RowLayout> {
    cards
        .iter()
        .map(|c| {
            let desc = if c.description.is_empty() {
                "—".to_string()
            } else {
                c.description.clone()
            };
            let (desc1, desc2) = wrap_two_lines(&desc, BODY_PT, TCOL_DESC - 2.0);
            let h = if desc2.is_some() {
                TROW_H_2LINE
            } else {
                TROW_H
            };
            RowLayout {
                no: c.voucher_no.clone(),
                desc1,
                desc2,
                party: if c.counterparty.is_empty() {
                    "—".to_string()
                } else {
                    c.counterparty.clone()
                },
                amount: c.amount_label.clone(),
                h,
            }
        })
        .collect()
}

/// 贪心分页：按每行实际高度装填，每页都预留合计行高度。
/// 返回 `page → (start, end)` 行区间；`first_top` 用于首页（省纸模板首页有横幅），其余页用 `next_top`。
fn paginate_rows(rows: &[RowLayout], first_top: f32, next_top: f32) -> Vec<(usize, usize)> {
    let mut pages: Vec<(usize, usize)> = Vec::new();
    let mut i = 0usize;
    let mut top = first_top;
    while i < rows.len() {
        let mut y = top + THEAD_H;
        let mut j = i;
        while j < rows.len() && y + rows[j].h + TOTAL_LINE_H <= TABLE_BOTTOM {
            y += rows[j].h;
            j += 1;
        }
        if j == i {
            j = i + 1; // 兜底：至少放一行
        }
        pages.push((i, j));
        i = j;
        top = next_top;
    }
    if pages.is_empty() {
        pages.push((0, 0));
    }
    pages
}

/// 金额字号：12pt 放不下时退到 10.5pt（**绝不截断金额**）；返回 (字号, 右对齐起点)
fn amount_pt_and_x(label: &str) -> (f32, f32) {
    let min_x = MARGIN + TCOL_NO + TCOL_DESC + TCOL_PARTY + 1.0;
    let right = PAGE_W - MARGIN;
    let pt = if text_width_mm(label, AMOUNT_PT) <= TCOL_AMOUNT - 2.0 {
        AMOUNT_PT
    } else {
        AMOUNT_PT_MIN
    };
    (pt, (right - text_width_mm(label, pt)).max(min_x))
}

/// 画一页表格（表头 + 行 + 末页合计行），返回本页内容底部的 y（mm）
fn draw_table_page(
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    rows: &[RowLayout],
    range: (usize, usize),
    top: f32,
) -> f32 {
    draw_table_head(layer, font, top);
    let mut y = top + THEAD_H;
    for r in &rows[range.0..range.1] {
        draw_table_row(layer, font, r, y);
        y += r.h;
    }
    y
}

// ---------- v0.8 省纸模板常量（compact） ----------
/// 信息横幅高度（原独立封面页的替代，压到 22mm）
const C_BANNER_H: f32 = 22.0;
/// 竖版票据每页张数（3×3）
const C_PORTRAITS_PER_PAGE: usize = 9;
/// 省纸：横图桶每页（2×2）
const C_LANDSCAPES_PER_PAGE: usize = 4;

// ---------- 日报共享渲染（档案/省纸两模板复用，参数化差异） ----------

/// 图片页版式规格：网格行列 + caption 样式（2026-10-16 起两模板 caption 均 10pt）
struct ImageGridSpec {
    cols: usize,
    rows: usize,
    caption_pt: f32,
    caption_h: f32,
    caption_dy: f32,
}

/// 档案模板图片规格（2×2/页，caption 7pt）
const SPEC_ARCHIVE_GRID: ImageGridSpec = ImageGridSpec {
    cols: IMG_COLS,
    rows: IMG_ROWS,
    caption_pt: CAPTION_PT,
    caption_h: 7.0,
    caption_dy: 0.9,
};
/// 省纸模板图片规格（caption 10pt，行列由桶决定）
const SPEC_COMPACT_CAPTION: (f32, f32, f32) = (CAPTION_PT, 7.0, 0.9);

/// 合计行（两模板共用）：分隔线 + 「合计 / N 张 / ￥金额」右对齐。
/// `text_dy` 为文本基线相对分隔线的下沉量（按行高微调）。
fn draw_total_line(
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    top: f32,
    count: usize,
    total_cents: i64,
    text_dy: f32,
) {
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
    layer.use_text("合计", TOTAL_PT, Mm(MARGIN + 1.0), y(top + text_dy), font);

    let amt_label = format!("￥{}", crate::models::voucher::format_money(total_cents));
    let cnt_label = format!("{count} 张");
    let (cnt_x, amt_x) = total_line_x(&cnt_label, &amt_label);
    layer.use_text(&amt_label, TOTAL_PT, Mm(amt_x), y(top + text_dy), font);
    layer.use_text(&cnt_label, TOTAL_PT, Mm(cnt_x), y(top + text_dy), font);
}

/// 合计行横坐标（mm）：`(张数 x, 金额 x)`——金额右对齐到页面右边界（与表格金额列同一条右边线），
/// 张数放在**金额左侧 6mm 处**。
///
/// 历史：原先张数是"右边界 − 2mm"，12pt 下金额更宽 → 两条文字叠在右下角
/// （用户 2026-10-17 截图报「右下角仍有重叠」）。抽成纯函数是为让"不重叠"成为可测不变量。
fn total_line_x(cnt_label: &str, amt_label: &str) -> (f32, f32) {
    let amt_x = right_x(amt_label, TOTAL_PT, PAGE_W - MARGIN);
    let cnt_x = amt_x - text_width_mm(cnt_label, TOTAL_PT) - 6.0;
    (cnt_x.max(MARGIN + 12.0), amt_x)
}

/// 单页图片网格（两模板共用）：按 spec 行列排布一页内的槽位，
/// 等比缩放放置图片 + 底部 caption（超出截断）。返回实际放置张数。
fn draw_image_grid(
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    imgs: &[&PdfImage],
    page_idx: usize,
    spec: &ImageGridSpec,
) {
    let per_page = spec.cols * spec.rows;
    let usable_w = PAGE_W - MARGIN * 2.0;
    let usable_h = PAGE_H - MARGIN * 2.0 - HEADER_H - FOOTER_H;
    let cell_w = (usable_w - GAP) / spec.cols as f32;
    let cell_h = (usable_h - GAP) / spec.rows as f32;
    for slot in 0..per_page {
        let idx = page_idx * per_page + slot;
        if idx >= imgs.len() {
            break;
        }
        let col = slot % spec.cols;
        let row = slot / spec.cols;
        let x = MARGIN + col as f32 * (cell_w + GAP);
        let top = MARGIN + HEADER_H + row as f32 * (cell_h + GAP);
        let img_area_h = cell_h - spec.caption_h - 1.0;
        place_image(layer, imgs[idx], x, top, cell_w, img_area_h);
        layer.set_fill_color(rgb(ACCENT.0, ACCENT.1, ACCENT.2));
        let caption = truncate_chars(
            &imgs[idx].caption,
            chars_for_width(&imgs[idx].caption, spec.caption_pt, cell_w - 2.0),
        );
        layer.use_text(
            caption,
            spec.caption_pt,
            Mm(x + 1.0),
            y(top + cell_h - spec.caption_dy),
            font,
        );
    }
}

/// 生成「按天日报」PDF。v0.8 起按 `daily_template` 分版式：
/// - compact（默认）：省纸——首页信息横幅 + 表格 38 行/页 + 竖票 3×3 / 横票 2×2
/// - archive：档案装订——独立封面 + 装订线 + 表格 34 行/页 + 图片 2×2
pub fn generate_daily(path: &Path, day: &DailyInput, opts: &PdfOptions) -> Result<()> {
    if day.cards.is_empty() {
        return Err(CoreError::Invalid("当天没有可导出的凭证".into()));
    }
    if opts.daily_template == "archive" {
        generate_daily_archive(path, day, opts)
    } else {
        generate_daily_compact(path, day, opts)
    }
}

/// 档案装订模板：P1 封面（装订线 + 档号范围/张数/金额合计）→ P2+ 汇总表（自动分页 + 合计行）→ 图片页（2×2/页）
fn generate_daily_archive(path: &Path, day: &DailyInput, opts: &PdfOptions) -> Result<()> {
    let base_title = if opts.title.trim().is_empty() {
        "原始凭证".to_string()
    } else {
        opts.title.trim().to_string()
    };
    let cover_title = format!("{base_title}封面");
    let sum_title = format!("凭证汇总表 · {}", day.date);

    let (doc, first_page, first_layer) =
        PdfDocument::new(&cover_title, Mm(PAGE_W), Mm(PAGE_H), "Cover");
    let font = load_cjk_font(&doc)?;

    let count = day.cards.len();
    let total_cents: i64 = day.cards.iter().filter_map(|c| c.amount_cents).sum();
    let img_count = if opts.include_images {
        day.images.len()
    } else {
        0
    };
    let img_pages = img_count.div_ceil(IMG_COLS * IMG_ROWS);
    // 2026-10-16 打印改版：按每行实际高度（8mm / 摘要两行 12mm）贪心分页
    let rows = layout_rows(&day.cards);
    let table_first_top = MARGIN + HEADER_H + TSUM_GAP;
    let pages = paginate_rows(&rows, table_first_top, table_first_top);
    let table_pages = pages.len();
    // 相关凭证清单页（数据驱动：没有关联就没有这一页）
    let app_lines = appendix_lines(&day.appendices);
    let app_slices = appendix_page_slices(&app_lines);
    let appendix_pages = app_slices.len();
    let total_pages = 1 + table_pages + appendix_pages + img_pages;

    // ===== P1 封面 =====
    {
        let layer = doc.get_page(first_page).get_layer(first_layer);
        draw_cover_page(&layer, &font, day, &base_title);
    }

    // ===== P2+ 汇总表页 =====
    for (t, range) in pages.iter().enumerate() {
        let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Summary");
        let layer = doc.get_page(p).get_layer(l);
        draw_header(&layer, &font, &sum_title);

        // 首页摘要行：当日共 N 张 · 合计 ¥X（仅统计已填金额的凭证）
        if t == 0 {
            layer.set_fill_color(rgb(DARK.0, DARK.1, DARK.2));
            layer.use_text(
                format!(
                    "当日共 {count} 张凭证 · 合计 ￥{}（按识别金额统计）",
                    crate::models::voucher::format_money(total_cents)
                ),
                BODY_PT,
                Mm(MARGIN),
                y(MARGIN + HEADER_H + 7.0),
                &font,
            );
        }

        // 表头（每页重复）+ 行（摘要可折两行）
        let table_top = MARGIN + HEADER_H + TSUM_GAP;
        let bottom = draw_table_page(&layer, &font, &rows, *range, table_top);
        // 合计行（最后一页表格底部）
        if t == table_pages - 1 {
            draw_total_line(&layer, &font, bottom + 1.0, count, total_cents, 5.0);
        }

        draw_footer(&layer, &font, t + 2, total_pages);
    }

    // ===== 相关凭证清单页（附件清单页） =====
    for (i, range) in app_slices.iter().enumerate() {
        let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Appendix");
        let layer = doc.get_page(p).get_layer(l);
        draw_header(&layer, &font, &format!("相关凭证清单 · {}", day.date));
        draw_appendix_page(&layer, &font, &app_lines, *range);
        draw_footer(&layer, &font, 1 + table_pages + i + 1, total_pages);
    }

    // ===== 图片页（2×2/页，caption 对齐参照：附件N：摘要_对方_金额元（档号）） =====
    let img_refs: Vec<&PdfImage> = day.images.iter().collect();
    for page_idx in 0..img_pages {
        let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Images");
        let layer = doc.get_page(p).get_layer(l);
        draw_header(&layer, &font, &format!("附件 · {}", day.date));
        draw_image_grid(&layer, &font, &img_refs, page_idx, &SPEC_ARCHIVE_GRID);
        draw_footer(
            &layer,
            &font,
            1 + table_pages + appendix_pages + page_idx + 1,
            total_pages,
        );
    }

    doc.save(&mut BufWriter::new(std::fs::File::create(path)?))?;
    Ok(())
}

/// 省纸模板：首页信息横幅 + 汇总表 → 竖票 3×3 → 横票 2×2。
/// 无独立封面页（横幅替代）；表格行高 5.5mm，首页 32 行 / 后续 38 行。
/// 图片按宽高比自动分桶（等比缩放完整显示，不混排：横图塞 3×3 窄格会不可读）。
fn generate_daily_compact(path: &Path, day: &DailyInput, opts: &PdfOptions) -> Result<()> {
    let sum_title = format!("凭证汇总表 · {}", day.date);

    let (doc, first_page, first_layer) =
        PdfDocument::new(&sum_title, Mm(PAGE_W), Mm(PAGE_H), "Summary");
    let font = load_cjk_font(&doc)?;

    let count = day.cards.len();
    let total_cents: i64 = day.cards.iter().filter_map(|c| c.amount_cents).sum();

    // 图片分桶：竖图（h > w）进 3×3 页，横图进 2×2 页。
    // 读尺寸头失败按横图处理（2×2 单格更大，保守）；等比缩放下两者都完整显示。
    let mut portraits: Vec<&PdfImage> = Vec::new();
    let mut landscapes: Vec<&PdfImage> = Vec::new();
    for img in &day.images {
        match image::image_dimensions(&img.path) {
            Ok((w, h)) if h > w => portraits.push(img),
            _ => landscapes.push(img),
        }
    }
    let img_pages = if opts.include_images {
        portraits.len().div_ceil(C_PORTRAITS_PER_PAGE)
            + landscapes.len().div_ceil(C_LANDSCAPES_PER_PAGE)
    } else {
        0
    };

    // 表格分页：首页含横幅（可用高度更小），后续页从表头开始；按每行实际高度贪心装填
    let rows = layout_rows(&day.cards);
    let first_top = MARGIN + HEADER_H + C_BANNER_H + TSUM_GAP;
    let next_top = MARGIN + HEADER_H + TSUM_GAP;
    let pages = paginate_rows(&rows, first_top, next_top);
    let table_pages = pages.len();
    // 相关凭证清单页（数据驱动：没有关联就没有这一页）
    let app_lines = appendix_lines(&day.appendices);
    let app_slices = appendix_page_slices(&app_lines);
    let appendix_pages = app_slices.len();
    let total_pages = table_pages + appendix_pages + img_pages;

    // ===== 表格页（首页含信息横幅，无独立封面） =====
    for (t, range) in pages.iter().enumerate() {
        let layer = if t == 0 {
            doc.get_page(first_page).get_layer(first_layer)
        } else {
            let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Summary");
            doc.get_page(p).get_layer(l)
        };
        draw_header(&layer, &font, &sum_title);

        if t == 0 {
            draw_compact_banner(&layer, &font, day, count, total_cents);
        }
        let table_top = if t == 0 { first_top } else { next_top };
        let bottom = draw_table_page(&layer, &font, &rows, *range, table_top);
        // 合计行（末页表格底部）
        if t == table_pages - 1 {
            draw_total_line(&layer, &font, bottom + 1.0, count, total_cents, 5.0);
        }

        draw_footer(&layer, &font, t + 1, total_pages);
    }

    // ===== 相关凭证清单页（附件清单页） =====
    for (i, range) in app_slices.iter().enumerate() {
        let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Appendix");
        let layer = doc.get_page(p).get_layer(l);
        draw_header(&layer, &font, &format!("相关凭证清单 · {}", day.date));
        draw_appendix_page(&layer, &font, &app_lines, *range);
        draw_footer(&layer, &font, table_pages + i + 1, total_pages);
    }

    // ===== 图片页：先竖票 3×3，后横票 2×2（caption 6.5pt 紧凑） =====
    if opts.include_images {
        let mut page_no = table_pages + appendix_pages;
        let (pt, ch, dy) = SPEC_COMPACT_CAPTION;
        let buckets: [(Vec<&PdfImage>, ImageGridSpec); 2] = [
            (
                portraits,
                ImageGridSpec {
                    cols: 3,
                    rows: 3,
                    caption_pt: pt,
                    caption_h: ch,
                    caption_dy: dy,
                },
            ),
            (
                landscapes,
                ImageGridSpec {
                    cols: 2,
                    rows: 2,
                    caption_pt: pt,
                    caption_h: ch,
                    caption_dy: dy,
                },
            ),
        ];
        for (imgs, spec) in buckets {
            if imgs.is_empty() {
                continue;
            }
            let pages = imgs.len().div_ceil(spec.cols * spec.rows);
            for page_idx in 0..pages {
                let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Images");
                let layer = doc.get_page(p).get_layer(l);
                draw_header(&layer, &font, &format!("附件 · {}", day.date));
                draw_image_grid(&layer, &font, &imgs, page_idx, &spec);
                page_no += 1;
                draw_footer(&layer, &font, page_no, total_pages);
            }
        }
    }

    doc.save(&mut BufWriter::new(std::fs::File::create(path)?))?;
    Ok(())
}

/// 省纸模板信息横幅（替代档案模板的独立封面页）：
/// 左侧主色竖线 + 3 行关键信息（日期/生成时间、档号范围、张数/金额合计）。
fn draw_compact_banner(
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    day: &DailyInput,
    count: usize,
    total_cents: i64,
) {
    let top = MARGIN + HEADER_H;

    // 左侧主色竖线（视觉锚点，提示这是摘要信息块）
    layer.set_outline_thickness(0.8);
    layer.set_outline_color(rgb(ACCENT.0, ACCENT.1, ACCENT.2));
    layer.add_line(Line {
        points: vec![
            (Point::new(Mm(MARGIN), y(top)), false),
            (Point::new(Mm(MARGIN), y(top + C_BANNER_H)), false),
        ],
        is_closed: false,
    });

    let first_no = day
        .cards
        .first()
        .map(|c| c.voucher_no.as_str())
        .unwrap_or_default();
    let last_no = day
        .cards
        .last()
        .map(|c| c.voucher_no.as_str())
        .unwrap_or_default();
    let range = if first_no == last_no {
        first_no.to_string()
    } else {
        format!("{first_no} ～ {last_no}")
    };
    let generated = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let rows = [
        format!("凭证日期：{} · 生成时间：{generated}", day.date),
        format!("档号范围：{range}"),
        format!(
            "{count} 张 · 合计 ￥{}（按识别金额统计）",
            crate::models::voucher::format_money(total_cents)
        ),
    ];
    for (i, r) in rows.iter().enumerate() {
        layer.set_fill_color(rgb(DARK.0, DARK.1, DARK.2));
        layer.use_text(
            r,
            BODY_PT,
            Mm(MARGIN + 3.0),
            y(top + 5.0 + i as f32 * 6.8),
            font,
        );
    }
}

/// 封面页（档案装订形态）：
/// 左侧装订线（竖排「装订线」三字 + 细线）+ 居中大标题 + 5 行信息表
/// 标题在页面上半部居中，信息表在标题下方，整体视觉重心中上。
fn draw_cover_page(
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    day: &DailyInput,
    title: &str,
) {
    // 装订线三字（竖排）——与标题垂直居中对齐
    let title_top = 140.0; // 标题 baseline 距页顶 140mm（原 88mm 下移导致上方 78mm 空白，上移 52mm 至中上位置）
    layer.set_fill_color(rgb(GREY.0, GREY.1, GREY.2));
    for (i, ch) in ["装", "订", "线"].iter().enumerate() {
        layer.use_text(*ch, 12.0, Mm(18.0), y(title_top + i as f32 * 14.0), font);
    }
    // 装订线（贯穿全页，左侧 24.7mm）
    layer.set_outline_thickness(0.3);
    layer.set_outline_color(rgb(STROKE.0, STROKE.1, STROKE.2));
    layer.add_line(Line {
        points: vec![
            (Point::new(Mm(24.7), y(15.0)), false),
            (Point::new(Mm(24.7), y(282.0)), false),
        ],
        is_closed: false,
    });

    // 居中大标题（18pt → 22pt，更醒目；上移 52mm 至中上位置）
    let cover_title = format!("{title}封面");
    layer.set_fill_color(rgb(DARK.0, DARK.1, DARK.2));
    let tw = text_width_mm(&cover_title, 22.0);
    layer.use_text(
        &cover_title,
        22.0,
        Mm((PAGE_W - tw) / 2.0),
        y(title_top + 10.0),
        font,
    );

    // 信息表 5 行（下移至标题下方，table_top = 175mm → 标题下约 25mm）
    let count = day.cards.len();
    let total_cents: i64 = day.cards.iter().filter_map(|c| c.amount_cents).sum();
    let unrecognized = day
        .cards
        .iter()
        .filter(|c| c.amount_cents.is_none())
        .count();
    let first_no = day
        .cards
        .first()
        .map(|c| c.voucher_no.as_str())
        .unwrap_or_default();
    let last_no = day
        .cards
        .last()
        .map(|c| c.voucher_no.as_str())
        .unwrap_or_default();
    let range = if first_no == last_no {
        first_no.to_string()
    } else {
        format!("{first_no} ～ {last_no}")
    };
    let unrecognized_note = if unrecognized > 0 {
        format!("，{unrecognized} 张未识别金额")
    } else {
        String::new()
    };
    let amount = format!(
        "￥{}（按识别金额统计{unrecognized_note}）",
        crate::models::voucher::format_money(total_cents)
    );
    let generated = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let rows: [(String, String); 5] = [
        ("凭证日期".into(), day.date.clone()),
        ("档号范围".into(), range),
        ("附件张数".into(), format!("{count} 张")),
        ("金额合计".into(), amount),
        ("生成时间".into(), generated),
    ];

    let table_left = 42.3;
    let table_right = 178.0;
    let label_x = table_left + 2.0;
    let value_x = 77.6;
    let row_h = 18.4;
    let table_top = title_top + 35.0; // 标题下方 25mm 开始表格
    for (i, (label, value)) in rows.iter().enumerate() {
        let top = table_top + i as f32 * row_h;
        layer.set_outline_thickness(0.4);
        layer.set_outline_color(rgb(STROKE.0, STROKE.1, STROKE.2));
        layer.add_line(Line {
            points: vec![
                (Point::new(Mm(table_left), y(top + row_h)), false),
                (Point::new(Mm(table_right), y(top + row_h)), false),
            ],
            is_closed: false,
        });
        layer.set_fill_color(rgb(GREY.0, GREY.1, GREY.2));
        layer.use_text(label, 10.5, Mm(label_x), y(top + 12.0), font);
        layer.set_fill_color(rgb(DARK.0, DARK.1, DARK.2));
        layer.use_text(value, 10.5, Mm(value_x), y(top + 12.0), font);
    }
}

// ================= 表格引擎（v0.14：日报表 / 年度汇总表共用的列定义渲染） =================
// 重构前 draw_table_head/draw_yearly_head 尾部 10 行与 draw_table_row/draw_yearly_row 的
// 基线公式(top + row_h - 1.9)、行底分隔线(top + row_h - 0.3)、填色切换完全逐字重复；
// 年度侧列偏移还是裸 magic number(30/50/70/92/114)。现收口为：列定义数据驱动表头 +
// 统一 cell 文本/分隔线基元。列 x 值逐一保持重构前的数值（layout golden 测试锁定）。

/// 表头单元格水平定位：左对齐固定 x，或右对齐至右页边
#[derive(Clone, Copy)]
enum HeadColX {
    Left(f32),
    Right,
}

/// 表头列定义：列名 + 定位
struct HeadCol<'a> {
    label: &'a str,
    x: HeadColX,
}

/// 表头渲染：灰字列名（金额右对齐），底部 0.6mm 横线。日报/年度共用。
fn draw_head_cells(
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    top: f32,
    cols: &[HeadCol<'_>],
) {
    layer.set_fill_color(rgb(GREY.0, GREY.1, GREY.2));
    let y0 = top + 4.0;
    for c in cols {
        let x = match c.x {
            HeadColX::Left(x) => Mm(x),
            HeadColX::Right => Mm(right_x(c.label, HEAD_PT, PAGE_W - MARGIN)),
        };
        layer.use_text(c.label, HEAD_PT, x, y(y0), font);
    }
    hline(layer, 0.6, top + THEAD_H - 0.5, STROKE);
}

/// 单元格文本：统一「填色 → 可选按宽度截断 → 落字」，表行/合计行共用。
#[allow(clippy::too_many_arguments)]
fn cell_text(
    layer: &PdfLayerReference,
    font: &IndirectFontRef,
    s: &str,
    font_pt: f32,
    x: f32,
    y_mm: f32,
    cap_mm: Option<f32>,
    fill: (f32, f32, f32),
) {
    layer.set_fill_color(rgb(fill.0, fill.1, fill.2));
    let text = match cap_mm {
        Some(w) => std::borrow::Cow::Owned(truncate_chars(s, chars_for_width(s, font_pt, w))),
        None => std::borrow::Cow::Borrowed(s),
    };
    layer.use_text(&*text, font_pt, Mm(x), y(y_mm), font);
}

/// 横向分隔线（表头底 / 行底 / 年合计顶共用；颜色区分加粗线）
fn hline(layer: &PdfLayerReference, thickness: f32, y_mm: f32, color: (f32, f32, f32)) {
    layer.set_outline_thickness(thickness);
    layer.set_outline_color(rgb(color.0, color.1, color.2));
    layer.add_line(Line {
        points: vec![
            (Point::new(Mm(MARGIN), y(y_mm)), false),
            (Point::new(Mm(PAGE_W - MARGIN), y(y_mm)), false),
        ],
        is_closed: false,
    });
}

/// 日报表头（列定义数据驱动；编号/摘要/对方账户 固定 x，金额右对齐）
fn draw_table_head(layer: &PdfLayerReference, font: &IndirectFontRef, top: f32) {
    draw_head_cells(
        layer,
        font,
        top,
        &[
            HeadCol {
                label: "编号",
                x: HeadColX::Left(MARGIN + 1.0),
            },
            HeadCol {
                label: "摘要",
                x: HeadColX::Left(MARGIN + TCOL_NO + 1.0),
            },
            HeadCol {
                label: "对方账户",
                x: HeadColX::Left(MARGIN + TCOL_NO + TCOL_DESC + 1.0),
            },
            HeadCol {
                label: "金额",
                x: HeadColX::Right,
            },
        ],
    );
}

/// 行绘制（2026-10-16 打印改版：正文 12pt、摘要最多两行、金额在独立列内右对齐**不压列**）
fn draw_table_row(layer: &PdfLayerReference, font: &IndirectFontRef, r: &RowLayout, top: f32) {
    let desc_x = MARGIN + TCOL_NO + 1.0;
    let party_x = MARGIN + TCOL_NO + TCOL_DESC + 1.0;
    // 单行：基线贴行底留 1.9mm；两行：5.0 / 10.4mm（行距 5.4mm ≥ 12pt 行高）
    let (b1, b2) = if r.desc2.is_some() {
        (top + 5.0, Some(top + 10.4))
    } else {
        (top + r.h - 1.9, None)
    };

    // 编号（主色）
    cell_text(
        layer,
        font,
        &r.no,
        BODY_PT,
        MARGIN + 1.0,
        b1,
        Some(TCOL_NO - 2.0),
        ACCENT,
    );
    // 摘要（最多两行；空摘要已在 layout_rows 里换成「—」）
    cell_text(
        layer,
        font,
        &r.desc1,
        BODY_PT,
        desc_x,
        b1,
        Some(TCOL_DESC - 2.0),
        BODY,
    );
    if let (Some(d2), Some(y2)) = (r.desc2.as_deref(), b2) {
        cell_text(
            layer,
            font,
            d2,
            BODY_PT,
            desc_x,
            y2,
            Some(TCOL_DESC - 2.0),
            BODY,
        );
    }
    // 对方账户（小灰字）
    cell_text(
        layer,
        font,
        &r.party,
        BODY_PT,
        party_x,
        b1,
        Some(TCOL_PARTY - 2.0),
        GREY,
    );
    // 金额（独立列内右对齐：12pt 放不下才退 10.5pt，绝不侵入「对方账户」列）
    if !r.amount.is_empty() {
        let (pt, x) = amount_pt_and_x(&r.amount);
        cell_text(
            layer,
            font,
            &r.amount,
            pt,
            x,
            b1,
            Some(TCOL_AMOUNT - 2.0),
            DARK,
        );
    }

    // 行底分隔线（省墨：仅横向细线）
    hline(layer, 0.25, top + r.h - 0.3, STROKE);
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
    layer.use_text(title, TITLE_PT, Mm(MARGIN), y(MARGIN + 8.0), font);
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
        FOOTER_PT,
        Mm(MARGIN),
        y(PAGE_H - MARGIN + 3.0),
        font,
    );
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
        // macOS
        // 注：PingFang.ttc 在较新的 macOS（本机 macOS 27）上已不存在，保留在前只为兼容旧系统；
        // Hiragino Sans GB 是当前系统上体积较小的 CJK 集合字体（约 23 MB，而 STHeiti 55 MB），
        // 排在 STHeiti 之前以免回退到最大的那个（2026-09-23 审计 A4）。
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/Supplemental/Songti.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        // Linux（fonts-noto-cjk 的 TTC 优先；Droid 为单文件 TTF 兜底，
        // 防部分 PDF 库解析集合字体失败）
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
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

    /// 合计行「不重叠」：张数必须整体落在金额左侧（用户 2026-10-17 报「右下角仍有重叠」的回放）。
    #[test]
    fn total_line_labels_do_not_overlap() {
        for (cnt, amt) in [
            ("6 张", "￥130,039.08"),
            ("102 张", "￥9,999,999.99"),
            ("1 张", "￥0.00"),
        ] {
            let (cnt_x, amt_x) = total_line_x(cnt, amt);
            let cnt_right = cnt_x + text_width_mm(cnt, TOTAL_PT);
            assert!(
                cnt_right <= amt_x,
                "「{cnt}」右边界 {cnt_right:.2}mm 压到金额左边界 {amt_x:.2}mm"
            );
            assert!(cnt_x >= MARGIN + 12.0 - 0.01, "「{cnt}」起点越出左边线");
            assert!(
                amt_x + text_width_mm(amt, TOTAL_PT) <= PAGE_W - MARGIN + 0.01,
                "金额越出右边线"
            );
        }
    }

    #[test]
    fn width_estimation() {
        assert_eq!(chars_for_width("12345", 8.0, 100.0), 5);
        assert!(chars_for_width("一二三四五", 8.0, 5.0) < 5);
    }

    #[test]
    fn daily_columns_layout_golden() {
        // 2026-10-16 打印改版后的列几何：编号 40 / 摘要 68 / 对方 42 / **金额独立预留 40**
        // （改版前金额只剩 25mm、靠右对齐贴页边 → 金额位数一多就压进「对方账户」列）
        assert_eq!(MARGIN + TCOL_NO, 50.0, "摘要列起点");
        assert_eq!(MARGIN + TCOL_NO + TCOL_DESC, 118.0, "对方账户列起点");
        assert_eq!(
            MARGIN + TCOL_NO + TCOL_DESC + TCOL_PARTY,
            160.0,
            "金额列起点"
        );
        assert_eq!(
            MARGIN + TCOL_NO + TCOL_DESC + TCOL_PARTY + TCOL_AMOUNT,
            200.0,
            "金额列右边界"
        );
    }

    #[test]
    fn daily_layout_invariants() {
        // ① 四列吃满可用宽，不溢出页边
        assert_eq!(
            TCOL_NO + TCOL_DESC + TCOL_PARTY + TCOL_AMOUNT,
            PAGE_W - MARGIN * 2.0
        );
        // ② 最宽的常见金额在 12pt 下必须放得进金额列（含 2mm 余量）
        let widest = "￥99,999,999.99";
        assert!(
            text_width_mm(widest, AMOUNT_PT) <= TCOL_AMOUNT - 2.0,
            "最宽金额 {} mm 超过金额列可用宽 {} mm",
            text_width_mm(widest, AMOUNT_PT),
            TCOL_AMOUNT - 2.0
        );
        // ③ 金额右对齐起点绝不越过金额列左边界（否则会压进「对方账户」列）
        let (_, x) = amount_pt_and_x(widest);
        assert!(
            x >= MARGIN + TCOL_NO + TCOL_DESC + TCOL_PARTY + 1.0,
            "金额压列：x={x}"
        );
        // ④ 行高 ≥ 字号 × 1.4（12pt → ≥5.9mm），两行行高 ≥ 两倍
        let row_h = TROW_H;
        let min_row_h = BODY_PT * 0.3528 * 1.4;
        assert!(row_h >= min_row_h, "行高 {row_h} < 字号×1.4 = {min_row_h}");
        let two_line_h = TROW_H_2LINE;
        let single_h = TROW_H;
        assert!(two_line_h >= single_h * 1.4, "两行行高不足");
        // ⑤ 超长金额退字号但仍不截断、仍不压列
        let huge = "￥9,999,999,999.99";
        let (pt, x2) = amount_pt_and_x(huge);
        assert!(
            (AMOUNT_PT_MIN..AMOUNT_PT).contains(&pt),
            "退让字号越界：{pt}"
        );
        assert!(text_width_mm(huge, pt) <= TCOL_AMOUNT - 2.0 + 0.01);
        assert!(x2 >= MARGIN + TCOL_NO + TCOL_DESC + TCOL_PARTY + 1.0);
    }

    #[test]
    fn daily_pagination_never_overflows() {
        // 40 行（含折行）贪心分页：每页内容 + 合计行预留都不得越过 TABLE_BOTTOM
        let cards: Vec<PdfCard> = (0..40)
            .map(|i| PdfCard {
                voucher_no: format!("20260820-{:03}", i + 1),
                voucher_date: "2026-08-20".into(),
                description: if i % 3 == 0 {
                    "八月办公用品采购第1批，含打印纸、墨盒与装订夹".into()
                } else {
                    "办公用品".into()
                },
                counterparty: "文一办公用品店".into(),
                amount_label: "￥123.45".into(),
                amount_cents: Some(12345),
                tags_line: String::new(),
            })
            .collect();
        let rows = layout_rows(&cards);
        assert_eq!(rows.len(), 40);
        let rows2 = rows.iter().filter(|r| r.desc2.is_some()).count();
        assert!(rows2 > 0, "长摘要应折成两行");
        for (first_top, next_top) in [
            (
                MARGIN + HEADER_H + C_BANNER_H + TSUM_GAP,
                MARGIN + HEADER_H + TSUM_GAP,
            ),
            (MARGIN + HEADER_H + TSUM_GAP, MARGIN + HEADER_H + TSUM_GAP),
        ] {
            let pages = paginate_rows(&rows, first_top, next_top);
            assert!(pages.len() >= 2);
            let mut covered = 0usize;
            for (pi, (a, b)) in pages.iter().enumerate() {
                assert!(b > a, "每页至少一行");
                let top = if pi == 0 { first_top } else { next_top };
                let content: f32 = rows[*a..*b].iter().map(|r| r.h).sum();
                assert!(
                    top + THEAD_H + content + TOTAL_LINE_H <= TABLE_BOTTOM + 0.001,
                    "第 {} 页越过 TABLE_BOTTOM",
                    pi + 1
                );
                covered += b - a;
            }
            assert_eq!(covered, rows.len(), "分页必须覆盖全部行");
        }
    }

    #[test]
    fn summary_wraps_to_two_lines() {
        let long = "八月办公用品采购第1批，含打印纸、墨盒与装订夹";
        let (l1, l2) = wrap_two_lines(long, BODY_PT, TCOL_DESC - 2.0);
        assert!(!l1.is_empty() && l2.is_some(), "长摘要必须折成两行");
        // 两行都不越列（估算宽 ≤ 列可用宽）
        assert!(text_width_mm(&l1, BODY_PT) <= TCOL_DESC - 2.0 + 0.01);
        assert!(text_width_mm(l2.as_deref().unwrap(), BODY_PT) <= TCOL_DESC - 2.0 + 0.01);
        // 短摘要不折行
        let (s1, s2) = wrap_two_lines("办公用品", BODY_PT, TCOL_DESC - 2.0);
        assert_eq!(s1, "办公用品");
        assert!(s2.is_none());
    }
}
