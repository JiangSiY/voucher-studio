//! G1 Spike A（打印 PoC）验证：printpdf 打印链路的中文 / A4 / 分页 / 无控件四项判定
//!
//! **2026-10-16（v0.22）校正**：卡片版式与「单份 PDF」已下线（日报成为唯一主线，
//! 方案 v2 批次①b），因此本文件的断言**全部改在按天日报版式上做**；
//! 原「3 列×7 行 = 21 张/页」的卡片分页断言随卡片版式一并删除。
//!
//! | 原标准 | 现在的验证方式 |
//! |---|---|
//! | ① 中文无豆腐块 | 嵌入 CJK 字体（`/FontFile2` 或 `/FontFile3`，随字体是 glyf 还是 CFF）+ 文本走 Identity-H CID 编码；+ 样张体积断言锁定「字体子集化」仍生效；样张人工复核 |
//! | ② A4 与 @page 边距 | 每页 MediaBox = 595.27563 × 841.88983 pt（210×297mm）；边距由绘制几何常量保证 |
//! | ③ 分页精确 | 按 v0.22 A1 版式（12pt、行高 8mm / 摘要折两行 12mm、贪心分页）算表页；图片页横图 2×2 → 4/页 |
//! | ④ 控制区不出现在输出 | PDF 由数据生成而非截屏；表格页内容流零位图 XObject |
//!
//! 样张与 `report.json` 落盘到 `spikes/print-poc/` 供人工复核（清单1 第 42 项、清单2 第 27 项引用）。

use std::path::{Path, PathBuf};

use voucher_core::services::pdf::{self, DailyInput, PdfCard, PdfImage, PdfOptions};

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

fn pdf_text(path: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned()
}

fn page_count(text: &str) -> usize {
    text.matches("MediaBox").count()
}

/// 样张 + 报告留档：44 张凭证（跨表页）+ 6 张图片（跨图片页），一次覆盖四项判定。
#[test]
fn write_poc_sample_and_report() {
    let dir = tempfile::tempdir().unwrap();
    let img_dir = dir.path().join("imgs");
    std::fs::create_dir_all(&img_dir).unwrap();
    let mut imgs = Vec::new();
    for i in 0..6 {
        let p = img_dir.join(format!("att-{i}.png"));
        make_png(&p);
        imgs.push(PdfImage {
            path: p,
            caption: format!(
                "附件{}：八月办公用品_文一办公用品店_108.07元（2026-08-20-{i:03}）",
                i + 1
            ),
        });
    }

    let out = dir.path().join("daily.pdf");
    pdf::generate_daily(
        &out,
        &DailyInput {
            date: "2026-08-20".into(),
            cards: (1..=44).map(card).collect(),
            images: imgs,
            appendices: Vec::new(),
        },
        &PdfOptions {
            title: "原始凭证".into(),
            include_images: true,
            daily_template: "compact".into(),
        },
    )
    .expect("generate daily pdf");

    let text = pdf_text(&out);
    let pages = page_count(&text);
    // printpdf 0.12.8 起字体程序按字体风味落盘：TrueType(glyf) → /FontFile2，
    // CFF（如 Hiragino Sans GB TTC）→ /FontFile3 + /CIDFontType0C。两者都算「字体已嵌入」。
    let font_embedded = text.matches("FontFile").count() >= 1;
    let cid = text.contains("/Encoding/Identity-H");
    let a4 = text
        .match_indices("MediaBox")
        .all(|(i, _)| text[i..i + 34].contains(A4_W_PT) && text[i..i + 34].contains(A4_H_PT));
    let raster = text.matches("/Subtype/Image").count();
    let pdf_bytes = std::fs::metadata(&out).unwrap().len();
    // 子集化前缀（如 `HEIGID+HiraginoSansGB-W3`）：printpdf 0.12 的 `subset_fonts`
    // 只在开了 `text_layout` feature 时才打这个 6 字母前缀，故它是最直接的特征探针。
    let font_name = text
        .match_indices("/FontName/")
        .map(|(i, _)| {
            text[i + "/FontName/".len()..]
                .split(['/', '>', ' ', '\n', '\r'])
                .next()
                .unwrap_or_default()
        })
        .find(|n| n.contains('+'))
        .unwrap_or_default()
        .to_string();

    // v0.22 A1 版式（摘要 24 字 → 折两行 12mm）：首页 16 行 + 次页 18 行 + 末页 10 行 = 3 个表页；
    // 6 张横图（96×72）走 2×2 → 2 个图片页；没有关联 → 无清单页；合计 5 页。
    assert_eq!(pages, 5, "表页 16+18+10 三页 + 图片 2×2 两页 = 5 页");
    assert!(
        font_embedded && cid,
        "判定①：CJK 字体嵌入 + Identity-H CID 编码"
    );
    // 判定①的子集化探针：整份字体入库时（printpdf 0.12 未开 text_layout feature 的行为）
    // 单是 Hiragino Sans GB 就要 ≈9.4MB/份；子集化后本样张（5 页、6 张 96×72 附件）< 1MB。
    assert!(
        pdf_bytes < 1_500_000,
        "判定①：样张 {pdf_bytes} 字节 —— 字体疑似未子集化（整字体入库约 9.4MB/份）"
    );
    assert!(
        !font_name.is_empty(),
        "判定①：字体名缺少子集化前缀（如 HEIGID+…），subset_fonts 未生效"
    );
    assert!(a4, "判定②：每页 MediaBox 均为 A4（210×297mm）");
    assert_eq!(
        raster, 6,
        "判定④：位图只出现在图片附录页（6 张附件各一次），表格页零位图"
    );

    // 样张与报告落盘到仓库（`spikes/print-poc/` 随仓库留档）
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let poc_dir = repo_root.join("spikes").join("print-poc");
    std::fs::create_dir_all(&poc_dir).unwrap();
    // 样张仅在未被阅读器锁定时才覆盖（判定已由上方断言完成）
    let _ = std::fs::copy(&out, poc_dir.join("sample-daily-report.pdf"));

    let report = serde_json::json!({
        "spike": "A-print",
        "date": "2026-10-16",
        "printpdf": "0.12.8（2026-10-18 自 0.7 升级，修 RUSTSEC-2026-0187；features = png/jpeg/text_layout，default-features = false）",
        "sample_bytes": pdf_bytes,
        "embedded_font": font_name,
        "approach": "printpdf (Rust backend, direct data-to-PDF) instead of WebView print",
        "layout": {
            "kind": "按天日报（v0.22 A1 版式）",
            "table": "正文/表头/金额/合计 12pt（小四）；列宽 编号 40 / 摘要 68 / 对方账户 42 / 金额独立预留 40mm；行高 8mm、摘要折两行 12mm；按行高贪心分页",
            "images": "横图 2×2（省纸模板竖图 3×3）；caption 10pt",
            "templates": "compact（省纸，默认）/ archive（账本式）"
        },
        "criteria": {
            "cjk_no_tofu": {
                "pass": font_embedded && cid,
                "evidence": format!("CJK 字体程序已嵌入（{font_name}）+ Identity-H CID 编码 + 子集化前缀 + 样张人工复核（PyMuPDF 提取 44/44 凭证号、摘要连贯、无 U+FFFD）"),
                "manual_review": "spikes/print-poc/sample-daily-report.pdf"
            },
            "a4_and_margins": {
                "pass": a4,
                "evidence": "every MediaBox = 595.27563 x 841.88983 pt (210x297mm), margins by geometry constants (10mm)"
            },
            "pagination_exact": {
                "pass": pages == 5,
                "evidence": "44 vouchers (wrapped summaries, 12mm rows) -> 16+18+10 rows = 3 table pages; 6 landscape images at 2x2 -> 2 image pages; total 5"
            },
            "no_controls_in_output": {
                "pass": raster == 6,
                "evidence": "bitmaps appear only on the image appendix pages; table pages contain zero Image XObjects (PDF generated from data, WebView DOM never involved)"
            }
        },
        "v022_note": "卡片版式与「单份 PDF」于 2026-10-16 下线（方案 v2 批次①b）：本报告只覆盖日报版式；几何不变量另由 pdf.rs 单测（daily_layout_invariants / daily_pagination_never_overflows / summary_wraps_to_two_lines）锁定",
        "platforms": {
            "windows": "historical: verified under v0.6 (automated + manual sample); Windows support removed in v0.17.0 — see docs/开发日志.md",
            "macos": "automated assertions pass on macOS (arm64); manual sample walkthrough pending (see 挂账). Font candidates include PingFang/Hiragino/STHeiti/Songti"
        },
        "known_tradeoff": format!("字体已启用子集化（0.7 用 font_subsetting feature；0.12.8 改为 PdfSaveOptions::subset_fonts，且**必须**开 text_layout feature 才真生效，见 crates/voucher-core/Cargo.toml 注释）：本样张 {pdf_bytes} B；未子集化时整份 Hiragino Sans GB 约 9.4MB/份（2026-09-23 之前的状态）")
    });
    std::fs::write(
        poc_dir.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();
}
