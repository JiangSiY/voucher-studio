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

// ---------- v0.18 批次 5：精确筛选（VoucherQuery 扩维）与台账聚合 ----------

/// 精确筛选：标签（任一命中）/ 对方（整串相等）/ 金额区间 / 金额为空 / OCR 状态 / 导出覆盖
#[test]
fn voucher_query_supports_precise_filters() -> Result<()> {
    use voucher_core::services::query::VoucherQuery;
    let (_dir, ctx) = test_ctx();
    let a = mk(&ctx, "记-001", "2026-08-05", Some(10_000), "甲物业", "租金")?;
    let b = mk(&ctx, "记-002", "2026-08-06", Some(20_000), "乙公司", "水电")?;
    let _c = mk(&ctx, "记-003", "2026-08-07", None, "甲物业", "杂费")?;
    attach(&ctx, a, "hash-q-1")?; // 默认 pending
    attach(&ctx, b, "hash-q-2")?; // b 之后被标成 done（附件行必须先存在）
    ctx.db.with_conn(|conn| {
        conn.execute(
            "UPDATE attachments SET ocr_status='done' WHERE voucher_id=?1",
            [b],
        )?;
        conn.execute(
            "UPDATE vouchers SET tags=json_array('差旅','报销') WHERE id=?1",
            [a],
        )?;
        conn.execute(
            "UPDATE vouchers SET tags=json_array('差旅') WHERE id=?1",
            [b],
        )?;
        Ok(())
    })?;

    let count = |q: VoucherQuery| -> Result<usize> { Ok(voucher_mgr::list(&ctx, &q)?.0.len()) };

    // 标签精确：'差旅' 命中 a、b（c 无标签）；'报销' 只命中 a
    assert_eq!(
        count(VoucherQuery {
            tags: vec!["差旅".into()],
            ..Default::default()
        })?,
        2
    );
    assert_eq!(
        count(VoucherQuery {
            tags: vec!["报销".into()],
            ..Default::default()
        })?,
        1
    );
    // 多标签是"或"：差旅 ∪ 报销 = a、b
    assert_eq!(
        count(VoucherQuery {
            tags: vec!["差旅".into(), "报销".into()],
            ..Default::default()
        })?,
        2
    );

    // 对方精确：'甲物业' 命中 a、c；模糊串 '甲' 不命中（与 keyword 的模糊匹配区分开）
    assert_eq!(
        count(VoucherQuery {
            counterparty: Some("甲物业".into()),
            ..Default::default()
        })?,
        2
    );
    assert_eq!(
        count(VoucherQuery {
            counterparty: Some("甲".into()),
            ..Default::default()
        })?,
        0
    );

    // 金额区间与"金额为空"
    assert_eq!(
        count(VoucherQuery {
            amount_from: Some(15_000),
            ..Default::default()
        })?,
        1
    );
    assert_eq!(
        count(VoucherQuery {
            amount_to: Some(15_000),
            ..Default::default()
        })?,
        1
    );
    assert_eq!(
        count(VoucherQuery {
            amount_empty: true,
            ..Default::default()
        })?,
        1
    );
    // amount_empty 优先于区间（两者同给时按空金额筛）
    assert_eq!(
        count(VoucherQuery {
            amount_empty: true,
            amount_from: Some(0),
            ..Default::default()
        })?,
        1
    );

    // OCR 状态：只 a 是 pending
    assert_eq!(
        count(VoucherQuery {
            ocr_status: Some("pending".into()),
            ..Default::default()
        })?,
        1
    );
    assert_eq!(
        count(VoucherQuery {
            ocr_status: Some("done".into()),
            ..Default::default()
        })?,
        1
    );

    // 导出覆盖：谁都没导出过 → 全部 never，any 为空
    assert_eq!(
        count(VoucherQuery {
            exported: Some("never".into()),
            ..Default::default()
        })?,
        3
    );
    assert_eq!(
        count(VoucherQuery {
            exported: Some("any".into()),
            ..Default::default()
        })?,
        0
    );
    Ok(())
}

/// 排序：默认与历史一致（日期倒序）；金额排序把"金额为空"排在最后
#[test]
fn voucher_query_sort_variants() -> Result<()> {
    use voucher_core::services::query::VoucherQuery;
    let (_dir, ctx) = test_ctx();
    mk(&ctx, "记-001", "2026-08-05", Some(10_000), "甲", "租金")?;
    mk(&ctx, "记-002", "2026-08-20", Some(20_000), "乙", "水电")?;
    mk(&ctx, "记-003", "2026-08-10", None, "丙", "杂费")?;

    let ids = |q: VoucherQuery| -> Result<Vec<String>> {
        Ok(voucher_mgr::list(&ctx, &q)?
            .0
            .into_iter()
            .map(|v| v.voucher_no)
            .collect())
    };

    // 默认：日期倒序（历史行为，不得改）
    assert_eq!(
        ids(VoucherQuery::default())?,
        vec!["记-002", "记-003", "记-001"]
    );
    // 金额倒序：200 元 → 100 元 → 空金额最后
    assert_eq!(
        ids(VoucherQuery {
            sort: Some("amount_desc".into()),
            ..Default::default()
        })?,
        vec!["记-002", "记-001", "记-003"]
    );
    // 金额升序：空金额同样排在最后（NULL 不应被当成"最小"顶到最前）
    assert_eq!(
        ids(VoucherQuery {
            sort: Some("amount_asc".into()),
            ..Default::default()
        })?,
        vec!["记-001", "记-002", "记-003"]
    );
    // 日期升序
    assert_eq!(
        ids(VoucherQuery {
            sort: Some("date_asc".into()),
            ..Default::default()
        })?,
        vec!["记-001", "记-003", "记-002"]
    );
    Ok(())
}

/// 月度透视：月份 × 对方 / 月份 × 标签（每月前 N 个键，按金额倒序）
#[test]
fn monthly_breakdown_by_counterparty_and_tag() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    mk(&ctx, "记-001", "2026-08-05", Some(30_000), "甲物业", "租金")?;
    mk(&ctx, "记-002", "2026-08-06", Some(10_000), "乙公司", "水电")?;
    mk(&ctx, "记-003", "2026-09-02", Some(50_000), "甲物业", "租金")?;
    // 空对方账户 → 透视里应归为「(未填)」，暴露数据缺口
    mk(&ctx, "记-004", "2026-09-03", None, "", "杂费")?;
    ctx.db.with_conn(|conn| {
        conn.execute(
            "UPDATE vouchers SET tags=json_array('差旅') WHERE voucher_no='记-003'",
            [],
        )?;
        conn.execute(
            "UPDATE vouchers SET tags=json_array('差旅','报销') WHERE voucher_no='记-004'",
            [],
        )?;
        Ok(())
    })?;

    let rows = dashboard::monthly_breakdown(&ctx, None, None, "counterparty", 2)?;
    // 9 月在前：甲物业 50000 / (未填) 0；8 月：甲物业 30000 / 乙公司 10000
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].month, "2026-09");
    assert_eq!(rows[0].key, "甲物业");
    assert_eq!((rows[0].count, rows[0].amount_cents), (1, 50_000));
    assert!(
        rows.iter()
            .any(|r| r.month == "2026-09" && r.key == "(未填)"),
        "空对方应归为(未填)"
    );

    // 每月只取前 1 个 → 每月只剩金额最大的甲物业（8 月 30000 / 9 月 50000）
    let top1 = dashboard::monthly_breakdown(&ctx, None, None, "counterparty", 1)?;
    assert_eq!(top1.len(), 2);
    assert!(top1.iter().all(|r| r.key == "甲物业"));

    // 标签透视：一张多标签分别计数（与 tag_stats 同口径）
    let tags = dashboard::monthly_breakdown(&ctx, None, None, "tag", 5)?;
    let sep: Vec<_> = tags.iter().filter(|r| r.month == "2026-09").collect();
    assert_eq!(sep.len(), 2, "9 月两个标签各一行（差旅 / 报销）");
    // 一张凭证的多个标签**分别计数**（与 tag_stats 同口径）：记-003 差旅、记-004 差旅+报销
    let df = sep.iter().find(|r| r.key == "差旅").expect("差旅行");
    let bx = sep.iter().find(|r| r.key == "报销").expect("报销行");
    assert_eq!((df.count, bx.count), (2, 1));
    Ok(())
}

/// 导出覆盖度：从未导出的口径（含归档）与格式分布
#[test]
fn export_coverage_reports_never_exported() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let a = mk(&ctx, "记-001", "2026-08-05", Some(10_000), "甲", "租金")?;
    mk(&ctx, "记-002", "2026-08-06", Some(20_000), "乙", "水电")?;

    let before = dashboard::export_coverage(&ctx, None, None)?;
    assert_eq!(
        (before.total, before.exported, before.never_exported),
        (2, 0, 2)
    );
    assert!(before.by_format.is_empty(), "还没有批次");

    // 造一条导出批次 + 明细（模拟已导出 a）
    ctx.db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO export_batches (batch_no, voucher_count, pdf_path, status, created_at, format)
             VALUES ('EXP-1', 1, '/tmp/x.pdf', 'done', '2026-08-05T10:00:00', 'pdf')",
            [],
        )?;
        let batch_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO export_items (batch_id, voucher_id, sort_order) VALUES (?1, ?2, 0)",
            rusqlite::params![batch_id, a],
        )?;
        Ok(())
    })?;

    let after = dashboard::export_coverage(&ctx, None, None)?;
    assert_eq!(
        (after.total, after.exported, after.never_exported),
        (2, 1, 1)
    );
    assert_eq!(after.by_format.len(), 1);
    assert_eq!(after.by_format[0].format, "pdf");
    assert_eq!(after.by_format[0].vouchers, 1);

    // 窗口收窄到只有"从未导出"的那张的日期 → 覆盖度随之变化（口径与窗口联动）
    let windowed =
        dashboard::export_coverage(&ctx, Some("2026-08-06".into()), Some("2026-08-31".into()))?;
    assert_eq!((windowed.total, windowed.never_exported), (1, 1));
    Ok(())
}

/// 存储占用：原图体积走 SQL；增强版体积默认不计算（需显式开启）
#[test]
fn storage_usage_counts_and_lazy_bytes() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let a = mk(&ctx, "记-001", "2026-08-05", Some(10_000), "甲", "租金")?;
    attach(&ctx, a, "hash-s-1")?;

    let u = dashboard::storage_usage(&ctx, false)?;
    assert_eq!(u.original_count, 1);
    assert_eq!(u.original_bytes, 10, "attach() 写入 file_size=10");
    assert_eq!(u.enhanced_count, 0);
    assert!(u.enhanced_bytes.is_none(), "未显式请求时不逐文件 stat");
    assert_eq!(u.ocr_cache_count, 0);
    Ok(())
}
