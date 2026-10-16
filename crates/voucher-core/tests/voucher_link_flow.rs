//! v0.22 凭证关联 / 事项组：功能语义 + 升级安全 + 留痕独立性。
//!
//! 覆盖方案 v2（`docs/方案-v2-打印日报与凭证关联（定稿）.md`）里可测的几条：
//! - 两两关联**归一化 + 幂等**（同一对同 `kind` 只一行，重复建立＝覆盖备注）；
//! - **双向可见**（a 端 / b 端都能查到，且 `is_a` 正确）；
//! - 不同 `kind` 是不同关系，可并存；
//! - 自关联、不存在的凭证 → 拒绝；
//! - 建/解关联都写 **append-only 审计**；解关联幂等；
//! - **删凭证级联清理关联，但不碰审计表**（留痕独立于凭证生命周期）；
//! - 事项组：建组 / 加成员（幂等）/ 排序 / 移出 / 按凭证查组 / 列表 / 删组级联；
//! - **升级安全**：老库（无这四张表）重新 `init` 后凭证一张不少、表补齐、立即可写。

mod common;

use common::*;
use voucher_core::models::enums::VoucherLinkKind;
use voucher_core::models::NewVoucher;
use voucher_core::services::query::VoucherQuery;
use voucher_core::services::{links, voucher_mgr};
use voucher_core::{CoreContext, Result};

/// 打开（或再打开）指定目录的上下文
fn ctx_at(dir: &std::path::Path) -> Result<CoreContext> {
    install_memory_keyring();
    CoreContext::init(Some(dir.to_path_buf()))
}

/// 造一张凭证，返回 id
fn mk(ctx: &CoreContext, no: &str, amount_cents: Option<i64>) -> Result<i64> {
    let v = voucher_mgr::create_voucher(
        ctx,
        NewVoucher {
            voucher_no: no.into(),
            voucher_date: "2026-08-20".into(),
            description: format!("{no} 摘要"),
            counterparty: "文一办公用品店".into(),
            amount_cents,
            status: None,
            tags: Vec::new(),
            note: String::new(),
        },
    )?;
    Ok(v.id)
}

fn new_ctx() -> (tempfile::TempDir, CoreContext) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ctx = ctx_at(tmp.path()).expect("init ctx");
    (tmp, ctx)
}

#[test]
fn link_unlink_roundtrip_normalized_and_idempotent() -> Result<()> {
    let (_dir, ctx) = new_ctx();
    let a = mk(&ctx, "记-A", Some(10_000))?;
    let b = mk(&ctx, "记-B", Some(20_000))?;
    let c = mk(&ctx, "记-C", Some(30_000))?;

    // 归一化：用 (c, b) 顺序建立，落库应是 (min, max)
    let l = links::link(&ctx, c, b, VoucherLinkKind::Detail, "  八月差旅明细  ")?;
    assert_eq!(l.a_id, b.min(c), "a_id 应是较小 id");
    assert_eq!(l.b_id, b.max(c), "b_id 应是较大 id");
    assert_eq!(l.note, "八月差旅明细", "备注两端去空白");

    // 幂等：同一对同 kind 再建 → 同一行，备注被覆盖（created_at 保留）
    let l2 = links::link(&ctx, b, c, VoucherLinkKind::Detail, "改备注")?;
    assert_eq!(l2.id, l.id, "同一对同 kind 仍是一行");
    assert_eq!(l2.created_at, l.created_at, "created_at 保留首次建立时间");
    assert_eq!(l2.note, "改备注");

    // 双向可见 + is_a 正确
    let of_b = links::links_of(&ctx, b)?;
    assert_eq!(of_b.len(), 1);
    assert_eq!(of_b[0].other.id, c);
    assert!(of_b[0].is_a, "b 是归一化后的 a 端");
    let of_c = links::links_of(&ctx, c)?;
    assert_eq!(of_c[0].other.id, b);
    assert!(!of_c[0].is_a, "c 是归一化后的 b 端");

    // 不同 kind 是不同关系，可并存
    links::link(&ctx, b, c, VoucherLinkKind::Explains, "说明")?;
    assert_eq!(links::links_of(&ctx, b)?.len(), 2, "两种关系并存");

    // 自关联 / 不存在的凭证 → 拒绝
    assert!(links::link(&ctx, a, a, VoucherLinkKind::Explains, "").is_err());
    assert!(links::link(&ctx, a, 999_999, VoucherLinkKind::Explains, "").is_err());

    // 审计：3 条建立事件（detail 建了两次 upsert + explains 一次），都带 linked
    let ev = links::events_of(&ctx, b)?;
    assert_eq!(ev.len(), 3, "每次建立都留痕（upsert 也算一次动作）");
    assert!(ev.iter().all(|e| e.action == "linked"));

    // 解除（幂等：第二次返回 false）
    assert!(links::unlink(&ctx, c, b, VoucherLinkKind::Detail)?);
    assert!(!links::unlink(&ctx, c, b, VoucherLinkKind::Detail)?);
    assert_eq!(
        links::links_of(&ctx, b)?.len(),
        1,
        "detail 已解，explains 仍在"
    );
    let ev2 = links::events_of(&ctx, b)?;
    assert_eq!(ev2.len(), 4);
    assert_eq!(ev2[0].action, "unlinked", "最新一条是解除动作");
    assert_eq!(ev2[0].note, "改备注", "解除事件保留当时备注");
    Ok(())
}

#[test]
fn deleting_voucher_cascades_links_but_keeps_audit_trail() -> Result<()> {
    let (_dir, ctx) = new_ctx();
    let a = mk(&ctx, "记-A", Some(100))?;
    let b = mk(&ctx, "记-B", Some(200))?;
    links::link(&ctx, a, b, VoucherLinkKind::Explains, "A 说明 B")?;

    let deleted = voucher_mgr::delete_vouchers(&ctx, &[b])?;
    assert_eq!(deleted, 1);

    assert!(
        links::links_of(&ctx, a)?.is_empty(),
        "关联随凭证级联清理，不留孤儿"
    );
    // 审计留痕**不随凭证消亡**（这正是 `voucher_link_events` 刻意不加外键的理由）
    let ev = links::events_of(&ctx, b)?;
    assert_eq!(ev.len(), 1, "被删凭证的关联留痕仍可追溯");
    assert_eq!(ev[0].action, "linked");
    Ok(())
}

#[test]
fn groups_carry_details_and_are_idempotent() -> Result<()> {
    let (_dir, ctx) = new_ctx();
    let a = mk(&ctx, "记-A", Some(100))?;
    let b = mk(&ctx, "记-B", Some(200))?;
    let c = mk(&ctx, "记-C", None)?;

    // 标题必填
    assert!(links::create_group(&ctx, "   ", "").is_err());

    let g = links::create_group(&ctx, "2026-08 差旅", "含机票与住宿")?;
    assert_eq!(g.title, "2026-08 差旅");
    assert!(g.members.is_empty());
    assert_eq!(g.members_amount_cents(), 0);

    // 加成员（保持入参顺序）+ 幂等（重复加入不再新增）
    let g = links::add_to_group(&ctx, g.id, &[c, a, b])?;
    let ids: Vec<i64> = g.members.iter().map(|v| v.id).collect();
    assert_eq!(ids, vec![c, a, b], "成员按加入顺序");
    let g = links::add_to_group(&ctx, g.id, &[a, c])?;
    assert_eq!(g.members.len(), 3, "重复加入是幂等的");
    assert_eq!(
        g.members_amount_cents(),
        300,
        "组内合计是派生值（300 分 = 3 元）"
    );

    // 空入参 / 不存在的组 / 不存在的凭证
    assert!(links::add_to_group(&ctx, g.id, &[]).is_err());
    assert!(links::add_to_group(&ctx, 999_999, &[a]).is_err());
    assert!(links::add_to_group(&ctx, g.id, &[999_999]).is_err());

    // 按凭证查组 + 列表
    let of_a = links::groups_of_voucher(&ctx, a)?;
    assert_eq!(of_a.len(), 1);
    assert_eq!(of_a[0].id, g.id);
    assert!(links::groups_of_voucher(&ctx, 999_999)?.is_empty());
    assert_eq!(links::list_groups(&ctx)?.len(), 1);

    // 移出（幂等）
    let g = links::remove_from_group(&ctx, g.id, a)?;
    assert_eq!(g.members.len(), 2);
    let g = links::remove_from_group(&ctx, g.id, a)?;
    assert_eq!(g.members.len(), 2, "重复移出不报错");

    // 删组 → 成员行级联清理
    assert!(links::delete_group(&ctx, g.id)?);
    assert!(!links::delete_group(&ctx, g.id)?, "重复删组返回 false");
    assert!(links::groups_of_voucher(&ctx, c)?.is_empty());
    Ok(())
}

#[test]
fn upgrade_from_pre_v022_db_keeps_vouchers_and_adds_link_tables() -> Result<()> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().to_path_buf();

    let before = {
        let ctx = ctx_at(&dir)?;
        let a = mk(&ctx, "记-A", Some(100))?;
        let b = mk(&ctx, "记-B", Some(200))?;
        links::link(&ctx, a, b, VoucherLinkKind::Explains, "老库里的关联")?;
        let before = voucher_count(&ctx)?;
        assert_eq!(before, 2);

        // 还原成 v0.22 之前的老库形态：四张新表都不存在
        ctx.db.with_conn(|conn| {
            conn.execute_batch(
                "DROP TABLE voucher_links;
                 DROP TABLE voucher_groups;
                 DROP TABLE voucher_group_items;
                 DROP TABLE voucher_link_events;",
            )?;
            let left: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN
                 ('voucher_links','voucher_groups','voucher_group_items','voucher_link_events')",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(left, 0, "前置：已还原为老库（无关联四表）");
            Ok(())
        })?;
        before
    };

    // 新版本再次启动（同一数据目录）：清库分支只应响应 user_version < 4
    let ctx = ctx_at(&dir)?;
    assert_eq!(
        voucher_count(&ctx)?,
        before,
        "升级不得清库：既有凭证必须一张不少"
    );

    // 四张表已补齐且立即可写（幂等 `execute_batch(SCHEMA_V9)` 生效的证据）
    let ids: Vec<i64> = ctx.db.with_conn(|conn| {
        let mut stmt = conn.prepare("SELECT id FROM vouchers ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<i64>>>()?)
    })?;
    assert_eq!(ids.len(), 2);
    let l = links::link(&ctx, ids[0], ids[1], VoucherLinkKind::Detail, "升级后新建")?;
    assert!(l.id > 0);
    let g = links::create_group(&ctx, "升级后新组", "")?;
    assert!(links::add_to_group(&ctx, g.id, &ids)?.members.len() == 2);
    assert_eq!(links::links_of(&ctx, ids[0])?.len(), 1);
    Ok(())
}

#[test]
fn unlinked_filter_matches_only_vouchers_without_links_or_groups() -> Result<()> {
    let (_dir, ctx) = new_ctx();
    let a = mk(&ctx, "记-A", Some(100))?;
    let b = mk(&ctx, "记-B", Some(200))?;
    let c = mk(&ctx, "记-C", Some(300))?;
    let d = mk(&ctx, "记-D", Some(400))?;

    // a—b 有关联；c 在事项组里；d 什么都没有
    links::link(&ctx, a, b, VoucherLinkKind::Explains, "说明")?;
    let g = links::create_group(&ctx, "2026-08 差旅", "")?;
    links::add_to_group(&ctx, g.id, &[c])?;

    let only = |ctx: &CoreContext, v: bool| -> Result<(u64, Vec<i64>)> {
        let q = VoucherQuery {
            unlinked: Some(v),
            ..Default::default()
        };
        // `voucher_mgr::list` 返回 (items, total)
        let (items, total) = voucher_mgr::list(ctx, &q)?;
        Ok((total, items.iter().map(|x| x.id).collect()))
    };

    let (total, ids) = only(&ctx, true)?;
    assert_eq!(total, 1, "只有 d 未被佐证");
    assert_eq!(ids, vec![d]);

    let (total, mut ids) = only(&ctx, false)?;
    assert_eq!(total, 3, "a/b（有关联）与 c（在组里）都算已有佐证");
    ids.sort_unstable();
    let mut expect = vec![a, b, c];
    expect.sort_unstable();
    assert_eq!(ids, expect);

    // 解除关联后 a、b 回到"未被佐证"（筛选口径跟着关联走）
    assert!(links::unlink(&ctx, a, b, VoucherLinkKind::Explains)?);
    let (total, mut ids) = only(&ctx, true)?;
    assert_eq!(total, 3);
    ids.sort_unstable();
    let mut expect = vec![a, b, d];
    expect.sort_unstable();
    assert_eq!(ids, expect);

    // None = 不筛（默认列表口径不受影响）
    let (items, total) = voucher_mgr::list(&ctx, &VoucherQuery::default())?;
    assert_eq!((items.len() as u64, total), (4, 4));
    Ok(())
}
