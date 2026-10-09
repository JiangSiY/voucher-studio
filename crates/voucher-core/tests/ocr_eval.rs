//! OCR 评测 harness（**可选，环境变量门控**）：在真实凭证基准集上量自动填写准确度。
//!
//! 用法：
//! ```text
//! VOUCHER_OCR_EVAL=~/Downloads/开发/voucher-ocr-eval/2026-09 \
//!   cargo test -p voucher-core --release --test ocr_eval -- --nocapture
//! ```
//!
//! 基准集（仓库外，真实票据，勿入库）：目录下有 `images/` 与 `manifest.json`，
//! 每张图带 `expected`（日期/金额分/对方账户）+ `flags`。答案 = 用户核过的账面。
//!
//! 跑的是**生产同款链路**：`ingestion::import_images(run_ocr=true)` →
//! 增强（OpenCV 七步）→ 本地 PP-OCRv5 → 抽取（有版面走几何配对，否则纯文本）→
//! `apply_fields` 门控自动填写。因此测的是端到端结果，不是实验室里的抽取函数。
//!
//! 报告两层口径：
//! - **抽取层**（原始结果，不看门控）：金额/日期/对方的 命中 / 填错 / 漏掉
//! - **落库层**（过门控后进了凭证的值）：同上——金额的「填错」是核心 KPI（宁空勿错）
//!
//! **为什么环境变量门控**：与 perf_bench 同理——跑一遍要几分钟且依赖本机模型与基准集，
//! 不能在 CI 默认执行（标 `#[ignore]` 会被 CI 的 `--include-ignored` 真跑）。

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::*;
use serde::Deserialize;
use voucher_core::services::ocr_fields::Confidence;
use voucher_core::services::ocr_layout::DocKind;
use voucher_core::services::{ingestion, ocr_fields, ocr_layout, voucher_mgr};
use voucher_core::Result;

#[derive(Deserialize)]
struct Manifest {
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    id: i64,
    image: Option<String>,
    expected: Expected,
}

#[derive(Deserialize)]
struct Expected {
    voucher_date: String,
    amount_cents: Option<i64>,
    counterparty: String,
}

/// 单字段三分类计数
#[derive(Default, Clone, Copy)]
struct Tally {
    hit: usize,
    wrong: usize,
    miss: usize,
}

impl Tally {
    fn add(&mut self, got: Option<&str>, want: Option<&str>) {
        match (got, want) {
            (Some(g), Some(w)) if g == w => self.hit += 1,
            (Some(_), Some(_)) => self.wrong += 1,
            (None, Some(_)) => self.miss += 1,
            _ => {} // 基准无此字段：不评
        }
    }
    fn line(&self, name: &str) -> String {
        let total = self.hit + self.wrong + self.miss;
        let pct = |n: usize| {
            if total == 0 {
                0.0
            } else {
                n as f64 * 100.0 / total as f64
            }
        };
        format!(
            "  {name:<6} 命中 {}/{}（{:.0}%）｜填错 {}（{:.1}%）｜漏掉 {}（{:.0}%）",
            self.hit,
            total,
            pct(self.hit),
            self.wrong,
            pct(self.wrong),
            self.miss,
            pct(self.miss)
        )
    }
}

#[test]
fn eval_ocr_autofill_accuracy() -> Result<()> {
    let Some(eval_dir) = std::env::var("VOUCHER_OCR_EVAL")
        .ok()
        .filter(|v| !v.is_empty())
    else {
        println!("跳过 OCR 评测（未设 VOUCHER_OCR_EVAL；指向基准集目录启用）");
        return Ok(());
    };
    let dir = PathBuf::from(eval_dir);
    let manifest_path = dir.join("manifest.json");
    if !manifest_path.is_file() {
        println!("跳过：找不到 {}", manifest_path.display());
        return Ok(());
    }

    // 模型前置（仓库自带 assets/ocr-models）
    let models = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ocr-models");
    if !voucher_core::LocalOcr::is_installed(&models) {
        println!("跳过：本地 OCR 模型不存在（{}）", models.display());
        return Ok(());
    }

    let manifest: Manifest =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("读 manifest.json"))
            .expect("解析 manifest.json");

    let (tdir, ctx) = test_ctx();
    assert!(
        voucher_core::LocalOcr::ensure_installed(&models, &ctx.paths.models)?,
        "临时库应装载模型"
    );

    let mut raw_amount = Tally::default();
    let mut raw_date = Tally::default();
    let mut raw_cp = Tally::default();
    let mut raw_cp_near = 0usize; // 近似命中（一方包含另一方：全称 vs 简写）
    let mut filled_amount = Tally::default();
    let mut filled_date = Tally::default();
    let mut filled_cp = Tally::default();
    let mut filled_cp_near = 0usize;
    let mut done = 0usize;
    let mut skipped = 0usize;
    let mut wrong_amount_cases: Vec<String> = Vec::new();
    let mut raw_wrong_amount_cases: Vec<String> = Vec::new();
    let mut wrong_date_cases: Vec<String> = Vec::new();
    let mut wrong_cp_cases: Vec<String> = Vec::new();
    let mut low_amount_samples: Vec<String> = Vec::new();
    // 凭证类型 → (张数, 金额命中, 金额错, 日期命中, 对方命中)——④版式模板的决策依据
    let mut kind_stats: BTreeMap<&'static str, (usize, usize, usize, usize, usize)> =
        BTreeMap::new();
    let mut unknown_samples: Vec<String> = Vec::new();
    // 抽取层金额置信档 →（对, 错）——看「拦下的都是什么档」，为提档找方向
    let mut amount_conf_dist: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    // 导入默认 = 今天（未填则保持）——落库层凭此区分「没填」与「填错」
    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    for (n, e) in manifest.entries.iter().enumerate() {
        let Some(img_name) = &e.image else {
            skipped += 1;
            continue;
        };
        let src = dir.join("images").join(img_name);
        if !src.is_file() {
            skipped += 1;
            continue;
        }
        // 拷进临时目录再导入（不改动基准集；导入链路会做增强/识别/填单）
        let dst = tdir.path().join(format!(
            "{}.{}",
            e.id,
            img_name.rsplit('.').next().unwrap_or("jpg")
        ));
        std::fs::copy(&src, &dst)?;

        let summary = ingestion::import_images(&ctx, &[dst], None, true)?;
        let Some(v) = summary.vouchers.first() else {
            println!("  ⚠ 凭证 #{} 导入失败（{}）", e.id, img_name);
            skipped += 1;
            continue;
        };
        let detail = voucher_mgr::get_detail(&ctx, v.voucher_id)?;
        let voucher = &detail.voucher;

        // —— 抽取层诊断：对落库的识别文本重跑抽取（与填单时刻同口径）——
        let att = &detail.attachments[0];
        let text = att.ocr_text.as_deref().unwrap_or("");
        let fields = match ctx
            .db
            .with_conn(|conn| voucher_core::db::repos::ocr_layout_repo::get_layout(conn, att.id))?
        {
            Some(lines) if !lines.is_empty() => ocr_layout::extract_with_layout(&lines),
            _ => ocr_fields::extract_validated(text),
        };

        let exp_amt = e.expected.amount_cents.map(|c| c.to_string());
        let exp_date =
            (!e.expected.voucher_date.is_empty()).then(|| e.expected.voucher_date.clone());
        let exp_cp = (!e.expected.counterparty.is_empty()).then(|| e.expected.counterparty.clone());

        raw_amount.add(
            fields.amount_cents.map(|c| c.to_string()).as_deref(),
            exp_amt.as_deref(),
        );
        if let (Some(got), Some(want)) = (fields.amount_cents, e.expected.amount_cents) {
            if got != want {
                raw_wrong_amount_cases.push(format!(
                    "#{} 期望 {:.2} 抽出 {:.2}｜conf {:?} source {:?}",
                    e.id,
                    want as f64 / 100.0,
                    got as f64 / 100.0,
                    fields.confidence.amount,
                    fields.amount_source,
                ));
            }
        }
        raw_date.add(fields.date.as_deref(), exp_date.as_deref());
        if let (Some(got), Some(want)) = (fields.amount_cents, e.expected.amount_cents) {
            let slot = amount_conf_dist
                .entry(format!("{:?}", fields.confidence.amount))
                .or_default();
            if got == want {
                slot.0 += 1;
            } else {
                slot.1 += 1;
            }
        }
        raw_cp.add(fields.counterparty.as_deref(), exp_cp.as_deref());
        if let (Some(got), Some(want)) = (&fields.counterparty, &exp_cp) {
            let (g, w) = (got.trim(), want.trim());
            if g != w && (g.contains(w) || w.contains(g)) {
                raw_cp_near += 1;
            }
        }

        // 落库层：默认值（今天 / 空串）视为「未填」——不算填错
        let filled_date_got = (voucher.voucher_date != today).then(|| voucher.voucher_date.clone());
        let filled_cp_got =
            (!voucher.counterparty.is_empty()).then(|| voucher.counterparty.clone());
        filled_amount.add(
            voucher.amount_cents.map(|c| c.to_string()).as_deref(),
            exp_amt.as_deref(),
        );
        filled_date.add(filled_date_got.as_deref(), exp_date.as_deref());
        filled_cp.add(filled_cp_got.as_deref(), exp_cp.as_deref());
        if let (Some(got), Some(want)) = (&filled_cp_got, &exp_cp) {
            let (g, w) = (got.trim(), want.trim());
            if g != w && (g.contains(w) || w.contains(g)) {
                filled_cp_near += 1;
            }
        }

        // 金额落库错误 = 最严重个案：逐条点名（连同抽取值/置信度/金额来源）
        if let (Some(got), Some(want)) = (voucher.amount_cents, e.expected.amount_cents) {
            if got != want {
                wrong_amount_cases.push(format!(
                    "#{} 期望 {:.2} 实落 {:.2}｜抽取 {:?} conf {:?} source {:?}",
                    e.id,
                    want as f64 / 100.0,
                    got as f64 / 100.0,
                    fields.amount_cents,
                    fields.confidence.amount,
                    fields.amount_source,
                ));
            }
        }
        // 日期落库「填了但不对」个案（排除未填）——看是认错还是口径差异
        if let (Some(got), Some(want)) = (&filled_date_got, &exp_date) {
            if got != want {
                wrong_date_cases.push(format!(
                    "#{} 期望 {} 实落 {}｜抽取 {:?} conf {:?}",
                    e.id, want, got, fields.date, fields.confidence.date,
                ));
            }
        }
        // 对方落库错例逐条留样（判断是「抓错字符串」还是「口径不同」）
        if let (Some(got), Some(want)) = (&filled_cp_got, &exp_cp) {
            if got != want {
                wrong_cp_cases.push(format!(
                    "#{} 期望「{}」实填「{}」｜conf {:?}",
                    e.id, want, got, fields.confidence.counterparty,
                ));
            }
        }
        // 被门控拦下的「唯一兜底」低档金额：留几例文本片段看是什么票
        if let Some(c) = fields.amount_cents {
            if matches!(fields.confidence.amount, Some(Confidence::Low))
                && Some(c) == e.expected.amount_cents
                && low_amount_samples.len() < 5
            {
                let snippet: String = text.chars().take(70).collect();
                low_amount_samples.push(format!(
                    "#{} 金额 {:.2}｜文本前 70 字：{}",
                    e.id,
                    c as f64 / 100.0,
                    snippet.replace('\n', " ／ ")
                ));
            }
        }

        // 类型统计（④版式模板决策依据）：金额/日期/对方 命中共用抽取层口径
        let kind = ocr_layout::classify_text(text);
        let slot = kind_stats.entry(kind.label()).or_default();
        slot.0 += 1;
        if let (Some(got), Some(want)) = (fields.amount_cents, e.expected.amount_cents) {
            if got == want {
                slot.1 += 1;
            } else {
                slot.2 += 1;
            }
        }
        if let (Some(got), Some(want)) = (fields.date.as_deref(), exp_date.as_deref()) {
            if got == want {
                slot.3 += 1;
            }
        }
        if let (Some(got), Some(want)) = (fields.counterparty.as_deref(), exp_cp.as_deref()) {
            if got == want {
                slot.4 += 1;
            }
        }
        if matches!(kind, DocKind::Unknown) && unknown_samples.len() < 8 {
            let snippet: String = text.chars().take(50).collect();
            unknown_samples.push(format!("#{}｜{}", e.id, snippet.replace('\n', " ／ ")));
        }

        done += 1;
        if (n + 1) % 10 == 0 {
            println!("  … 已评测 {}", n + 1);
        }
    }

    println!("\n=== OCR 评测（{} 张参与 / {} 张跳过）===", done, skipped);
    println!("抽取层（未过门控的原始结果）：");
    println!("{}", raw_amount.line("金额"));
    println!("{}", raw_date.line("日期"));
    println!("{}", raw_cp.line("对方"));
    println!("  （对方近似命中 {raw_cp_near} 例：全称 vs 简写，一方包含另一方）");
    println!("抽取层金额置信档分布（对/错）：");
    for (conf, (ok, bad)) in &amount_conf_dist {
        println!("  {conf}: 对 {ok} 错 {bad}");
    }
    println!("落库层（过门控自动填写后；未填不算错）：");
    println!("{}", filled_amount.line("金额"));
    println!("{}", filled_date.line("日期"));
    println!("{}", filled_cp.line("对方"));
    println!("  （对方近似命中 {filled_cp_near} 例）");
    println!("凭证类型分布（按张数降序；金额命中=抽取层与基准一致）：");
    let mut kinds: Vec<_> = kind_stats.iter().collect();
    kinds.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    for (k, (n, amt_ok, amt_bad, date_ok, cp_ok)) in kinds {
        println!(
            "  {k:<8} {n} 张｜金额 {amt_ok}/{}（错 {amt_bad}）｜日期命中 {date_ok}｜对方命中 {cp_ok}",
            amt_ok + amt_bad
        );
    }
    if !unknown_samples.is_empty() {
        println!("未识别类型样本（看分类规则要不要补关键词）：");
        for c in &unknown_samples {
            println!("  {c}");
        }
    }
    if wrong_amount_cases.is_empty() {
        println!("金额落库错误：0 例 ✅（宁空勿错成立）");
    } else {
        println!("金额落库错误 {} 例：", wrong_amount_cases.len());
        for c in &wrong_amount_cases {
            println!("  {c}");
        }
    }
    if !raw_wrong_amount_cases.is_empty() {
        println!(
            "抽取层金额认错 {} 例（被门控拦下未落库）：",
            raw_wrong_amount_cases.len()
        );
        for c in &raw_wrong_amount_cases {
            println!("  {c}");
        }
    }
    if !wrong_date_cases.is_empty() {
        println!("日期落库「填了但不对」{} 例：", wrong_date_cases.len());
        for c in &wrong_date_cases {
            println!("  {c}");
        }
    }
    if !wrong_cp_cases.is_empty() {
        println!(
            "对方落库错误 {} 例（看是抓错字符串还是口径不同）：",
            wrong_cp_cases.len()
        );
        for c in &wrong_cp_cases {
            println!("  {c}");
        }
    }
    if !low_amount_samples.is_empty() {
        println!("低档「唯一兜底」金额样本（2026-10-09 用户拍板后已放开自动填）：");
        for c in &low_amount_samples {
            println!("  {c}");
        }
    }
    Ok(())
}
