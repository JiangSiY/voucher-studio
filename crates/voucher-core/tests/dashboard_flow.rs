//! v0.18 工作台只读聚合（集成）：窗口口径 / 待办计数 / 轻量重复计数 / 只取 id。
//!
//! 这套用例的意义不只是"功能可用"，而是**锁住口径**——旧仪表盘的核心病灶是
//! 同一屏三套时间口径且互不说明（统计卡全库、状态分布只算 6 个月窗口、Top 是全历史前 10），
//! 于是"草稿 12 / 分布里草稿 5"会被当成数据错误。这里的用例把每个数字的算法固定下来。

mod common;

use common::*;
use voucher_core::models::enums::VoucherStatus;
use voucher_core::models::{NewAttachment, NewVoucher};
use voucher_core::services::{dashboard, dup_review, period, reports, voucher_mgr};
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

/// 轻量计数 vs 核查队列派生值：逐项相等（口径一致性的唯一判据）。
///
/// 返回轻量计数，便于调用方继续断言具体数值。任何一步处置之后都应调用它——
/// 两条 SQL（`pending_count` 的高置信度对 + 逐组扣减）与队列（`review_queue` 逐对证据）
/// 是三套实现，只有用例能钉住它们不漂移。
fn assert_count_matches_queue(ctx: &CoreContext, step: &str) -> Result<dup_review::PendingCount> {
    let light = dup_review::pending_count(ctx)?;
    let queue = dup_review::review_queue(ctx)?;
    let heavy_pairs: u32 = queue.iter().map(|g| g.pending.len() as u32).sum();
    let heavy_high: u32 = queue
        .iter()
        .flat_map(|g| g.pending.iter())
        .filter(|p| p.evidence.level == dup_review::EvidenceLevel::High)
        .count() as u32;
    assert_eq!(
        light.groups,
        queue.len() as u32,
        "[{step}] 组数须与队列一致"
    );
    assert_eq!(light.pairs, heavy_pairs, "[{step}] 对数须与队列一致");
    assert_eq!(
        light.high_pairs, heavy_high,
        "[{step}] 高置信度对数须与队列逐对算出的结果一致"
    );
    Ok(light)
}

/// 口径一致性（审计新增的守卫）：轻量计数必须等于核查队列派生出的数字——
/// 否则侧栏徽标与核查页会互相打架（旧实现的教训）。
///
/// 审计 A-8（2026-09-27）把它扩成**判定矩阵**：初始态 → 判定一对 → **删除一张**（旧用例
/// 完全没覆盖的路径，也是最容易分叉的一步：删完之后组从 3 张缩成 2 张，两条计数口径
/// 各按自己的方式重算）。
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

    let light = assert_count_matches_queue(&ctx, "初始")?;
    assert_eq!(light.groups, 2, "两个组：0805 三张 + 0901 两张");
    assert_eq!(light.pairs, 4, "C(3,2) + C(2,2) = 3 + 1");
    assert_eq!(
        light.high_pairs, 2,
        "a-b 同图、a-c 同对方同摘要（b-c 无共同证据）"
    );

    // ① 判定一对（不删凭证）后两边同步下降
    dup_review::resolve_pair(&ctx, a, b, "同图但确实是两笔")?;
    let after_resolve = assert_count_matches_queue(&ctx, "判定 a-b 后")?;
    assert_eq!(after_resolve.pairs, light.pairs - 1, "判定一对后应少一对");
    assert_eq!(
        after_resolve.high_pairs,
        light.high_pairs - 1,
        "a-b 是高置信度对"
    );

    // ② 删除一张（确认重复）后依然逐项一致——旧用例未覆盖的路径
    dup_review::remove_duplicate(&ctx, a, c, "同对方同摘要，保留 a")?;
    let after_remove = assert_count_matches_queue(&ctx, "删除 c 后")?;
    // c 消失后 0805 组只剩 a、b，而 a-b 已判定 → 该组再无未判定对，整组退出统计
    assert_eq!(after_remove.groups, 1, "0805 组已无未判定对");
    assert_eq!(after_remove.pairs, 1, "只剩 0901 组那一对");
    assert_eq!(after_remove.high_pairs, 0, "剩余的那对只有同额（弱证据）");
    Ok(())
}

/// 空库（没有附件、没有凭证）时聚合不得报错（2026-09-27 批次 7 由压测用例发现）。
///
/// 病灶：`storage_usage` 的 `SUM(CASE WHEN enhanced_path IS NOT NULL …)` 在空表上返回 NULL，
/// 而读取端按 `i64` 解 → `InvalidColumnType(Null)`。新装用户打开洞察页就会看到 `db` 错误，
/// 而老用例都先造了附件，**完全绕过了这条路径**。
#[test]
fn aggregations_survive_empty_library() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let usage = dashboard::storage_usage(&ctx, false)?;
    assert_eq!(usage.original_count, 0);
    assert_eq!(
        usage.enhanced_count, 0,
        "空库的增强版计数必须是 0，而不是报错"
    );
    assert_eq!(usage.ocr_cache_count, 0);
    let coverage = dashboard::export_coverage(&ctx, None, None)?;
    assert_eq!((coverage.total, coverage.never_exported), (0, 0));
    let overview = dashboard::overview(&ctx, None, None, None)?;
    assert_eq!(overview.total, 0);
    assert!(dashboard::monthly_breakdown(&ctx, None, None, "tag", 5)?.is_empty());
    assert_eq!(dup_review::pending_count(&ctx)?.pairs, 0);
    Ok(())
}

/// 脏数据口径统一（审计 A-9，2026-09-27）：外部工具把 `tags` 写成非法 JSON 之后——
/// 列表仍可用（该行按空标签处理）、标签透视不得抛错、报告页与洞察页表现一致。
///
/// 修前的真实病灶不在 `row_to_voucher` 本身，而在 **两处 SQL 口径不一致**：
/// `reports.rs` 的 `tag_stats` 有 `json_valid` 兜底、`dashboard.rs` 的
/// `monthly_breakdown(dim="tag")` 没有——同一个坏值，报告页降级、洞察页直接报 `db` 错误。
///
/// （`status` 列有 CHECK 约束，注入不了非法值，故那条告警分支只能靠代码评审保证。）
#[test]
fn dirty_tags_degrade_consistently_across_aggregations() -> Result<()> {
    use voucher_core::services::query::VoucherQuery;
    let (_dir, ctx) = test_ctx();
    let good = mk(&ctx, "记-001", "2026-08-05", Some(1_000), "甲", "租金")?;
    let dirty = mk(&ctx, "记-002", "2026-08-05", Some(2_000), "乙", "水电")?;
    ctx.db.with_conn(|conn| {
        conn.execute(
            "UPDATE vouchers SET tags=?1 WHERE id=?2",
            rusqlite::params!["[\"办公\"]", good],
        )?;
        conn.execute(
            "UPDATE vouchers SET tags=?1 WHERE id=?2",
            rusqlite::params!["{不是 JSON", dirty],
        )?;
        Ok(())
    })?;

    // ① 行映射降级但不失败：坏值按空标签处理，其余字段照常读出
    let listed = voucher_mgr::list(&ctx, &VoucherQuery::default())?.0;
    let dirty_row = listed
        .iter()
        .find(|v| v.id == dirty)
        .expect("脏数据行仍应能读出");
    assert!(
        dirty_row.tags.is_empty(),
        "非法 JSON 按空标签处理（不是把整行读失败）"
    );
    assert_eq!(dirty_row.status, VoucherStatus::Draft, "状态列正常");

    // ② 标签透视（洞察页）不得报错，且脏数据行不产生任何键
    let breakdown = dashboard::monthly_breakdown(&ctx, None, None, "tag", 10)?;
    let keys: Vec<&str> = breakdown.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(
        keys,
        vec!["办公"],
        "只有合法标签进透视；脏数据行被兜底成空数组"
    );
    assert_eq!(breakdown[0].count, 1);

    // ③ 报告页 tag_stats 与之一致（同一条脏数据在两条 SQL 路径上表现相同）
    let tags = reports::tag_stats(&ctx, 12, 10, None, None)?;
    let names: Vec<&str> = tags.iter().map(|t| t.tag.as_str()).collect();
    assert!(names.contains(&"办公"), "合法标签应在报告页出现：{names:?}");
    assert!(
        names.iter().all(|n| !n.contains("不是")),
        "脏数据不得进统计：{names:?}"
    );
    Ok(())
}

/// 弱证据组：只有同额（无同图/无同对方同摘要）时高置信度对为 0，
/// 且依然满足 `high_pairs <= pairs`（这正是旧实现那句"一定不会超过"注释所指的情形）
#[test]
fn high_pairs_stays_within_pairs_when_evidence_is_weak() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    // 4 张同日期同金额，彼此没有任何强证据
    for (no, cp, desc) in [
        ("记-001", "甲", "租金"),
        ("记-002", "乙", "水电"),
        ("记-003", "丙", "差旅"),
        ("记-004", "丁", "杂费"),
    ] {
        mk(&ctx, no, "2026-08-05", Some(50_000), cp, desc)?;
    }
    let light = assert_count_matches_queue(&ctx, "纯弱证据组")?;
    assert_eq!(light.groups, 1);
    assert_eq!(light.pairs, 6, "C(4,2)");
    assert_eq!(light.high_pairs, 0, "没有任何一张图或对方+摘要成对");

    // 再叠两张同图的（强证据）：只多出 1 对高置信度，组维度的对数不变
    let x = mk(&ctx, "记-005", "2026-08-05", Some(50_000), "戊", "耗材")?;
    let y = mk(&ctx, "记-006", "2026-08-05", Some(50_000), "己", "办公")?;
    attach(&ctx, x, "hash-img-x")?;
    attach(&ctx, y, "hash-img-x")?;
    let light2 = assert_count_matches_queue(&ctx, "叠加同图两张")?;
    assert_eq!(light2.pairs, 15, "C(6,2)");
    assert_eq!(light2.high_pairs, 1, "只有 x-y 这一对同图");
    Ok(())
}

/// 整组判完 → 组与对都归零；同日期同金额的另一组不受影响
#[test]
fn pending_count_matches_queue_when_group_fully_decided() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let a = mk(&ctx, "记-001", "2026-08-05", Some(10_000), "甲", "租金")?;
    let b = mk(&ctx, "记-002", "2026-08-05", Some(10_000), "乙", "水电")?;
    mk(&ctx, "记-003", "2026-09-01", Some(20_000), "丙", "差旅")?;
    mk(&ctx, "记-004", "2026-09-01", Some(20_000), "丁", "杂费")?;

    let before = assert_count_matches_queue(&ctx, "两组各一对")?;
    assert_eq!((before.groups, before.pairs), (2, 2));

    dup_review::resolve_pair(&ctx, a, b, "两张独立费用")?;
    let after = assert_count_matches_queue(&ctx, "0805 组判完")?;
    assert_eq!((after.groups, after.pairs), (1, 1), "只剩 0901 那一组");
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

    // 对方未填（透视里的「(未填)」行要能精确下钻）
    assert_eq!(
        count(VoucherQuery {
            counterparty_empty: true,
            ..Default::default()
        })?,
        0,
        "本例三张都填了对方"
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

/// 排序：默认与历史一致（日期倒序）；金额排序把"金额为空"排在最后；
/// 五个 sort 取值全覆盖（含 `updated_desc`，2026-09-27 批次 2 补）
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
    // 「最近修改」：唯一按 updated_at 排序的分支（覆盖缺口，2026-09-27 批次 2 补齐）。
    // 新建的三张 updated_at 可能落在同一秒，故显式写入可区分的时间戳
    ctx.db.with_conn(|conn| {
        conn.execute(
            "UPDATE vouchers SET updated_at = CASE voucher_no
                 WHEN '记-001' THEN '2026-09-02T10:00:00.000Z'
                 WHEN '记-002' THEN '2026-09-01T10:00:00.000Z'
                 ELSE '2026-09-03T10:00:00.000Z' END",
            [],
        )?;
        Ok(())
    })?;
    assert_eq!(
        ids(VoucherQuery {
            sort: Some("updated_desc".into()),
            ..Default::default()
        })?,
        vec!["记-003", "记-001", "记-002"]
    );
    // 取 id（跨页全选的基础）**不受排序影响**——显示顺序变了，选中集合与顺序都不变，
    // 否则"全选后批量处理"会随下拉里的排序悄悄改变处理顺序
    let all_ids = |sort: &str| -> Result<Vec<i64>> {
        voucher_core::services::query::voucher_ids(
            &ctx,
            &VoucherQuery {
                sort: Some(sort.into()),
                ..Default::default()
            },
        )
    };
    assert_eq!(
        all_ids("amount_desc")?,
        all_ids("date_desc")?,
        "取 id 固定按日期倒序，不随 sort 变化"
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

// ---------- 2026-09-27 审计 P1：**待办计数必须等于对应筛选的列表**（口径等价交叉用例） ----------
//
// 背景：审计发现三处"计数与点进去的列表不一致"——两类 SQL 各写各的、没人比对。
// 这批用例把"计数 == 该筛选下的 list total"变成断言，任何一边改动而另一边没跟上都会红。

/// P1-1：待识别计数 = pending + failed，故下钻必须带**多值**状态
#[test]
fn ocr_pending_count_matches_list_with_multi_status() -> Result<()> {
    use voucher_core::services::query::VoucherQuery;
    let (_dir, ctx) = test_ctx();
    let a = mk(&ctx, "记-001", "2026-08-05", Some(1_000), "甲", "租金")?;
    let b = mk(&ctx, "记-002", "2026-08-06", Some(2_000), "乙", "水电")?;
    let c = mk(&ctx, "记-003", "2026-08-07", Some(3_000), "丙", "差旅")?;
    attach(&ctx, a, "hash-ocr-a")?; // pending（默认）
    attach(&ctx, b, "hash-ocr-b")?; // 下面标成 failed
    attach(&ctx, c, "hash-ocr-c")?; // 下面标成 done
    ctx.db.with_conn(|conn| {
        conn.execute(
            "UPDATE attachments SET ocr_status='failed' WHERE voucher_id=?1",
            [b],
        )?;
        conn.execute(
            "UPDATE attachments SET ocr_status='done' WHERE voucher_id=?1",
            [c],
        )?;
        Ok(())
    })?;

    let todos = dashboard::todos(&ctx)?;
    assert_eq!(todos.ocr_pending, 2, "待识别 = pending(1) + failed(1)");

    // 多值筛选：与计数同口径
    let both = voucher_mgr::list(
        &ctx,
        &VoucherQuery {
            ocr_status: Some("pending,failed".into()),
            ..Default::default()
        },
    )?
    .1;
    assert_eq!(
        both as u64, todos.ocr_pending,
        "计数必须等于「pending,failed」的列表总数"
    );

    // 单值会少算——正是审计里"待识别 3 张、点进去 1 张"的成因，用例固定住这个差异
    let only_pending = voucher_mgr::list(
        &ctx,
        &VoucherQuery {
            ocr_status: Some("pending".into()),
            ..Default::default()
        },
    )?
    .1;
    assert_eq!(only_pending, 1, "只带 pending 会漏掉 failed 那张");
    Ok(())
}

/// P1-2：陈旧草稿计数 = `draft` 且 `updated_at < 现在−7 天`，故下钻必须带 `staleDays`
#[test]
fn stale_draft_count_matches_list_with_stale_days() -> Result<()> {
    use voucher_core::services::query::VoucherQuery;
    let (_dir, ctx) = test_ctx();
    let old_draft = mk(&ctx, "记-001", "2026-08-05", Some(1_000), "甲", "租金")?;
    let old_reviewed = mk(&ctx, "记-002", "2026-08-06", Some(2_000), "乙", "水电")?;
    mk(&ctx, "记-003", "2026-08-07", Some(3_000), "丙", "差旅")?; // 新建草稿（不陈旧）
    let old_ts = (chrono::Local::now() - chrono::Duration::days(10))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    ctx.db.with_conn(|conn| {
        conn.execute(
            "UPDATE vouchers SET updated_at=?1 WHERE id IN (?2, ?3)",
            rusqlite::params![old_ts, old_draft, old_reviewed],
        )?;
        conn.execute(
            "UPDATE vouchers SET status='reviewed' WHERE id=?1",
            [old_reviewed],
        )?;
        Ok(())
    })?;

    let todos = dashboard::todos(&ctx)?;
    assert_eq!(todos.stale_drafts, 1, "只有那张 10 天没动的草稿算陈旧");

    let listed = voucher_mgr::list(
        &ctx,
        &VoucherQuery {
            status: Some(VoucherStatus::Draft),
            stale_days: Some(todos.stale_draft_days),
            ..Default::default()
        },
    )?
    .1;
    assert_eq!(
        listed as u64, todos.stale_drafts,
        "计数必须等于同口径列表总数"
    );

    // 不带 staleDays 的"全部草稿"会是 2 张——正是审计里"陈旧 1 张、点进去 2 张"的成因
    let all_drafts = voucher_mgr::list(
        &ctx,
        &VoucherQuery {
            status: Some(VoucherStatus::Draft),
            ..Default::default()
        },
    )?
    .1;
    assert_eq!(all_drafts, 2);
    Ok(())
}

/// P1-4：`export_coverage.never_exported` 必须等于 `exported=never` 的列表总数
/// （此前只测了"谁都没导出"与"any=0"，**"已导出必须被 never 排除"这一分支无人覆盖**）
#[test]
fn never_exported_matches_list_after_an_export() -> Result<()> {
    use voucher_core::services::query::VoucherQuery;
    let (_dir, ctx) = test_ctx();
    let exported = mk(&ctx, "记-001", "2026-08-05", Some(10_000), "甲", "租金")?;
    mk(&ctx, "记-002", "2026-08-06", Some(20_000), "乙", "水电")?;
    mk(&ctx, "记-003", "2026-08-07", Some(30_000), "丙", "差旅")?;

    // 造一条成功批次并包含 记-001
    ctx.db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO export_batches (batch_no, voucher_count, pdf_path, status, created_at, format)
             VALUES ('EXP-AUDIT-1', 1, '/tmp/a.pdf', 'done', '2026-08-05T10:00:00', 'pdf')",
            [],
        )?;
        let batch_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO export_items (batch_id, voucher_id, sort_order) VALUES (?1, ?2, 0)",
            rusqlite::params![batch_id, exported],
        )?;
        Ok(())
    })?;

    let cov = dashboard::export_coverage(&ctx, None, None)?;
    let never_listed = voucher_mgr::list(
        &ctx,
        &VoucherQuery {
            exported: Some("never".into()),
            ..Default::default()
        },
    )?
    .1 as u64;
    let any_listed = voucher_mgr::list(
        &ctx,
        &VoucherQuery {
            exported: Some("any".into()),
            ..Default::default()
        },
    )?
    .1 as u64;

    assert_eq!(
        cov.never_exported, never_listed,
        "聚合与筛选两条 SQL 必须同口径"
    );
    assert_eq!(cov.exported, any_listed, "「导出过」也要同口径");
    assert_eq!(
        (cov.total, cov.never_exported, cov.exported),
        (3, 2, 1),
        "已导出的那张必须被 never 排除"
    );
    Ok(())
}
