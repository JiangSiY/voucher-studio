//! v0.16 迁移安全：**老库升级不丢数据**。
//!
//! 用户手上是 v0.15 的库（无 `dup_decisions`）。本用例模拟"老库 → 新版本启动"：
//! 造数据 → 删掉 v0.16 新增的表与索引（还原成老库形态）→ 重新 `CoreContext::init`
//! → 断言 ① 既有凭证一张不少（`init` 的清库分支只在 `user_version < 4` 时触发）
//! ② 处置表已补齐、可直接写入判定。
//!
//! 这条不变量比"功能可用"更要紧：迁移一旦清库，用户的历史凭证就没了。

mod common;

use common::*;
use voucher_core::models::NewVoucher;
use voucher_core::services::{dup_review, voucher_mgr};
use voucher_core::{CoreContext, Result};

/// 打开（或再打开）指定目录的上下文
fn ctx_at(dir: &std::path::Path) -> Result<CoreContext> {
    install_memory_keyring();
    CoreContext::init(Some(dir.to_path_buf()))
}

#[test]
fn upgrade_from_pre_v016_db_keeps_vouchers_and_adds_decisions_table() -> Result<()> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().to_path_buf();

    // ---- 第一步：新版本建库 + 造两张同日期同金额的凭证（构成一组疑似重复）----
    let (a_id, b_id, before) = {
        let ctx = ctx_at(&dir)?;
        let mk = |no: &str| -> Result<i64> {
            let v = voucher_mgr::create_voucher(
                &ctx,
                NewVoucher {
                    voucher_no: no.into(),
                    voucher_date: "2026-08-20".into(),
                    description: "租金".into(),
                    counterparty: "甲物业".into(),
                    amount_cents: Some(66_600),
                    status: None,
                    tags: Vec::new(),
                    note: String::new(),
                },
            )?;
            Ok(v.id)
        };
        let a = mk("记-2026-08-020")?;
        let b = mk("记-2026-08-021")?;
        let before = voucher_count(&ctx)?;
        assert_eq!(before, 2);

        // ---- 还原成老库形态：v0.16 之前没有 dup_decisions（表 + 唯一索引都没有）----
        ctx.db.with_conn(|conn| {
            conn.execute_batch("DROP TABLE dup_decisions;")?;
            let left: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='dup_decisions'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(left, 0, "前置：已还原为老库（无处置表）");
            Ok(())
        })?;
        (a, b, before)
    };
    // 第一个上下文在此释放（用完即弃），模拟应用退出

    // ---- 第二步：新版本再次启动（同一数据目录）----
    let ctx = ctx_at(&dir)?;

    assert_eq!(
        voucher_count(&ctx)?,
        before,
        "升级不得清库：既有凭证必须一张不少（清库分支只应响应 user_version < 4）"
    );

    let queue = dup_review::review_queue(&ctx)?;
    assert_eq!(queue.len(), 1, "老库里的这一组疑似重复应照常出现在队列");

    // 处置表已补齐且可写（幂等 `execute_batch` 生效的证据）
    let id = dup_review::resolve_pair(&ctx, a_id, b_id, "升级后标记一次")?;
    assert!(id > 0, "升级后应能直接写判定");
    assert!(dup_review::review_queue(&ctx)?.is_empty());
    Ok(())
}
