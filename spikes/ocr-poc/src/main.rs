//! G1 Spike B：本地 OCR（ort + RapidOCR PP-OCRv4）可行性验证
//!
//! 对 testdata/ 下 ≥20 张凭证图推理，与 ground_truth.json 比对：
//! - 字符级准确率：1 - 编辑距离 / 标准答案长度（去空白后整体比对）
//! - 字段级准确率：标准答案每一行，能被「单行识别结果」或「同行拼接」精确命中的比例
//!
//! 判定阈值（对齐开发方案）：字符级准确率 ≥ 95%（即误差 ≤ 5%）
//!
//! 用法（在项目根目录）：
//!   cargo run -p ocr-poc -- [--models assets/ocr-models] [--data spikes/ocr-poc/testdata]
//!
//! **已退役（2026-10-26，第六十七笔）**：v0.23 把本地 OCR 从 `ort` + PP-OCRv5 换成 Apple Vision，
//! `services/ocr_local.rs` 与 `assets/ocr-models/` 已删除，因此本 spike **不再随工作区编译**——
//! 它记录的 G1 证据（`report.json`：字符级 99.90% / 字段级 99.07%）仍保留，需要复现时取
//! git 历史中 v0.22.5 及之前的版本。
//!                          [--report spikes/ocr-poc/report.json] [--verbose]

use std::path::PathBuf;
use std::time::Instant;

use voucher_core::services::ocr_local::LocalOcr;

#[derive(serde::Deserialize)]
struct GtImage {
    file: String,
    lines: Vec<String>,
}

#[derive(serde::Deserialize)]
struct Gt {
    images: Vec<GtImage>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageReport {
    file: String,
    ms: u128,
    char_accuracy: f64,
    field_accuracy: f64,
    fields_total: usize,
    fields_hit: usize,
    recognized: Vec<String>,
    ground_truth: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    images: Vec<ImageReport>,
    total_images: usize,
    avg_char_accuracy: f64,
    avg_field_accuracy: f64,
    total_ms: u128,
    pass: bool,
    threshold: f64,
}

fn normalize(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let (m, n) = (a.len(), b.len());
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut cur = vec![0usize; n + 1];
    for i in 1..=m {
        cur[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[n]
}

/// 把识别行聚成「视觉行」：按 y 中心聚类（差 < 12px 同行），组内按 x 从左到右拼接
/// （双列布局的标签+值归一行，且保持「标签在前、值在后」的阅读顺序）
fn group_rows(lines: &[voucher_core::services::ocr_local::OcrLine]) -> Vec<String> {
    let mut sorted: Vec<_> = lines.iter().collect();
    // 先按 y 中心排序，保证同行元素相邻
    sorted.sort_by(|a, b| {
        let (ay, by) = (
            (a.box_xyxy[1] + a.box_xyxy[3]) / 2.0,
            (b.box_xyxy[1] + b.box_xyxy[3]) / 2.0,
        );
        ay.partial_cmp(&by).unwrap()
    });
    let mut groups: Vec<(f32, Vec<&voucher_core::services::ocr_local::OcrLine>)> = Vec::new();
    for l in sorted {
        let y = (l.box_xyxy[1] + l.box_xyxy[3]) / 2.0;
        match groups.iter_mut().find(|(gy, _)| (gy - y).abs() < 12.0) {
            Some((_, v)) => v.push(l),
            None => groups.push((y, vec![l])),
        }
    }
    groups
        .iter()
        .map(|(_, v)| {
            let mut v = v.clone();
            v.sort_by(|a, b| a.box_xyxy[0].partial_cmp(&b.box_xyxy[0]).unwrap());
            v.iter().map(|l| normalize(&l.text)).collect::<String>()
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut models = PathBuf::from("assets/ocr-models");
    let mut data = PathBuf::from("spikes/ocr-poc/testdata");
    let mut report_path = PathBuf::from("spikes/ocr-poc/report.json");
    let mut verbose = false;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--models" => models = PathBuf::from(args.next().unwrap()),
            "--data" => data = PathBuf::from(args.next().unwrap()),
            "--report" => report_path = PathBuf::from(args.next().unwrap()),
            "--verbose" => verbose = true,
            other => return Err(format!("未知参数: {other}").into()),
        }
    }

    println!("== G1 Spike B：本地 OCR 可行性验证 ==");
    println!("模型目录: {}", models.display());
    println!("测试数据: {}", data.display());

    let t0 = Instant::now();
    let mut engine = LocalOcr::load(&models)?;
    println!(
        "引擎加载完成（词表 {} 字符，耗时 {}ms）",
        engine.dict_len(),
        t0.elapsed().as_millis()
    );

    let gt_raw = std::fs::read_to_string(data.join("ground_truth.json"))?;
    // PowerShell 生成可能带 BOM，serde_json 不接受
    let gt: Gt = serde_json::from_str(gt_raw.trim_start_matches('\u{feff}'))?;
    println!("测试图片: {} 张\n", gt.images.len());

    let mut reports = Vec::new();
    let mut sum_char = 0.0;
    let mut sum_field = 0.0;
    let mut total_ms = 0u128;

    for g in &gt.images {
        let img = image::open(data.join(&g.file))?;
        let t = Instant::now();
        let lines = engine.recognize_image(&img)?;
        let ms = t.elapsed().as_millis();
        total_ms += ms;

        // 字符级：整图去空白后编辑距离
        let rec_all: String = lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        let gt_all: String = g.lines.join("");
        let (a, b): (Vec<char>, Vec<char>) = (
            normalize(&rec_all).chars().collect(),
            normalize(&gt_all).chars().collect(),
        );
        let dist = levenshtein(&a, &b);
        let char_acc = if b.is_empty() {
            1.0
        } else {
            1.0 - dist as f64 / b.len() as f64
        };

        // 字段级：GT 行 == 任一识别行（单行）或任一视觉行（同行拼接）
        let rec_lines_norm: Vec<String> = lines
            .iter()
            .map(|l| normalize(&l.text))
            .filter(|s| !s.is_empty())
            .collect();
        let rows = group_rows(&lines);
        let mut hit = 0;
        let mut fields = 0;
        for l in &g.lines {
            let nl = normalize(l);
            if nl.is_empty() {
                continue;
            }
            fields += 1;
            if rec_lines_norm.iter().any(|r| r == &nl) || rows.contains(&nl) {
                hit += 1;
            } else if verbose {
                println!("    MISS [{}]: {nl}", g.file);
            }
        }
        let field_acc = hit as f64 / fields.max(1) as f64;

        sum_char += char_acc;
        sum_field += field_acc;
        reports.push(ImageReport {
            file: g.file.clone(),
            ms,
            char_accuracy: char_acc,
            field_accuracy: field_acc,
            fields_total: fields,
            fields_hit: hit,
            recognized: lines.iter().map(|l| l.text.clone()).collect(),
            ground_truth: g.lines.clone(),
        });

        println!(
            "{:<9} 字符 {:>6.2}%  字段 {:>6.2}%  ({:>3}/{:>3})  {:>5}ms",
            g.file,
            char_acc * 100.0,
            field_acc * 100.0,
            hit,
            fields,
            ms
        );
        if verbose {
            for l in &lines {
                println!(
                    "    [score {:.2}] y=({:.0},{:.0}) {}",
                    l.score, l.box_xyxy[1], l.box_xyxy[3], l.text
                );
            }
            println!("    -- 行分组 --");
            for r in &rows {
                println!("    ROW [{}]", r);
            }
        }
    }

    let n = gt.images.len().max(1) as f64;
    let avg_char = sum_char / n;
    let avg_field = sum_field / n;
    let threshold = 0.95;
    let pass = avg_char >= threshold;

    println!("\n---- 汇总 ----");
    println!("平均字符级准确率: {:.2}%  (阈值 95%)", avg_char * 100.0);
    println!("平均字段级准确率: {:.2}%", avg_field * 100.0);
    println!(
        "总耗时: {}ms (平均 {}ms/张)",
        total_ms,
        total_ms / (gt.images.len().max(1) as u128)
    );
    println!("G1 门判定: {}", if pass { "通过 ✅" } else { "不通过 ❌" });

    let report = Report {
        images: reports,
        total_images: gt.images.len(),
        avg_char_accuracy: avg_char,
        avg_field_accuracy: avg_field,
        total_ms,
        pass,
        threshold,
    };
    std::fs::create_dir_all(report_path.parent().unwrap())?;
    std::fs::write(&report_path, serde_json::to_string_pretty(&report)?)?;
    println!("报告已写入: {}", report_path.display());

    if !pass {
        std::process::exit(1);
    }
    Ok(())
}
