//! v0.16 疑似重复**处置闭环**（集成）：证据分档 / 判定幂等 / 排除已判定 / 留痕删除 /
//! 封账限制 / 越组守卫。
//!
//! 背景：用户 2026-09-23 反馈"能检测出重复，但没有处理流程，检测出重复后一直挂着提示"。
//! 本文件锁住补齐后的行为契约——判定口径仍是"同日期 + 同金额"（不变），
//! 新增的是**处置**（不是重复 / 确认重复并删除 + 留痕）与**已判定不再打扰**。

mod common;

use common::*;
use voucher_core::db::repos::{attachment_repo, voucher_repo};
use voucher_core::models::enums::DupDecisionKind;
use voucher_core::models::{Attachment, NewAttachment, NewVoucher, Voucher};
use voucher_core::services::{dup_review, period, voucher_mgr};
use voucher_core::{CoreContext, CoreError, Result};

fn mk(
    ctx: &CoreContext,
    no: &str,
    date: &str,
    amount: Option<i64>,
    counterparty: &str,
    description: &str,
) -> Result<Voucher> {
    voucher_mgr::create_voucher(
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
    )
}

/// 给凭证挂一张"图片"：落一份真实文件（删除路径要清理物理文件）+ 一条附件行。
fn attach(ctx: &CoreContext, voucher_id: i64, hash: &str) -> Result<Attachment> {
    let rel = format!("2026-09/{hash}.jpg");
    let abs = ctx.paths.attachment_abs(&rel);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&abs, b"fake-image")?;
    ctx.db.with_conn(|conn| {
        attachment_repo::insert(
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
    })
}

fn is_pair(a: i64, b: i64, x: i64, y: i64) -> bool {
    (a == x && b == y) || (a == y && b == x)
}

/// 证据分档：同图 → 高；(同对方 ∧ 同摘要) → 高；同对方或同摘要 → 中；仅同日同金额 → 低。
/// 分档**只影响排序与提示强弱，不做过滤**（保持"宁多报不漏报"）。
#[test]
fn evidence_tiers_follow_image_and_field_overlap() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let d = "2026-09-01";
    let a = mk(&ctx, "记-001", d, Some(10_000), "甲", "租金")?;
    let b = mk(&ctx, "记-002", d, Some(10_000), "乙", "水电")?; // 与 a 同图
    let c = mk(&ctx, "记-003", d, Some(10_000), "甲", "差旅")?; // 仅同对方
    let e = mk(&ctx, "记-005", d, Some(10_000), "甲", "租金")?; // 同对方 + 同摘要
    let f = mk(&ctx, "记-006", d, Some(10_000), "丁", "杂费")?; // 全不同
    attach(&ctx, a.id, "hash-shared")?;
    attach(&ctx, b.id, "hash-shared")?;
    attach(&ctx, e.id, "hash-other")?;

    let queue = dup_review::review_queue(&ctx)?;
    assert_eq!(queue.len(), 1, "5 张同日期同金额应合成 1 组");
    assert_eq!(queue[0].pending.len(), 10, "5 张应有 C(5,2)=10 对");
    assert_eq!(queue[0].decided_pairs, 0);
    let pair = |x: i64, y: i64| {
        queue[0]
            .pending
            .iter()
            .find(|p| is_pair(p.a.id, p.b.id, x, y))
            .expect("该对应在队列里")
    };

    let ab = &pair(a.id, b.id).evidence;
    assert!(ab.same_image, "同一份图片内容 → same_image");
    assert_eq!(ab.level, dup_review::EvidenceLevel::High);

    let ae = &pair(a.id, e.id).evidence;
    assert!(!ae.same_image, "不同图片内容");
    assert!(ae.same_counterparty && ae.same_description);
    assert_eq!(ae.level, dup_review::EvidenceLevel::High);

    let ac = &pair(a.id, c.id).evidence;
    assert!(ac.same_counterparty && !ac.same_description);
    assert_eq!(ac.level, dup_review::EvidenceLevel::Medium);

    let af = &pair(a.id, f.id).evidence;
    assert!(!af.same_image && !af.same_counterparty && !af.same_description);
    assert_eq!(af.level, dup_review::EvidenceLevel::Low);

    // 排序：高在前、低在后（先看最像的）
    let levels: Vec<&str> = queue[0]
        .pending
        .iter()
        .map(|p| p.evidence.level.label_cn())
        .collect();
    assert_eq!(levels.first(), Some(&"高"), "高置信度应排在最前");
    assert_eq!(levels.last(), Some(&"低"), "低置信度应排在最后");
    Ok(())
}

/// 判定"不是重复"：不再出现在保存提示与待核查队列；同对重复判定幂等（含反向传参）。
#[test]
fn not_duplicate_decision_hides_pair_and_is_idempotent() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let a = mk(&ctx, "记-101", "2026-08-05", Some(123_456), "甲", "租金")?;
    let b = mk(&ctx, "记-102", "2026-08-05", Some(123_456), "乙", "水电")?;

    assert_eq!(
        dup_review::undecided_suspects(&ctx, a.id)?.len(),
        1,
        "未处置时应提示对端"
    );
    // 反向传参（b→a）也必须命中同一对：内部按 (min, max) 归一化
    dup_review::resolve_pair(&ctx, b.id, a.id, "同一天两笔正常付款")?;

    assert!(
        dup_review::undecided_suspects(&ctx, a.id)?.is_empty(),
        "已判定后保存提示不应再出现（用户报的「一直挂着提示」）"
    );
    assert!(
        dup_review::review_queue(&ctx)?.is_empty(),
        "全部对已判定的组不进待核查队列"
    );
    let summary = dup_review::pending_summary(&ctx)?;
    assert_eq!((summary.groups, summary.pairs), (0, 0));

    // 再判一次（正向）：幂等，不新增记录，备注覆盖为最新
    dup_review::resolve_pair(&ctx, a.id, b.id, "复核后确认不是重复")?;
    let hist = dup_review::history(&ctx, 10)?;
    assert_eq!(hist.len(), 1, "同一对只应有一条处置记录");
    assert_eq!(hist[0].decision, DupDecisionKind::NotDuplicate);
    assert_eq!(hist[0].note, "复核后确认不是重复");
    let (na, nb) = (a.id.min(b.id), a.id.max(b.id));
    assert_eq!((hist[0].a_id, hist[0].b_id), (na, nb), "存库按 a<b 归一化");
    Ok(())
}

/// 确认重复并删除：留痕（被删凭证完整 JSON + 附件清单）+ 删凭证 + 清理物理文件，一次事务内完成。
#[test]
fn remove_duplicate_keeps_audit_payload_and_cleans_files() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let keep = mk(&ctx, "记-201", "2026-08-06", Some(88_800), "甲", "租金")?;
    let dup = mk(&ctx, "记-202", "2026-08-06", Some(88_800), "甲", "租金")?;
    let att = attach(&ctx, dup.id, "hash-dup-201")?;
    let abs = ctx.paths.attachment_abs(&att.file_path);
    assert!(abs.exists(), "前置：附件文件已落盘");

    let (decision_id, deleted) = dup_review::remove_duplicate(&ctx, keep.id, dup.id, "重复上传")?;
    assert!(decision_id > 0);
    assert_eq!(deleted, 1);

    assert!(
        ctx.db
            .with_conn(|c| voucher_repo::get(c, dup.id))?
            .is_none(),
        "被删凭证应已移除"
    );
    assert!(
        ctx.db
            .with_conn(|c| voucher_repo::get(c, keep.id))?
            .is_some(),
        "保留的凭证必须还在"
    );

    let hist = dup_review::history(&ctx, 5)?;
    assert_eq!(hist.len(), 1);
    let rec = &hist[0];
    assert_eq!(rec.decision, DupDecisionKind::DuplicateRemoved);
    assert_eq!((rec.keep_id, rec.removed_id), (Some(keep.id), Some(dup.id)));
    let payload = rec.removed_payload.as_ref().expect("被删凭证必须留痕");
    assert_eq!(payload["voucher"]["voucherNo"], "记-202");
    assert_eq!(payload["voucher"]["amountCents"], 88_800);
    assert_eq!(payload["voucher"]["counterparty"], "甲");
    assert_eq!(
        payload["attachments"][0]["fileHash"], "hash-dup-201",
        "附件清单也要留痕（含内容哈希，便于日后核对原始图片）"
    );

    assert!(!abs.exists(), "被删凭证的附件文件应被清理");
    assert!(
        dup_review::review_queue(&ctx)?.is_empty(),
        "处置后该组不应再出现在队列"
    );
    Ok(())
}

/// 审计日志不随凭证删除消失（`dup_decisions` 刻意无外键）。
#[test]
fn audit_survives_deleting_the_kept_voucher() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let keep = mk(&ctx, "记-301", "2026-08-07", Some(3_300), "甲", "租金")?;
    let dup = mk(&ctx, "记-302", "2026-08-07", Some(3_300), "甲", "租金")?;
    dup_review::remove_duplicate(&ctx, keep.id, dup.id, "")?;
    voucher_mgr::delete_vouchers(&ctx, &[keep.id])?;
    let hist = dup_review::history(&ctx, 5)?;
    assert_eq!(hist.len(), 1, "两张凭证都没了，处置留痕仍应在案");
    assert!(hist[0].removed_payload.is_some());
    Ok(())
}

/// 已封账月份：**禁止删除**（与列表页删除同口径），但**允许标注"不是重复"**（不碰凭证）。
/// 失败时事务回滚——不得留下"已确认删除"的假留痕。
#[test]
fn locked_period_blocks_removal_but_allows_marking_not_duplicate() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let a = mk(&ctx, "记-401", "2026-07-10", Some(5_000), "甲", "租金")?;
    let b = mk(&ctx, "记-402", "2026-07-10", Some(5_000), "甲", "租金")?;
    period::lock_period(&ctx, "2026-07")?;

    dup_review::resolve_pair(&ctx, a.id, b.id, "封账月，先只标注")?;

    let err = dup_review::remove_duplicate(&ctx, a.id, b.id, "").expect_err("封账月应拒绝删除");
    assert!(
        matches!(err, CoreError::Invalid(_)),
        "应为 Invalid，实际 {err:?}"
    );

    assert!(
        ctx.db.with_conn(|c| voucher_repo::get(c, b.id))?.is_some(),
        "拒绝删除后凭证必须完好"
    );
    let hist = dup_review::history(&ctx, 5)?;
    assert_eq!(hist.len(), 1);
    assert_eq!(
        hist[0].decision,
        DupDecisionKind::NotDuplicate,
        "删除被拒后不得把留痕改成 duplicate_removed（事务整体回滚）"
    );
    Ok(())
}

/// 越组守卫：只允许删"确实是同一组疑似重复"的凭证（同日期 + 同金额且金额非空）。
#[test]
fn removal_rejects_vouchers_outside_the_group() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let a = mk(&ctx, "记-501", "2026-06-01", Some(100), "甲", "x")?;
    let other_date = mk(&ctx, "记-502", "2026-06-02", Some(100), "甲", "x")?;
    let empty1 = mk(&ctx, "记-503", "2026-06-01", None, "甲", "x")?;
    let empty2 = mk(&ctx, "记-504", "2026-06-01", None, "甲", "x")?;

    let err =
        dup_review::remove_duplicate(&ctx, a.id, other_date.id, "").expect_err("日期不同应拒绝");
    assert!(matches!(err, CoreError::Invalid(_)), "实际 {err:?}");
    let err =
        dup_review::remove_duplicate(&ctx, empty1.id, empty2.id, "").expect_err("金额为空应拒绝");
    assert!(matches!(err, CoreError::Invalid(_)), "实际 {err:?}");
    let err = dup_review::resolve_pair(&ctx, a.id, a.id, "").expect_err("同张应拒绝");
    assert!(matches!(err, CoreError::Invalid(_)), "实际 {err:?}");
    assert!(
        dup_review::history(&ctx, 5)?.is_empty(),
        "被拒的操作不得留下任何处置记录"
    );
    Ok(())
}

/// 一组多张：删掉其中一张后，组内剩余的对仍在队列（工作可以分批推进）。
#[test]
fn removal_keeps_remaining_pairs_pending() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let a = mk(&ctx, "记-601", "2026-05-01", Some(7_700), "甲", "租金")?;
    let b = mk(&ctx, "记-602", "2026-05-01", Some(7_700), "甲", "租金")?;
    let c = mk(&ctx, "记-603", "2026-05-01", Some(7_700), "乙", "水电")?;
    assert_eq!(dup_review::pending_summary(&ctx)?.pairs, 3);

    dup_review::remove_duplicate(&ctx, a.id, c.id, "重复上传")?;
    let queue = dup_review::review_queue(&ctx)?;
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].vouchers.len(), 2, "组内只剩两张");
    assert_eq!(queue[0].pending.len(), 1, "只剩 a-b 一对未判定");
    assert!(is_pair(
        queue[0].pending[0].a.id,
        queue[0].pending[0].b.id,
        a.id,
        b.id
    ));

    dup_review::resolve_pair(&ctx, a.id, b.id, "两张都是本月租金，分别对应两个门面")?;
    assert_eq!(dup_review::pending_summary(&ctx)?.pairs, 0);
    assert!(dup_review::review_queue(&ctx)?.is_empty());
    assert_eq!(dup_review::history(&ctx, 10)?.len(), 2, "两次处置各留一条");
    Ok(())
}
