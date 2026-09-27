//! 性能压测（**可选，环境变量门控**）。
//!
//! 用法：
//! ```text
//! VOUCHER_BENCH=1   cargo test -p voucher-core --test perf_bench -- --nocapture   # 默认 2000 张
//! VOUCHER_BENCH=500 cargo test -p voucher-core --test perf_bench -- --nocapture   # 自定义规模
//! ```
//!
//! **为什么不标 `#[ignore]`**：CI 以 `--include-ignored` 全量执行（见 `.github/workflows/ci.yml`），
//! 标了 ignore 反而会被 CI 真跑（造数千张 + 聚合，白烧 runner 时间）。环境变量门控才能做到
//! "默认什么都不做、需要时手动跑"。
//!
//! 判据是**宽松软阈值**：只抓"数量级退化"（例如索引被删导致全表扫、N+1 查询变 N×M），
//! 不追求微基准精度——测试跑在 debug 构建下，绝对值本就比 release 慢数倍。
//!
//! 对应审计报告 §5 第 3 项（"全量扫描性能未压测"）：本文件把『没有工具』变成『有工具可跑』，
//! 真正的结论仍需在有代表性的真实数据量下各跑一次并记录。

mod common;

use std::time::Instant;

use common::*;
use voucher_core::models::NewVoucher;
use voucher_core::services::query::VoucherQuery;
use voucher_core::services::{dashboard, dup_review, voucher_mgr};
use voucher_core::Result;

/// 默认规模（张）
const DEFAULT_ROWS: usize = 2000;
/// 单条路径的软阈值（debug 构建；只抓数量级退化）
const BUDGET_MS: u128 = 5_000;

fn bench_rows() -> Option<usize> {
    match std::env::var("VOUCHER_BENCH") {
        Ok(v) if !v.is_empty() && v != "0" => Some(v.parse().unwrap_or(DEFAULT_ROWS)),
        _ => None,
    }
}

#[test]
fn bench_list_and_aggregations() -> Result<()> {
    let Some(rows) = bench_rows() else {
        println!("跳过性能压测（未设 VOUCHER_BENCH；VOUCHER_BENCH=1 或 =N 启用）");
        return Ok(());
    };
    let (_dir, ctx) = test_ctx();

    // 造数：**周期 336**（28 天 × 12 月）——同周期内 (日期, 金额) 完全相同，于是每 336 张里
    // 会形成一组"同日期同金额"的疑似重复（`VOUCHER_BENCH=2000` 时约 6 张/组），
    // 这样 `pending_count` / `review_queue` 这两条重算路径才真的被压到；
    // 描述唯一、无附件，故高置信度对为 0（弱证据为主，接近真实"同额不同事"的情形）。
    let t0 = Instant::now();
    for i in 0..rows {
        let period = i % 336;
        let month = (period / 28) + 1;
        voucher_mgr::create_voucher(
            &ctx,
            NewVoucher {
                voucher_no: format!("压-{i:05}"),
                voucher_date: format!("2026-{month:02}-{:02}", (period % 28) + 1),
                description: format!("压测摘要 {i}"),
                counterparty: format!("对方 {}", i % 50),
                amount_cents: Some(((period as i64) + 1) * 100),
                status: None,
                tags: vec![format!("标签{}", i % 20)],
                note: String::new(),
            },
        )?;
    }
    println!("造数 {rows} 张耗时 {}ms", t0.elapsed().as_millis());

    let timed = |name: &str, f: &dyn Fn() -> Result<usize>| -> Result<()> {
        let t = Instant::now();
        let n = f()?;
        let ms = t.elapsed().as_millis();
        println!("  {name:<38} {ms:>7}ms  （{n} 行）");
        assert!(
            ms < BUDGET_MS,
            "{name} 耗时 {ms}ms 超过软阈值 {BUDGET_MS}ms——疑似索引失效或 N+1（不是微基准问题）"
        );
        Ok(())
    };

    println!("路径耗时（debug 构建，软阈值 {BUDGET_MS}ms）：");
    let page = VoucherQuery {
        page: 1,
        page_size: 50,
        ..Default::default()
    };
    timed("query.list（首页 50 条）", &|| {
        Ok(voucher_mgr::list(&ctx, &page)?.0.len())
    })?;
    timed("query.voucher_ids（跨页全选）", &|| {
        Ok(voucher_core::services::query::voucher_ids(&ctx, &page)?.len())
    })?;
    timed("dashboard.overview（本月窗口）", &|| {
        Ok(dashboard::overview(
            &ctx,
            Some("2026-01-01".into()),
            Some("2026-12-31".into()),
            None,
        )?
        .by_status
        .len())
    })?;
    timed("dashboard.monthly_breakdown(对方)", &|| {
        Ok(dashboard::monthly_breakdown(&ctx, None, None, "counterparty", 5)?.len())
    })?;
    timed("dashboard.monthly_breakdown(标签)", &|| {
        Ok(dashboard::monthly_breakdown(&ctx, None, None, "tag", 5)?.len())
    })?;
    timed("dashboard.export_coverage", &|| {
        Ok(dashboard::export_coverage(&ctx, None, None)?
            .by_format
            .len())
    })?;
    timed("dashboard.storage_usage", &|| {
        Ok(dashboard::storage_usage(&ctx, false)?.original_count as usize)
    })?;
    timed("dashboard.todos", &|| {
        let t = dashboard::todos(&ctx)?;
        Ok((t.ocr_pending + t.stale_drafts + t.amount_missing) as usize)
    })?;
    timed("dup_review.pending_count（重算口径）", &|| {
        let c = dup_review::pending_count(&ctx)?;
        Ok((c.groups + c.pairs + c.high_pairs) as usize)
    })?;
    timed("dup_review.review_queue（逐对证据）", &|| {
        Ok(dup_review::review_queue(&ctx)?.len())
    })?;
    Ok(())
}
