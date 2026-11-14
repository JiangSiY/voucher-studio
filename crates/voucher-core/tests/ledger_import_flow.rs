//! 台账导入（财务工作流集成）集成测试：字段映射、幂等、坏行不中断、dry-run。
//!
//! 覆盖 2026-11-08 引入的 `import_ledger_csv` 语义（v0.24.0 之后的未发版批次）：
//! - 收款/付款/计提三类清单都能导入，`ledger_ref`（流水号/单据号）落库；
//! - 日期容忍 `2026/10/1`、`2026-10-01 12:00:00` 等写法并规范化；
//! - 同一 CSV 再跑一次：全部命中、不新增（幂等键 = `vouchers.ledger_ref` 唯一索引）；
//! - 坏行（空日期 / 金额非法 / 幂等键为空）**记错误继续**，不 panic、不中断整批；
//! - `dry_run` 只统计不写库。

mod common;

use common::*;
use voucher_core::db::repos::voucher_repo;
use voucher_core::services::ingestion::{import_ledger_csv, LedgerKind};
use voucher_core::{CoreContext, Result};

/// 把 CSV 文本写成临时文件，返回路径
fn write_csv(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("写 CSV");
    path
}

/// 按幂等键取凭证
fn by_ref(ctx: &CoreContext, ledger_ref: &str) -> Option<voucher_core::models::Voucher> {
    ctx.db
        .with_conn(|conn| voucher_repo::find_by_ledger_ref(conn, ledger_ref))
        .expect("查 ledger_ref")
}

const SHOUKUAN_HEADER: &str =
    "流水号,日期,金额,账户,账户性质,对方,项目,方式,分类,票据类型,发票号,凭证号,方向\n";

#[test]
fn shoukuan_rows_map_to_vouchers() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let body = format!(
        "{SHOUKUAN_HEADER}\
SK-1,2026/10/1,1234.56,工行基本户,对公,宋大江,凯诗兰德,转账,电费,增值税专票,FP-001,,\n\
SK-2,2026-10-02 12:00:00,0.1,工行基本户,对公,卢丙哲,陈家湾,现金,,,,\n"
    );
    let csv = write_csv(dir.path(), "收款清单.csv", &body);

    let s = import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false)?;
    assert_eq!((s.created, s.matched, s.skipped), (2, 0, 0), "两行都应新建");
    assert!(s.errors.is_empty(), "不应有错误：{:?}", s.errors);
    assert_eq!(voucher_count(&ctx)?, 2);

    let v1 = by_ref(&ctx, "SK-1").expect("SK-1 应落库");
    assert_eq!(v1.voucher_date, "2026-10-01", "斜杠日期应规范化");
    assert_eq!(v1.amount_cents, Some(123_456));
    assert_eq!(v1.counterparty, "宋大江");
    assert_eq!(v1.description, "宋大江-凯诗兰德-SK-1");
    for tag in [
        "ledger:auto",
        "kind:shoukuan",
        "account:工行基本户",
        "project:凯诗兰德",
        "invoice_type:增值税专票",
        "invoice_no:FP-001",
    ] {
        assert!(
            v1.tags.iter().any(|t| t == tag),
            "缺标签 {tag}：{:?}",
            v1.tags
        );
    }

    let v2 = by_ref(&ctx, "SK-2").expect("SK-2 应落库");
    assert_eq!(v2.voucher_date, "2026-10-02", "带时间的日期应只取日期段");
    assert_eq!(v2.amount_cents, Some(10), "0.1 元 = 10 分");
    assert!(
        !v2.tags.iter().any(|t| t.starts_with("invoice_")),
        "空票据不应造标签"
    );
    Ok(())
}

#[test]
fn reimport_is_idempotent() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let body = format!(
        "{SHOUKUAN_HEADER}\
SK-1,2026-10-01,100.00,工行基本户,对公,宋大江,凯诗兰德,转账,,,,\n\
SK-2,2026-10-02,200.00,工行基本户,对公,宋大江,凯诗兰德,转账,,,,\n"
    );
    let csv = write_csv(dir.path(), "收款清单.csv", &body);

    let first = import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false)?;
    assert_eq!((first.created, first.matched), (2, 0));
    let total = voucher_count(&ctx)?;

    // 第二次：同一文件再导一遍，应全部命中现有凭证，不新增、不报错
    let second = import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false)?;
    assert_eq!((second.created, second.matched, second.skipped), (0, 2, 0));
    assert!(
        second.errors.is_empty(),
        "幂等重跑不应报错：{:?}",
        second.errors
    );
    assert_eq!(voucher_count(&ctx)?, total, "重跑不得新增凭证");
    Ok(())
}

#[test]
fn bad_rows_do_not_abort_batch() -> Result<()> {
    let (dir, ctx) = test_ctx();
    // 行 1 空日期、行 2 金额非法、行 3 幂等键为空、行 4 正常
    let body = format!(
        "{SHOUKUAN_HEADER}\
SK-BAD-DATE,,100.00,工行基本户,对公,宋大江,凯诗兰德,转账,,,,\n\
SK-BAD-AMOUNT,2026-10-01,abc,工行基本户,对公,宋大江,凯诗兰德,转账,,,,\n\
,2026-10-01,100.00,工行基本户,对公,宋大江,凯诗兰德,转账,,,,\n\
SK-OK,2026-10-01,100.00,工行基本户,对公,宋大江,凯诗兰德,转账,,,,\n"
    );
    let csv = write_csv(dir.path(), "收款清单.csv", &body);

    // 旧实现：空日期会让 `row.date[..7]` panic，整批导入中断（实测 exit 101）
    let s = import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false)?;
    assert_eq!(s.created, 1, "只有正常行新建");
    assert_eq!(s.skipped, 3, "三行坏数据进 skipped");
    assert_eq!(s.errors.len(), 3, "错误应逐行记录：{:?}", s.errors);
    assert_eq!(s.errors[0].ledger_ref, "SK-BAD-DATE");
    assert!(
        s.errors[0].reason.contains("日期"),
        "空日期原因：{}",
        s.errors[0].reason
    );
    assert_eq!(s.errors[1].ledger_ref, "SK-BAD-AMOUNT");
    assert!(
        s.errors[1].reason.contains("金额"),
        "金额原因：{}",
        s.errors[1].reason
    );
    assert!(
        s.errors[2].reason.contains("幂等键"),
        "空键原因：{}",
        s.errors[2].reason
    );
    assert_eq!(voucher_count(&ctx)?, 1, "坏行不得落库");

    // 错误明细落盘（exports/import-errors-*.csv）
    let exports = ctx.paths.exports.clone();
    let hits = std::fs::read_dir(&exports)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("import-errors-")
        })
        .count();
    assert!(hits >= 1, "错误明细应落盘到 {}", exports.display());
    Ok(())
}

#[test]
fn dry_run_does_not_write() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let body = format!(
        "{SHOUKUAN_HEADER}\
SK-1,2026-10-01,100.00,工行基本户,对公,宋大江,凯诗兰德,转账,,,,\n"
    );
    let csv = write_csv(dir.path(), "收款清单.csv", &body);

    let s = import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, true)?;
    assert_eq!(s.created, 1, "dry-run 也要给出将要新建的数量");
    assert_eq!(voucher_count(&ctx)?, 0, "dry-run 不得写库");
    assert!(by_ref(&ctx, "SK-1").is_none());
    Ok(())
}

#[test]
fn jiti_rows_use_doc_no_and_month() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let body = "\
单据号,日期,金额,未结,类型,分类,对方,项目,摘要,凭证号\n\
JD-1,2026-09-30,500.00,500.00,应付,材料,陈大涛,凯诗兰德,组件款,\n";
    let csv = write_csv(dir.path(), "计提清单.csv", body);

    let s = import_ledger_csv(&ctx, &csv, LedgerKind::Jiti, false)?;
    assert_eq!(
        (s.created, s.skipped),
        (1, 0),
        "计提行应新建：{:?}",
        s.errors
    );
    let v = by_ref(&ctx, "JD-1").expect("JD-1 应落库");
    assert_eq!(v.voucher_date, "2026-09-30");
    assert_eq!(v.description, "计提应付-2026-09-JD-1");
    assert_eq!(v.amount_cents, Some(50_000));
    assert!(
        v.tags.iter().any(|t| t == "kind:计提"),
        "标签：{:?}",
        v.tags
    );
    assert!(
        v.tags.iter().any(|t| t == "category:材料"),
        "标签：{:?}",
        v.tags
    );
    Ok(())
}

#[test]
fn missing_required_column_is_rejected() -> Result<()> {
    let (dir, ctx) = test_ctx();
    // 表头缺「对方」「项目」
    let body = "流水号,日期,金额,账户\nSK-1,2026-10-01,100.00,工行基本户\n";
    let csv = write_csv(dir.path(), "收款清单.csv", body);

    let err =
        import_ledger_csv(&ctx, &csv, LedgerKind::Shoukuan, false).expect_err("缺列应整体拒绝");
    assert!(err.to_string().contains("缺必需列"), "错误文案：{err}");
    assert_eq!(voucher_count(&ctx)?, 0);
    Ok(())
}
