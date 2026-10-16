//! G1 Spike A（打印 PoC）验证：printpdf 打印链路的 4 项判定标准
//!
//! 原判定标准按「WebView 打印」设计；实际实现定稿为 Rust 后端 printpdf 直接从数据生成 PDF
//! （打印中心 → export 命令 → pdf::generate），不经过 WebView DOM。4 项标准映射如下：
//!
//! | 原标准 | 映射后的验证方式 |
//! |---|---|
//! | ① 中文无豆腐块 | PDF 嵌入 CJK 字体文件（FontFile2）且文本走 Identity-H CID 编码；样张人工复核 |
//! | ② A4 纸张与 @page 边距生效 | 每页 MediaBox = 595.27563 × 841.88983 pt（210×297mm）；边距由绘制几何常量保证 |
//! | ③ 每页 4 张卡片分页精确 | 按定稿布局（3 列×7 行 = 21 张/页）断言：N 张卡片 → ceil(N/21) 页；图片附录 ceil(M/4) 页 |
//! | ④ 控制区不出现在打印预览 | 架构性保证：PDF 由数据生成而非截屏。断言卡片页内容流零位图 XObject（纯矢量+文本） |
//!
//! 样张输出到 `spikes/print-poc/` 供人工打开复核（判定①的最终确认）。

use std::path::{Path, PathBuf};

use voucher_core::services::pdf::{self, DailyInput, PdfCard, PdfImage, PdfInput, PdfOptions};

/// A4 纵向的 PDF 点值（printpdf 的 Mm(210)/Mm(297) 换算结果）
const A4_W_PT: &str = "595.27563";
const A4_H_PT: &str = "841.88983";

fn card(n: usize) -> PdfCard {
    PdfCard {
        voucher_no: format!("20260820-{n:03}"),
        voucher_date: "2026-08-20".into(),
        description: format!("八月办公用品采购第{n}批，含打印纸、墨盒与装订夹"),
        counterparty: if n.is_multiple_of(2) {
            "文一办公用品店".into()
        } else {
            String::new()
        },
        amount_label: if n.is_multiple_of(3) {
            String::new() // 留金额为空的卡片分支
        } else {
            format!("{}.{}", 100 + n, (n * 7) % 100)
        },
        amount_cents: if n.is_multiple_of(3) {
            None
        } else {
            Some((100 + n as i64) * 100 + (n as i64 * 7) % 100)
        },
        tags_line: if n.is_multiple_of(2) {
            "办公 · 报销".into()
        } else {
            String::new()
        },
    }
}

fn make_png(path: &Path) {
    let img = image::RgbImage::from_pixel(96, 72, image::Rgb([160u8, 90, 60]));
    img.save(path).expect("save png");
}

fn generate(dir: &Path, name: &str, cards: usize, images: usize, include_images: bool) -> PathBuf {
    let img_dir = dir.join("imgs");
    std::fs::create_dir_all(&img_dir).unwrap();
    let mut imgs = Vec::new();
    for i in 0..images {
        let p = img_dir.join(format!("att-{i}.png"));
        make_png(&p);
        imgs.push(PdfImage {
            path: p,
            caption: format!("20260820-00{i} att-{i}.png"),
        });
    }
    let out = dir.join(name);
    pdf::generate(
        &out,
        &PdfInput {
            cards: (1..=cards).map(card).collect(),
            images: imgs,
        },
        &PdfOptions {
            title: "凭证整理单".into(),
            card_style: "standard".into(),
            columns: 3,
            include_images,
            daily_template: "compact".into(),
        },
    )
    .expect("generate pdf");
    out
}

fn pdf_text(path: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned()
}

fn page_count(text: &str) -> usize {
    text.matches("MediaBox").count()
}

/// 标准 ② + ③：A4 几何精确、卡片/图片页分页精确
#[test]
fn a4_geometry_and_pagination_exact() {
    let dir = tempfile::tempdir().unwrap();

    // 25 张卡片（>21，跨页）+ 5 张图片（>4，附录跨页）
    let out = generate(dir.path(), "full.pdf", 25, 5, true);
    let text = pdf_text(&out);

    // ② 全部页面 MediaBox = A4 纵向（210×297mm）
    let boxes: Vec<&str> = text
        .match_indices("MediaBox")
        .map(|(i, _)| &text[i..i + 34])
        .collect();
    assert!(!boxes.is_empty(), "应至少存在一页");
    for b in &boxes {
        assert!(
            b.contains(A4_W_PT) && b.contains(A4_H_PT),
            "页面尺寸非 A4 纵向: {b}"
        );
    }

    // ③ 分页精确：卡片页 ceil(25/21)=2 + 图片附录页 ceil(5/4)=2 → 共 4 页
    assert_eq!(page_count(&text), 4, "25 卡片+5 图片应为 2+2=4 页");

    // 恰好一页（21 张）不产生第二页
    let one = generate(dir.path(), "one.pdf", 21, 0, false);
    assert_eq!(page_count(&pdf_text(&one)), 1, "21 张卡片恰好 1 页");

    // 空白卡片边界：1 张也成页
    let single = generate(dir.path(), "single.pdf", 1, 0, false);
    assert_eq!(page_count(&pdf_text(&single)), 1);
}

/// 标准 ①：中文以嵌入 CJK 字体渲染（非查看器默认字体替换 → 无豆腐块的前提）
#[test]
fn cjk_font_embedded_with_cid_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let out = generate(dir.path(), "font.pdf", 3, 0, false);
    let text = pdf_text(&out);

    // 嵌入 TrueType 字体文件（豆腐块的根因是查看器找不到字形只能画方块；
    // 字体文件随 PDF 走，字形必然存在）
    assert!(
        text.matches("FontFile2").count() >= 1,
        "PDF 应嵌入 TrueType 字体文件"
    );
    // 文本走 Identity-H CID 横排编码（CID 键控的中日韩文本标准编码）
    assert!(
        text.contains("/Encoding/Identity-H"),
        "文本应使用 Identity-H CID 编码"
    );
    // 字体源来自 CJK 候选列表（simhei/msyh/simsun/PingFang/…）
    assert!(
        pdf::find_cjk_font().is_some(),
        "系统应存在可用的 CJK 字体候选"
    );
}

/// 标准 ④：卡片页零位图（PDF 由数据直接生成，不存在「屏幕 UI 进打印输出」的通道；
/// 断言卡片页内容流不含任何 Image XObject——控制区若混入必然以位图/绘制指令出现）
#[test]
fn card_pages_contain_no_raster_content() {
    let dir = tempfile::tempdir().unwrap();

    // 纯卡片 PDF：全文档不允许出现位图 XObject
    let cards_only = generate(dir.path(), "cards.pdf", 22, 0, false);
    let text = pdf_text(&cards_only);
    assert_eq!(page_count(&text), 2, "22 张卡片应为 2 页（先确认分页正常）");
    assert_eq!(
        text.matches("/Subtype/Image").count(),
        0,
        "卡片页不应含位图内容（控制区/UI 元素混入的标志）"
    );

    // 对照组：带图片附录的 PDF 必须出现位图 XObject（证明检测手段有效，不是恒假）
    let with_imgs = generate(dir.path(), "with-imgs.pdf", 22, 2, true);
    assert!(
        pdf_text(&with_imgs).matches("/Subtype/Image").count() >= 2,
        "图片附录页应含位图 XObject（检测有效性对照）"
    );
}

/// v0.5 按天日报（档案装订形态）：P1 封面 + P2 汇总表 + P3+ 图片页；
/// 封面/汇总表为纯矢量文本，位图只出现在图片附录页。样张留档人工复核。
#[test]
fn daily_report_cover_layout() {
    let dir = tempfile::tempdir().unwrap();
    let img_dir = dir.path().join("imgs");
    std::fs::create_dir_all(&img_dir).unwrap();
    let mut imgs = Vec::new();
    for i in 0..4 {
        let p = img_dir.join(format!("att-{i}.png"));
        make_png(&p);
        imgs.push(PdfImage {
            path: p,
            caption: format!(
                "附件{}：八月办公用品_文一办公用品店_108.07元（2026-08-20-0{i}）",
                i + 1
            ),
        });
    }
    let out = dir.path().join("daily.pdf");
    pdf::generate_daily(
        &out,
        &DailyInput {
            date: "2026-08-20".into(),
            cards: (1..=5).map(card).collect(),
            images: imgs,
            appendices: Vec::new(),
        },
        &PdfOptions {
            title: "原始凭证".into(),
            card_style: "standard".into(),
            columns: 3,
            include_images: true,
            daily_template: "compact".into(),
        },
    )
    .expect("generate daily pdf");

    let text = pdf_text(&out);
    // v0.8 compact 模板：5 卡片 → 首页（横幅+汇总表）1 + 图片页 ceil(4/4)=1 → 共 2 页
    // （archive 模板为封面+汇总表+图片页 3 页）
    assert_eq!(
        page_count(&text),
        2,
        "5 卡片+4 图片 = 首页横幅+表格 1 + 图片页 1 = 2 页"
    );
    // 位图只出现在图片附录页（首页纯矢量）→ 恰好 4 个 Image XObject
    assert_eq!(
        text.matches("/Subtype/Image").count(),
        4,
        "位图应只出现在图片附录页"
    );

    // 样张留档人工复核
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let poc_dir = repo_root.join("spikes").join("print-poc");
    std::fs::create_dir_all(&poc_dir).unwrap();
    let _ = std::fs::copy(&out, poc_dir.join("sample-daily-report.pdf"));
}

/// 生成人工复核样张 + 报告（判定①的最终确认：打开 PDF 肉眼检查中文渲染）
#[test]
fn write_poc_sample_and_report() {
    let dir = tempfile::tempdir().unwrap();
    let out = generate(dir.path(), "sample.pdf", 44, 6, true);
    let text = pdf_text(&out);
    let pages = page_count(&text);
    let font_embedded = text.matches("FontFile2").count() >= 1;
    let cid = text.contains("/Encoding/Identity-H");
    let a4 = text
        .match_indices("MediaBox")
        .all(|(i, _)| text[i..i + 34].contains(A4_W_PT) && text[i..i + 34].contains(A4_H_PT));
    let cards_only = generate(dir.path(), "probe-cards.pdf", 22, 0, false);
    let no_raster = pdf_text(&cards_only).matches("/Subtype/Image").count() == 0;

    assert_eq!(
        pages, 5,
        "44 卡片 ceil(44/21)=3 页 + 6 图片 ceil(6/4)=2 页 = 5 页"
    );

    // 样张与报告落盘到仓库（.gitignore 不含 spikes，随仓库留档）
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let poc_dir = repo_root.join("spikes").join("print-poc");
    std::fs::create_dir_all(&poc_dir).unwrap();
    // 样张仅留档：被 PDF 阅读器锁定时跳过覆盖（判定已由上方断言完成）
    let _ = std::fs::copy(&out, poc_dir.join("sample-voucher-sheet.pdf"));

    let report = serde_json::json!({
        "spike": "A-print",
        "date": "2026-08-20",
        "approach": "printpdf (Rust backend, direct data-to-PDF) instead of WebView print",
        "criteria": {
            "cjk_no_tofu": {
                "pass": font_embedded && cid,
                "evidence": "FontFile2 embedded + Identity-H CID encoding + manual sample review",
                "manual_review": "spikes/print-poc/sample-voucher-sheet.pdf"
            },
            "a4_and_margins": {
                "pass": a4,
                "evidence": "every MediaBox = 595.27563 x 841.88983 pt (210x297mm), margins by geometry constants (10mm)"
            },
            "pagination_exact": {
                "pass": pages == 5,
                "evidence": "44 cards -> ceil(44/21)=3 pages, 6 images -> ceil(6/4)=2 pages, total 5 pages"
            },
            "no_controls_in_output": {
                "pass": no_raster,
                "evidence": "card pages contain zero Image XObjects; PDF generated from data, WebView DOM never involved"
            }
        },
        "layout_note": "original Spike A criterion assumed WebView print at 4 cards/page; final layout is 3 cols x 7 rows = 21 cards/page (ink-saving design per UI scheme). Criterion semantics = exact pagination, verified against final layout.",
        "platforms": {
            "windows": "historical: verified under v0.6 (automated + manual sample); Windows support removed in v0.17.0 — see docs/开发日志.md",
            "macos": "automated assertions pass on macOS (arm64); manual sample walkthrough still pending (see 挂账). Font candidates include PingFang/Hiragino/STHeiti/Songti, re-verify per risk register item 'Spike conclusion expiry'"
        },
        "known_tradeoff": "字体已启用子集化（printpdf font_subsetting，2026-09-23 起）：样张约 170-230 KB；此前的『嵌入整字体 ~10MB/份』问题已消除"
    });
    std::fs::write(
        poc_dir.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();
}
