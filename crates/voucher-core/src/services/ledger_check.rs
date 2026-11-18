//! 台账对账（M4）：把「台账导出 CSV」与本地凭证库逐条比对。
//!
//! 与 `reconcile.rs`（微信/支付宝账单对账）互补：那边比的是**钱**，
//! 这边比的是**台账记录与凭证卡是否一一对应**，回答三个问题：
//!
//! 1. 台账里有、凭证库里没有 → 漏建卡（`missing_voucher`）
//! 2. 两边都有但对不上 → 金额/日期/对方不一致（`*_mismatch`）
//! 3. 凭证库里有、这张台账表里没有 → 多余/串表（`orphan_voucher`）
//!
//! 口径约定：
//! - 主键是 `ledger_ref`；导入时写成 `doc:<编号>` / `cash:<编号>`，比对时两种前缀都试，
//!   也容忍 CSV 里直接给裸编号（老的 `台账导出.py` 产物）。
//! - 金额按分比较；容差由调用方给（默认 0，只允许完全相等）。
//! - 日期先规范化成 `YYYY-MM-DD` 再比；台账里 `2026/11/3` 与凭证 `2026-11-03` 视为同一天。
//! - 台账 CSV 缺编号列时不静默跳过：整批记为 `missing_ref`，提示先修导出脚本。
//! - 只读：绝不改凭证、不改台账。

use std::collections::HashMap;

use rusqlite::params;
use serde::Serialize;

use super::ingestion::{decode_csv_text, parse_amount_cents, parse_csv_line, parse_ledger_date};
use crate::{CoreContext, CoreError, Result};

/// 一条差异
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckIssue {
    /// missing_voucher / missing_ref / amount_mismatch / date_mismatch /
    /// counterparty_mismatch / orphan_voucher / duplicate_ref
    pub kind: String,
    pub ledger_ref: String,
    pub voucher_id: Option<i64>,
    pub voucher_no: String,
    pub ledger_date: String,
    pub voucher_date: String,
    pub ledger_amount_cents: Option<i64>,
    pub voucher_amount_cents: Option<i64>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    pub csv_path: String,
    pub checked_at: String,
    /// CSV 数据行数（不含表头）
    pub rows: usize,
    /// 完全对上的条数
    pub matched: usize,
    /// 台账有、凭证无
    pub missing: usize,
    /// 凭证有、台账无
    pub orphans: usize,
    /// 字段不一致（金额/日期/对方）
    pub mismatched: usize,
    /// 编号缺失或重复
    pub malformed: usize,
    pub tolerance_cents: i64,
    pub issues: Vec<CheckIssue>,
}

/// CSV 里的一行台账记录（只留比对需要的列）
struct CsvRow {
    line: usize,
    ledger_ref: String,
    date: String,
    amount_cents: Option<i64>,
    counterparty: String,
}

struct VoucherRow {
    id: i64,
    voucher_no: String,
    voucher_date: String,
    amount_cents: Option<i64>,
    counterparty: String,
}

/// 允许的编号列名（新版 V5 是 `流水序号`，旧导出脚本写 `流水号`）
const REF_COLUMNS: &[&str] = &["流水号", "流水序号", "单据号", "编号"];
const DATE_COLUMNS: &[&str] = &["日期", "单据日期", "记账日期"];
const AMOUNT_COLUMNS: &[&str] = &["金额", "单据金额", "发生额"];
const PARTY_COLUMNS: &[&str] = &["对方", "对方名称", "客户", "供应商"];

pub fn check(
    ctx: &CoreContext,
    csv_path: &std::path::Path,
    tolerance_cents: i64,
) -> Result<CheckReport> {
    if !csv_path.exists() {
        return Err(CoreError::NotFound(format!(
            "CSV 不存在：{}",
            csv_path.display()
        )));
    }
    let bytes = std::fs::read(csv_path)?;
    let text = decode_csv_text(&bytes);
    let mut lines = text.split('\n').collect::<Vec<_>>();
    if lines.is_empty() {
        return Err(CoreError::Invalid("CSV 为空".into()));
    }
    let header = parse_csv_line(lines.remove(0));
    let idx = |names: &[&str]| -> Option<usize> {
        header
            .iter()
            .position(|h| names.iter().any(|n| h.trim() == *n))
    };
    let ref_col = idx(REF_COLUMNS);
    let date_col = idx(DATE_COLUMNS)
        .ok_or_else(|| CoreError::Invalid(format!("CSV 缺日期列（表头：{}）", header.join(","))))?;
    let amount_col = idx(AMOUNT_COLUMNS);
    let party_col = idx(PARTY_COLUMNS);

    let mut rows: Vec<CsvRow> = Vec::new();
    for (offset, raw) in lines.iter().enumerate() {
        let line_no = offset + 2;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cells = parse_csv_line(trimmed);
        let cell = |i: Option<usize>| -> String {
            i.and_then(|i| cells.get(i))
                .map(|v| v.trim().to_string())
                .unwrap_or_default()
        };
        let date = match parse_ledger_date(&cell(Some(date_col))) {
            Ok((d, _)) => d,
            Err(_) => cell(Some(date_col)),
        };
        let amount_raw = cell(amount_col);
        let amount_cents = if amount_raw.is_empty() {
            None
        } else {
            parse_amount_cents(&amount_raw).ok()
        };
        rows.push(CsvRow {
            line: line_no,
            ledger_ref: cell(ref_col),
            date,
            amount_cents,
            counterparty: cell(party_col),
        });
    }

    let vouchers = ctx.db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT v.id, v.voucher_no, v.voucher_date, v.amount_cents, v.counterparty, v.ledger_ref
               FROM vouchers v
              WHERE TRIM(COALESCE(v.ledger_ref,'')) <> ''",
        )?;
        let rows = stmt.query_map(params![], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut map: HashMap<String, VoucherRow> = HashMap::new();
        for row in rows {
            let (id, voucher_no, voucher_date, amount_cents, counterparty, ledger_ref) = row?;
            map.insert(
                ledger_ref,
                VoucherRow {
                    id,
                    voucher_no,
                    voucher_date,
                    amount_cents,
                    counterparty,
                },
            );
        }
        Ok(map)
    })?;

    let mut issues: Vec<CheckIssue> = Vec::new();
    let mut matched = 0usize;
    let mut consumed: HashMap<String, usize> = HashMap::new();
    let mut seen_refs: HashMap<String, usize> = HashMap::new();

    for row in &rows {
        if row.ledger_ref.is_empty() {
            issues.push(CheckIssue {
                kind: "missing_ref".into(),
                ledger_ref: String::new(),
                voucher_id: None,
                voucher_no: String::new(),
                ledger_date: row.date.clone(),
                voucher_date: String::new(),
                ledger_amount_cents: row.amount_cents,
                voucher_amount_cents: None,
                message: format!(
                    "第 {line} 行没有编号：按新版台账导出脚本的列名重新导出（流水序号 / 单据号）",
                    line = row.line
                ),
            });
            continue;
        }
        if let Some(prev) = seen_refs.insert(row.ledger_ref.clone(), row.line) {
            issues.push(CheckIssue {
                kind: "duplicate_ref".into(),
                ledger_ref: row.ledger_ref.clone(),
                voucher_id: None,
                voucher_no: String::new(),
                ledger_date: row.date.clone(),
                voucher_date: String::new(),
                ledger_amount_cents: row.amount_cents,
                voucher_amount_cents: None,
                message: format!(
                    "编号重复：第 {prev} 行与第 {} 行都是 {}",
                    row.line, row.ledger_ref
                ),
            });
            continue;
        }

        // 三种写法都试：裸编号（旧导出）、cash:/doc: 前缀（v0.26 导入）
        let candidates = [
            row.ledger_ref.clone(),
            format!("cash:{}", row.ledger_ref),
            format!("doc:{}", row.ledger_ref),
        ];
        let hit = candidates
            .iter()
            .find_map(|key| vouchers.get(key).map(|v| (key.clone(), v)));

        let Some((key, v)) = hit else {
            issues.push(CheckIssue {
                kind: "missing_voucher".into(),
                ledger_ref: row.ledger_ref.clone(),
                voucher_id: None,
                voucher_no: String::new(),
                ledger_date: row.date.clone(),
                voucher_date: String::new(),
                ledger_amount_cents: row.amount_cents,
                voucher_amount_cents: None,
                message: format!("台账有、凭证库没有：{}", row.ledger_ref),
            });
            continue;
        };
        *consumed.entry(key.clone()).or_insert(0) += 1;

        let mut row_ok = true;
        if let (Some(a), Some(b)) = (row.amount_cents, v.amount_cents) {
            if (a - b).abs() > tolerance_cents {
                row_ok = false;
                issues.push(CheckIssue {
                    kind: "amount_mismatch".into(),
                    ledger_ref: row.ledger_ref.clone(),
                    voucher_id: Some(v.id),
                    voucher_no: v.voucher_no.clone(),
                    ledger_date: row.date.clone(),
                    voucher_date: v.voucher_date.clone(),
                    ledger_amount_cents: Some(a),
                    voucher_amount_cents: Some(b),
                    message: format!("金额不一致：台账 {} / 凭证 {}", yuan(a), yuan(b)),
                });
            }
        } else if v.amount_cents.is_none() {
            row_ok = false;
            issues.push(CheckIssue {
                kind: "amount_mismatch".into(),
                ledger_ref: row.ledger_ref.clone(),
                voucher_id: Some(v.id),
                voucher_no: v.voucher_no.clone(),
                ledger_date: row.date.clone(),
                voucher_date: v.voucher_date.clone(),
                ledger_amount_cents: row.amount_cents,
                voucher_amount_cents: None,
                message: "凭证还没填金额".into(),
            });
        }

        if !row.date.is_empty() && !v.voucher_date.is_empty() && row.date != v.voucher_date {
            row_ok = false;
            issues.push(CheckIssue {
                kind: "date_mismatch".into(),
                ledger_ref: row.ledger_ref.clone(),
                voucher_id: Some(v.id),
                voucher_no: v.voucher_no.clone(),
                ledger_date: row.date.clone(),
                voucher_date: v.voucher_date.clone(),
                ledger_amount_cents: row.amount_cents,
                voucher_amount_cents: v.amount_cents,
                message: format!("日期不一致：台账 {} / 凭证 {}", row.date, v.voucher_date),
            });
        }

        if !row.counterparty.trim().is_empty()
            && !v.counterparty.trim().is_empty()
            && row.counterparty.trim() != v.counterparty.trim()
        {
            row_ok = false;
            issues.push(CheckIssue {
                kind: "counterparty_mismatch".into(),
                ledger_ref: row.ledger_ref.clone(),
                voucher_id: Some(v.id),
                voucher_no: v.voucher_no.clone(),
                ledger_date: row.date.clone(),
                voucher_date: v.voucher_date.clone(),
                ledger_amount_cents: row.amount_cents,
                voucher_amount_cents: v.amount_cents,
                message: format!(
                    "对方不一致：台账 {} / 凭证 {}",
                    row.counterparty.trim(),
                    v.counterparty.trim()
                ),
            });
        }

        if row_ok {
            matched += 1;
        }
    }

    for (key, v) in &vouchers {
        if consumed.contains_key(key) {
            continue;
        }
        issues.push(CheckIssue {
            kind: "orphan_voucher".into(),
            ledger_ref: key.clone(),
            voucher_id: Some(v.id),
            voucher_no: v.voucher_no.clone(),
            ledger_date: String::new(),
            voucher_date: v.voucher_date.clone(),
            ledger_amount_cents: None,
            voucher_amount_cents: v.amount_cents,
            message: "凭证有、这张台账表里没有（漏导出 / 导出了别的月份 / 重复导入）".into(),
        });
    }

    let count = |k: &str| issues.iter().filter(|i| i.kind == k).count();
    Ok(CheckReport {
        csv_path: csv_path.to_string_lossy().into_owned(),
        checked_at: crate::utils::paths::now_iso(),
        rows: rows.len(),
        matched,
        missing: count("missing_voucher"),
        orphans: count("orphan_voucher"),
        mismatched: count("amount_mismatch")
            + count("date_mismatch")
            + count("counterparty_mismatch"),
        malformed: count("missing_ref") + count("duplicate_ref"),
        tolerance_cents,
        issues,
    })
}

/// 把差异表导成 CSV（UTF-8 BOM，Excel 直接打开不乱码）
pub fn export_issues(ctx: &CoreContext, report: &CheckReport) -> Result<String> {
    use std::io::Write;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let dir = ctx.paths.exports.join("checks");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("台账对账_{stamp}.csv"));
    let mut f = std::fs::File::create(&path)?;
    f.write_all(b"\xEF\xBB\xBF")?;
    f.write_all("问题类型,台账编号,凭证号,台账日期,凭证日期,台账金额,凭证金额,说明\n".as_bytes())?;
    for i in &report.issues {
        writeln!(
            f,
            "{},{},{},{},{},{},{},{}",
            i.kind,
            i.ledger_ref,
            i.voucher_no,
            i.ledger_date,
            i.voucher_date,
            i.ledger_amount_cents.map(yuan).unwrap_or_default(),
            i.voucher_amount_cents.map(yuan).unwrap_or_default(),
            i.message.replace(',', "，"),
        )?;
    }
    Ok(path.to_string_lossy().into_owned())
}

fn yuan(cents: i64) -> String {
    format!("{}.{:02}", cents / 100, (cents % 100).abs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NewVoucher, VoucherStatus};
    use crate::utils::paths::today;

    fn ctx() -> CoreContext {
        let dir = std::env::temp_dir().join(format!(
            "voucher-check-{}",
            chrono::Local::now().format("%Y%m%d%H%M%S%f")
        ));
        let ctx = CoreContext::init(Some(dir)).expect("init");
        ctx.db.with_conn(crate::db::schema::init).expect("schema");
        ctx
    }

    fn voucher(ctx: &CoreContext, no: &str, ref_: &str, date: &str, cents: i64, party: &str) {
        ctx.db
            .with_conn(|c| {
                crate::db::repos::voucher_repo::insert(
                    c,
                    &NewVoucher {
                        voucher_no: no.into(),
                        voucher_date: date.into(),
                        description: "测试".into(),
                        counterparty: party.into(),
                        amount_cents: Some(cents),
                        status: Some(VoucherStatus::Draft),
                        tags: vec![],
                        note: String::new(),
                        ledger_ref: Some(ref_.into()),
                    },
                )
            })
            .expect("insert");
    }

    fn write_csv(ctx: &CoreContext, body: &str) -> std::path::PathBuf {
        let p = ctx.paths.temp.join(format!(
            "check-{}.csv",
            chrono::Local::now().format("%Y%m%d%H%M%S%f")
        ));
        std::fs::write(&p, body).expect("write");
        p
    }

    #[test]
    fn matched_rows_pass() {
        let ctx = ctx();
        voucher(
            &ctx,
            "2026-11-01-01",
            "cash:9001",
            "2026-11-03",
            1_250_000,
            "凯诗蓝德",
        );
        let csv = write_csv(
            &ctx,
            "流水序号,日期,金额,对方\n9001,2026/11/03,12500.00,凯诗蓝德\n",
        );
        let r = check(&ctx, &csv, 0).expect("check");
        assert_eq!(r.rows, 1);
        assert_eq!(r.matched, 1);
        assert!(r.issues.is_empty(), "{:?}", r.issues);
    }

    #[test]
    fn amount_and_date_mismatch_are_reported() {
        let ctx = ctx();
        voucher(&ctx, "2026-11-01-01", "doc:4", "2026-09-02", 5_000, "张帅");
        let csv = write_csv(&ctx, "单据号,日期,金额,对方\n4,2026/09/03,50.01,张帅\n");
        let r = check(&ctx, &csv, 0).expect("check");
        assert_eq!(r.mismatched, 2, "{:?}", r.issues);
        assert_eq!(r.missing, 0);
    }

    #[test]
    fn missing_and_orphan_are_reported() {
        let ctx = ctx();
        voucher(
            &ctx,
            "2026-11-01-01",
            "cash:9002",
            "2026-11-05",
            3_000,
            "张帅",
        );
        let csv = write_csv(&ctx, "流水号,日期,金额,对方\n9001,2026/11/03,1.00,张三\n");
        let r = check(&ctx, &csv, 0).expect("check");
        assert_eq!(r.missing, 1);
        assert_eq!(r.orphans, 1);
        assert_eq!(r.matched, 0);
    }

    #[test]
    fn csv_without_ref_column_is_flagged() {
        let ctx = ctx();
        let csv = write_csv(&ctx, "日期,金额,对方\n2026/11/03,1.00,张三\n");
        let r = check(&ctx, &csv, 0).expect("check");
        assert_eq!(r.malformed, 1);
        assert_eq!(r.issues[0].kind, "missing_ref");
    }

    #[test]
    fn export_writes_bom_csv() {
        let ctx = ctx();
        let csv = write_csv(&ctx, "流水号,日期,金额\n9001,2026/11/03,1.00\n");
        let r = check(&ctx, &csv, 0).expect("check");
        let out = export_issues(&ctx, &r).expect("export");
        let bytes = std::fs::read(out).expect("read");
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        let _ = today();
    }
}
