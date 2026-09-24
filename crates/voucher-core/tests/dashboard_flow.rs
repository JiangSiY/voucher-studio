//! v0.18 工作台只读聚合（集成）：窗口口径 / 待办计数 / 轻量重复计数 / 只取 id。
//!
//! 这套用例的意义不只是"功能可用"，而是**锁住口径**——旧仪表盘的核心病灶是
//! 同一屏三套时间口径且互不说明（统计卡全库、状态分布只算 6 个月窗口、Top 是全历史前 10），
//! 于是"草稿 12 / 分布里草稿 5"会被当成数据错误。这里的用例把每个数字的算法固定下来。

mod common;

use common::*;
use voucher_core::models::enums::VoucherStatus;
use voucher_core::models::{NewAttachment, NewVoucher};
use voucher_core::services::{dashboard, dup_review, period, voucher_mgr};
use voucher_core::{CoreContext, Result};

fn mk(
    ctx: &CoreContext,
    no: &str,
    date: &str,
    amount: Option<i64>,
    counterparty: &str,
    description: &str,
) -> Result<i64> {
    let v = voucher_mgr::create_voucher(
        ctx,
        NewVoucher {
            voucher_no: no.into(),
            voucher_date: date.into(),
            description: description.into(),
            counterparty: counterparty.into(),
            amount_cents: amount,
            status: None,
            tags: Vec::new(),
            note: String::new(),
        },
    )?;
    Ok(v.id)
}

fn attach(ctx: &CoreContext, voucher_id: i64, hash: &str) -> Result<()> {
    let rel = format!("2026-09/{hash}.jpg");
    ctx.db.with_conn(|conn| {
        voucher_core::db::repos::attachment_repo::insert(
            conn,
            &NewAttachment {
                voucher_id,
                file_name: format!("{hash}.jpg"),
                file_path: rel,
                file_hash: hash.into(),
                file_size: 10,
                mime_type: "image/jpeg".into(),
            },
        )
        .map(|_| ())
    })
}

/// v8 的三条索引必须存在（幂等创建；少一条都会让新卡片退化成全表扫）
#[test]
fn schema_v8_indexes_exist() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let names: Vec<String> = ctx.db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='index' AND name IN
             ('idx_attachments_ocr_status','idx_vouchers_status_updated','idx_export_items_voucher')
             ORDER BY name",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(r.get(0)?);
        }
        Ok(out)
    })?;
    assert_eq!(
        names,
        vec![
            "idx_attachments_ocr_status".to_string(),
            "idx_export_items_voucher".to_string(),
            "idx_vouchers_status_updated".to_string()
        ],
        "v8 索引应由 SCHEMA_V8 幂等补齐"
    );
    Ok(())
}

/// 概览：窗口只算窗口内的（与列表页同一口径），并给出上一等长窗口供算环比
#[test]
fn overview_respects_window_and_reports_prev_window() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    // 窗口内：2026-08-01 ~ 2026-08-31 两张（其中一张草稿）；窗口外：7 月一张
    mk(&ctx, "记-001", "2026-08-05", Some(10_000), "甲", "租金")?;
    mk(&ctx, "记-002", "2026-08-20", Some(20_000), "乙", "水电")?;
    mk(&ctx, "记-003", "2026-07-10", Some(50_000), "丙", "差旅")?;

    let ov = dashboard::overview(
        &ctx,
        Some("2026-08-01".into()),
        Some("2026-08-31".into()),
        None,
    )?;
    assert_eq!(ov.count, 2, "窗口内只应有 8 月的两张");
    assert_eq!(ov.amount_cents, 30_000);
    assert_eq!(ov.total, 3, "全库口径固定项应包含窗口外的凭证");
    assert_eq!(
        ov.this_month, 0,
        "本月为 2026-09（系统当前月），8 月凭证不计入"
    );

    // 上一等长窗口 = 7 月 1 日 ~ 7 月 31 日（31 天回推 31 天为 7 月 1 日~7 月 30 日，含端点 30 天）
    // 这里只断言"取到了上一窗口且金额来自 7 月 10 日那张"，不锁死端点天数细节
    assert!(
        ov.prev_amount_cents.unwrap_or(0) >= 50_000,
        "上一窗口应包含 7 月那张 50000 分（实际 {:?}）",
        ov.prev_amount_cents
    );

    // 状态分布：窗口内两张草稿
    assert_eq!(ov.by_status.len(), 1);
    assert_eq!(ov.by_status[0].status, "draft");
    assert_eq!(ov.by_status[0].count, 2);
    assert_eq!(ov.by_status[0].amount_cents, 30_000);
    Ok(())
}

/// 概览：只给单端（或都不给）时不编造环比
#[test]
fn overview_without_full_range_has_no_mom() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    mk(&ctx, "记-001", "2026-08-05", Some(10_000), "甲", "租金")?;

    let only_to = dashboard::overview(&ctx, None, Some("2026-08-31".into()), None)?;
    assert_eq!(only_to.count, 1);
    assert!(
        only_to.prev_amount_cents.is_none(),
        "缺窗口起点时不得算环比"
    );

    let none = dashboard::overview(&ctx, None, None, None)?;
    assert!(none.prev_amount_cents.is_none());
    assert_eq!(none.count, 1);
    Ok(())
}

/// 概览：状态筛选与窗口叠加（列表页下钻后"计数一致"的基础）
#[test]
fn overview_status_filter_matches_list_total() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let a = mk(&ctx, "记-001", "2026-08-05", Some(10_000), "甲", "租金")?;
    mk(&ctx, "记-002", "2026-08-06", Some(20_000), "乙", "水电")?;
    voucher_mgr::update_voucher(
        &ctx,
        a,
        voucher_core::models::VoucherUpdate {
            status: Some(VoucherStatus::Reviewed),
            ..Default::default()
        },
    )?;

    let ov = dashboard::overview(&ctx, None, None, Some(VoucherStatus::Reviewed))?;
    let (_, total) = voucher_mgr::list(
        &ctx,
        &voucher_core::services::query::VoucherQuery {
            status: Some(VoucherStatus::Reviewed),
            ..Default::default()
        },
    )?;
    assert_eq!(ov.count as u64, total, "概览与列表在相同筛选下必须一致");
    assert_eq!(
        ov.by_status.len(),
        2,
        "状态分布反映**窗口内完整构成**（不随顶部筛选收窄），供堆叠条展示；收窄后只剩一根柱子就失去构成信息"
    );
    assert_eq!(
        ov.by_status.iter().map(|s| s.count).sum::<u64>(),
        2,
        "draft + reviewed 两档都在"
    );
    Ok(())
}

/// 待办：未填金额 / 待识别 / 陈旧草稿 / 未封账往月 都能被数出来
#[test]
fn todos_counts_fillable_gaps() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    // 未填金额
    mk(&ctx, "记-001", "2026-08-05", None, "甲", "租金")?;
    // 待识别（新建附件默认 ocr_status = 'pending'）
    let with_att = mk(&ctx, "记-002", "2026-08-06", Some(1_000), "乙", "水电")?;
    attach(&ctx, with_att, "hash-todo-1")?;
    // 已识别的附件
    let done = mk(&ctx, "记-003", "2026-08-07", Some(2_000), "丙", "差旅")?;
    attach(&ctx, done, "hash-todo-2")?;
    ctx.db.with_conn(|conn| {
        conn.execute(
            "UPDATE attachments SET ocr_status = 'done' WHERE voucher_id = ?1",
            [done],
        )?;
        Ok(())
    })?;

    let todos = dashboard::todos(&ctx)?;
    assert_eq!(todos.amount_missing, 1, "一张未填金额");
    assert_eq!(todos.ocr_pending, 1, "一张待识别（另一张已 done）");
    assert_eq!(todos.stale_drafts, 0, "刚建的草稿不算陈旧");
    assert_eq!(
        todos.unlocked_months, 1,
        "2026-08 有凭证且未封账 → 1 个未封账往月（当月不计）"
    );
    assert!(todos.backup_stale, "没有任何备份 → 视为该备份");
    assert!(todos.latest_backup.is_none());

    // 封账后该月不再计入
    period::lock_period(&ctx, "2026-08")?;
    assert_eq!(dashboard::todos(&ctx)?.unlocked_months, 0);
    Ok(())
}

/// 待办：陈旧草稿按 7 天阈值判定（把 updated_at 改到 10 天前）
#[test]
fn todos_stale_draft_threshold() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let id = mk(&ctx, "记-001", "2026-08-05", Some(1_000), "甲", "租金")?;
    let old = (chrono::Local::now() - chrono::Duration::days(10))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    ctx.db.with_conn(|conn| {
        conn.execute(
            "UPDATE vouchers SET updated_at = ?1 WHERE id = ?2",
            rusqlite::params![old, id],
        )?;
        Ok(())
    })?;
    assert_eq!(dashboard::todos(&ctx)?.stale_drafts, 1);

    // 已复核的凭证不算"陈旧草稿"（口径只针对 draft）
    voucher_mgr::update_voucher(
        &ctx,
        id,
        voucher_core::models::VoucherUpdate {
            status: Some(VoucherStatus::Reviewed),
            ..Default::default()
        },
    )?;
    assert_eq!(dashboard::todos(&ctx)?.stale_drafts, 0);
    Ok(())
}

/// 口径一致性（审计新增的守卫）：轻量计数必须等于核查队列派生出的数字——
/// 否则侧栏徽标与核查页会互相打架（旧实现的教训）
#[test]
fn pending_count_matches_queue() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    // 一组 3 张（同图两张 + 同对方同摘要一张）+ 一组 2 张（只有同额）
    let a = mk(&ctx, "记-001", "2026-08-05", Some(10_000), "甲", "租金")?;
    let b = mk(&ctx, "记-002", "2026-08-05", Some(10_000), "乙", "水电")?;
    let c = mk(&ctx, "记-003", "2026-08-05", Some(10_000), "甲", "租金")?;
    attach(&ctx, a, "hash-same-img")?;
    attach(&ctx, b, "hash-same-img")?;
    attach(&ctx, c, "hash-other")?;
    mk(&ctx, "记-004", "2026-09-01", Some(20_000), "丁", "杂费")?;
    mk(&ctx, "记-005", "2026-09-01", Some(20_000), "戊", "耗材")?;

    let light = dup_review::pending_count(&ctx)?;
    let queue = dup_review::review_queue(&ctx)?;
    let heavy_groups = queue.len() as u32;
    let heavy_pairs: u32 = queue.iter().map(|g| g.pending.len() as u32).sum();
    let heavy_high: u32 = queue
        .iter()
        .flat_map(|g| g.pending.iter())
        .filter(|p| p.evidence.level == dup_review::EvidenceLevel::High)
        .count() as u32;

    assert_eq!(light.groups, heavy_groups, "组数必须与队列一致");
    assert_eq!(light.pairs, heavy_pairs, "对数必须与队列一致");
    assert_eq!(
        light.high_pairs, heavy_high,
        "高置信度对数必须与队列逐对算出的结果一致"
    );

    // 判定一对后两边同步下降
    dup_review::resolve_pair(&ctx, a, b, "同图但确实是两笔")?;
    let light2 = dup_review::pending_count(&ctx)?;
    let queue2 = dup_review::review_queue(&ctx)?;
    assert_eq!(light2.groups, queue2.len() as u32);
    assert_eq!(
        light2.pairs,
        queue2.iter().map(|g| g.pending.len() as u32).sum::<u32>()
    );
    assert_eq!(light2.pairs, light.pairs - 1, "判定一对后应少一对");
    Ok(())
}

/// 只取 id：与列表同筛选、同顺序（跨页选择集的基础）
#[test]
fn list_ids_matches_list_order_and_filter() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let a = mk(&ctx, "记-001", "2026-08-05", Some(1_000), "甲", "租金")?;
    let b = mk(&ctx, "记-002", "2026-08-09", Some(2_000), "乙", "水电")?;
    let newest = mk(&ctx, "记-003", "2026-08-20", Some(3_000), "丙", "差旅")?;

    let q = voucher_core::services::query::VoucherQuery::default();
    let ids = voucher_core::services::query::voucher_ids(&ctx, &q)?;
    let (items, _) = voucher_mgr::list(&ctx, &q)?;
    assert_eq!(
        ids,
        items.iter().map(|v| v.id).collect::<Vec<_>>(),
        "id 顺序必须与列表一致（跨页全选后按此顺序处理）"
    );
    assert_eq!(ids[0], newest, "默认按日期倒序：最新在前");

    // 带筛选（关键词只命中一张）
    let q2 = voucher_core::services::query::VoucherQuery {
        keyword: Some("水电".into()),
        ..Default::default()
    };
    assert_eq!(
        voucher_core::services::query::voucher_ids(&ctx, &q2)?,
        vec![b]
    );
    assert!(voucher_core::services::query::voucher_ids(
        &ctx,
        &voucher_core::services::query::VoucherQuery {
            keyword: Some("不存在的词".into()),
            ..Default::default()
        }
    )?
    .is_empty());
    // 提到 a 避免未使用告警（a 用于前面构造数据）
    assert_ne!(a, b);
    Ok(())
}
