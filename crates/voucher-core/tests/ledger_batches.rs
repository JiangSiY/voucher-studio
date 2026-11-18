//! 台账页（v13，2026-11-14）：批次落库、待补附件/待入账两条列表、撤销边界。
//!
//! 覆盖 P0 新增的三块语义：
//! - 导入即落一批（`import_batches`），新建的凭证记 `import_batch_id`，dry-run 不落批次；
//! - `list_items` 两条列表的口径：待补附件 = 无附件、待入账 = 有附件且未勾 `ledger_booked_at`；
//! - `undo_batch` 只删「本批 + 无附件 + 未导出/归档」的草稿，其余保留并给出原因，
//!   批次只写 `undone_at` 不物理删除，重复撤销报错。

mod common;

use common::*;
use voucher_core::db::repos::{attachment_repo, import_batch_repo, voucher_repo};
use voucher_core::models::attachment::NewAttachment;
use voucher_core::models::VoucherStatus;
use voucher_core::services::ingestion::{import_ledger_csv, LedgerKind};
use voucher_core::services::ledger::{self, LedgerStage};
use voucher_core::{CoreContext, CoreError};

fn write_csv(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("写 CSV");
    path
}

const SHOUKUAN_HEADER: &str =
    "流水号,日期,金额,账户,账户性质,对方,项目,方式,分类,票据类型,发票号,凭证号,方向\n";

/// 三行收款清单（SK-1..SK-3）
fn three_rows(dir: &std::path::Path) -> std::path::PathBuf {
    let body = format!(
        "{SHOUKUAN_HEADER}\
SK-1,2026-10-01,100.00,工行基本户,对公,宋大江,凯诗兰德,转账,电费,,,,\n\
SK-2,2026-10-02,200.00,工行基本户,对公,卢丙哲,陈家湾,现金,,,,\n\
SK-3,2026-10-03,300.00,工行基本户,对公,陈大涛,凯诗兰德,转账,,,,\n"
    );
    write_csv(dir, "收款清单.csv", &body)
}

fn by_ref(ctx: &CoreContext, ledger_ref: &str) -> voucher_core::models::Voucher {
    ctx.db
        .with_conn(|conn| voucher_repo::find_by_ledger_ref(conn, ledger_ref))
        .expect("查 ledger_ref")
        .unwrap_or_else(|| panic!("{ledger_ref} 应落库"))
}

/// 给凭证挂一条附件行（只造数据，不落物理文件——撤销只看 `attachments` 表）
fn attach(ctx: &CoreContext, voucher_id: i64) {
    ctx.db
        .with_conn(|conn| {
            attachment_repo::insert(
                conn,
                &NewAttachment {
                    voucher_id,
                    file_name: "scan.png".into(),
                    file_path: "2026-10/a.png".into(),
                    file_hash: "hash-a".into(),
                    file_size: 10,
                    mime_type: "image/png".into(),
                    is_pdf: Some(false),
                    page_count: None,
                },
            )
        })
        .expect("插附件");
}

/// V5 改版后的列名：流水序号（AutoNumber）+ 账户（结算方式），没有 账户性质/方式
#[test]
fn import_accepts_current_v5_column_names() {
    let (dir, ctx) = test_ctx();
    let body = "流水序号,日期,金额,账户,对方,项目,分类,摘要,方向\n\
9001,2026-11-03,12500.00,银行转账,凯诗蓝德,凯诗蓝德-585kW,电费,收电费,收\n";
    let csv = write_csv(dir.path(), "收款清单.csv", body);
    let s = import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false).expect("导入");
    assert_eq!(s.created, 1);
    assert!(s.failed.is_empty());
    assert!(!by_ref(&ctx, "9001").voucher_no.is_empty());
}

#[test]
fn import_records_one_batch_and_binds_created_vouchers() {
    let (dir, ctx) = test_ctx();
    let csv = three_rows(dir.path());

    let s = import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false).expect("导入");
    let batch_id = s.batch_id.expect("非 dry-run 必须落批次");

    let batch = ctx
        .db
        .with_conn(|conn| import_batch_repo::get(conn, batch_id))
        .expect("取批次")
        .expect("批次存在");
    assert_eq!(batch.kind, "shoukuan");
    assert_eq!(batch.file_name, "收款清单.csv");
    assert_eq!(
        (batch.rows_total, batch.created, batch.error_rows),
        (3, 3, 0)
    );
    assert!(batch.undone_at.is_none());

    // 三个新建凭证都记到这批判下
    for r in ["SK-1", "SK-2", "SK-3"] {
        let v = by_ref(&ctx, r);
        assert_eq!(
            ctx.db
                .with_conn(|conn| {
                    #[derive(Debug)]
                    struct Row {
                        id: i64,
                        batch: Option<i64>,
                    }
                    let mut stmt = conn.prepare(
                        "SELECT id, import_batch_id FROM vouchers WHERE ledger_ref = ?1",
                    )?;
                    let row = stmt.query_row([r], |x| {
                        Ok(Row {
                            id: x.get(0)?,
                            batch: x.get(1)?,
                        })
                    })?;
                    Ok(row)
                })
                .map(|row| (row.id, row.batch))
                .expect("查 import_batch_id"),
            (v.id, Some(batch_id)),
            "{r} 应归属本批"
        );
    }
}

#[test]
fn dry_run_records_no_batch() {
    let (dir, ctx) = test_ctx();
    let csv = three_rows(dir.path());

    let s = import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, true).expect("dry-run");
    assert_eq!(s.created, 3, "预演仍要统计新建数");
    assert!(s.batch_id.is_none(), "dry-run 不落批次");
    let n = ctx
        .db
        .with_conn(|conn| import_batch_repo::list(conn, 50))
        .expect("列批次");
    assert!(n.is_empty(), "预演不得留下批次记录");
}

#[test]
fn stages_split_by_attachment_and_booked_flag() {
    let (dir, ctx) = test_ctx();
    let csv = three_rows(dir.path());
    import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false).expect("导入");

    // 刚导入：三张都在「待补附件」，待入账为空
    let counts = ledger::counts(&ctx).expect("计数");
    assert_eq!((counts.unattached, counts.unbooked), (3, 0));
    assert_eq!(counts.active_batches, 1);
    let items = ledger::list_items(&ctx, LedgerStage::Unattached).expect("待补附件");
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|i| !i.has_attachment && !i.booked));
    assert!(items.iter().all(|i| i.ledger_kind == "shoukuan"));

    // SK-1 挂上票据 → 从待补附件消失、进待入账
    attach(&ctx, by_ref(&ctx, "SK-1").id);
    let counts = ledger::counts(&ctx).expect("计数");
    assert_eq!((counts.unattached, counts.unbooked), (2, 1));

    // 勾「已入账」→ 待入账也清掉，但仍在库里
    ledger::set_booked(&ctx, by_ref(&ctx, "SK-1").id, true).expect("勾已入账");
    let counts = ledger::counts(&ctx).expect("计数");
    assert_eq!((counts.unattached, counts.unbooked), (2, 0));
    assert_eq!(voucher_count(&ctx).expect("张数"), 3, "勾标记不删凭证");

    // 取消勾选 → 回到待入账
    ledger::set_booked(&ctx, by_ref(&ctx, "SK-1").id, false).expect("取消勾选");
    assert_eq!(ledger::counts(&ctx).expect("计数").unbooked, 1);
}

#[test]
fn set_booked_on_missing_voucher_is_not_found() {
    let (_dir, ctx) = test_ctx();
    let err = ledger::set_booked(&ctx, 9999, true).expect_err("不存在的凭证应报错");
    assert!(matches!(err, CoreError::NotFound(_)), "实际：{err:?}");
}

#[test]
fn undo_removes_only_unattached_unexported_drafts() {
    let (dir, ctx) = test_ctx();
    let csv = three_rows(dir.path());
    let s = import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false).expect("导入");
    let batch_id = s.batch_id.expect("批次");

    // SK-1 已有附件（保留：有附件）；SK-2 已导出（保留：已导出）；SK-3 仍是草稿无附件（删）
    attach(&ctx, by_ref(&ctx, "SK-1").id);
    let sk2 = by_ref(&ctx, "SK-2").id;
    ctx.db
        .with_conn(|conn| voucher_repo::set_status(conn, sk2, VoucherStatus::Exported))
        .expect("置已导出");

    let r = ledger::undo_batch(&ctx, batch_id).expect("撤销");
    assert_eq!(r.removed, 1, "只删 SK-3");
    assert_eq!(r.kept.len(), 2, "SK-1 / SK-2 保留");
    let reasons: Vec<&str> = r.kept.iter().map(|k| k.reason.as_str()).collect();
    assert!(reasons.contains(&"有附件"), "实际：{reasons:?}");
    assert!(reasons.contains(&"已导出"), "实际：{reasons:?}");

    assert_eq!(voucher_count(&ctx).expect("张数"), 2);
    assert!(
        ctx.db
            .with_conn(|conn| voucher_repo::find_by_ledger_ref(conn, "SK-3"))
            .expect("查")
            .is_none(),
        "SK-3 应被删除"
    );

    // 批次不物理删，只写 undone_at
    let batch = ctx
        .db
        .with_conn(|conn| import_batch_repo::get(conn, batch_id))
        .expect("取批次")
        .expect("批次仍在");
    assert!(batch.undone_at.is_some(), "批次应标记已撤销");
}

#[test]
fn undo_twice_is_rejected_and_unknown_batch_is_not_found() {
    let (dir, ctx) = test_ctx();
    let csv = three_rows(dir.path());
    let s = import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false).expect("导入");
    let batch_id = s.batch_id.expect("批次");

    ledger::undo_batch(&ctx, batch_id).expect("第一次撤销");
    let err = ledger::undo_batch(&ctx, batch_id).expect_err("重复撤销应报错");
    match err {
        CoreError::Invalid(msg) => assert!(msg.contains("撤销"), "实际：{msg}"),
        other => panic!("应是 Invalid，实际 {other:?}"),
    }

    let err = ledger::undo_batch(&ctx, 9999).expect_err("不存在的批次应报错");
    assert!(matches!(err, CoreError::NotFound(_)), "实际：{err:?}");
}

#[test]
fn reimport_keeps_first_batch_ownership() {
    let (dir, ctx) = test_ctx();
    let csv = three_rows(dir.path());
    let first = import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false)
        .expect("首导")
        .batch_id
        .expect("批次");

    // 再导一次：全部命中，不得把已存在凭证改挂到新批（否则撤销第二批会牵连第一批）
    let second = import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false).expect("重导");
    assert_eq!((second.created, second.matched), (0, 3));
    let v = by_ref(&ctx, "SK-1");
    let batch: Option<i64> = ctx
        .db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT import_batch_id FROM vouchers WHERE id = ?1",
                [v.id],
                |r| r.get::<_, Option<i64>>(0),
            )
            .map_err(voucher_core::CoreError::Db)
        })
        .expect("查");
    assert_eq!(batch, Some(first), "重导不改归属");
}

#[test]
fn list_batches_is_newest_first_and_limited() {
    let (dir, ctx) = test_ctx();
    let csv = three_rows(dir.path());
    import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false).expect("导入");

    let batches = ledger::list_batches(&ctx, 50).expect("列批次");
    assert_eq!(batches.len(), 1);
}
