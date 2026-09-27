//! 只读聚合的耗时观测（审计报告 §4.3 的可观测性缺口）。
//!
//! 背景：`pending_summary`、`monthly_breakdown` 这类**每次调用都重算**的路径此前没有任何
//! 耗时记录——用户报"某页卡"时只能靠猜。这里给出统一留痕口径：超过 [`SLOW_MS`] 告警
//! （会落盘到数据目录 `logs/`，级别 Info 起），正常走 `debug`（默认不落盘，需要时再开）。
//!
//! 只覆盖**只读聚合**：写路径的耗时由各自的领域服务管，不在这里掺和。

use std::time::Instant;

/// 慢查询阈值。本地库在千张量级正常都在数十毫秒内，超过这个数说明要么数据量上了一个
/// 台阶、要么索引没走成——那种时候日志里必须留下痕迹，否则"变慢"永远无法归因。
pub const SLOW_MS: u128 = 500;

/// 记录一次只读聚合的耗时。`name` 用 `模块.函数` 形式（便于在日志里 grep 到具体路径）。
pub fn log_elapsed(name: &str, started: Instant) {
    let ms = started.elapsed().as_millis();
    if ms >= SLOW_MS {
        log::warn!(
            "{name} 耗时 {ms}ms（阈值 {SLOW_MS}ms）——疑似数据量上台阶或索引失效，值得看一眼"
        );
    } else {
        log::debug!("{name} 耗时 {ms}ms");
    }
}
