//! v0.13：微信 / 支付宝账单 CSV 对账（只读匹配 + 报告，不落库、不改凭证）。
//!
//! 定位：对账辅助——「账单有凭证无」（漏拍图）与「凭证有账单无」（金额/日期
//! 不符或多录）的差异盘点。与已删除的旧数据 importer 无关：纯分析，零写入。
//!
//! 解析策略：微信/支付宝导出的 CSV 头部是账户信息等无关行，先扫描找到
//! 表头行（含「交易时间」+「金额」），再按列名定位字段；编码 UTF-8 优先、
//! GBK 兜底（支付宝导出常为 GBK）；金额带 ¥ 前缀、千分位。
//!
//! 匹配规则：同日期 + 金额相等（支出账单对正金额凭证，收入账单对负金额；
//! 「不计收支」跳过）。金额匹配按分精确相等。

use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

use crate::db::repos::voucher_repo;
use crate::models::Voucher;
use crate::{CoreContext, CoreError, Result};

/// 一条账单记录（解析产物）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BillRecord {
    /// YYYY-MM-DD
    pub date: String,
    /// 金额（分；正数，方向由 direction 表达）
    pub amount_cents: i64,
    /// 收 / 支 / 不计收支
    pub direction: String,
    pub counterparty: String,
    /// 商品 / 商品说明
    pub subject: String,
    /// 原始行号（报告回溯用）
    pub line_no: usize,
}

/// 一条匹配：账单 ↔ 凭证
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileMatch {
    pub bill: BillRecord,
    pub voucher: Voucher,
}

/// 对账报告
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileReport {
    /// 账单总条数（不计收支除外）
    pub bill_count: usize,
    /// ✅ 账单 ↔ 凭证匹配
    pub matched: Vec<ReconcileMatch>,
    /// 🔵 账单有凭证无（漏拍图 / 未录入）
    pub bill_only: Vec<BillRecord>,
    /// 🔴 范围内凭证未命中任何账单（金额或日期不符 / 现金交易无账单）
    pub voucher_only: Vec<Voucher>,
    /// 对账范围（含筛选到的账单日期区间）
    pub date_from: String,
    pub date_to: String,
    /// 报告 CSV 落盘路径（exports/reconcile-<时间戳>.csv；失败为 None）
    pub report_path: Option<String>,
}

/// 对账主入口：解析账单 CSV → 与库内凭证匹配 → 落报告 CSV。
/// 范围 = 账单里出现的日期区间（库内同区间凭证参与匹配）。
pub fn reconcile(ctx: &CoreContext, csv_path: &Path) -> Result<ReconcileReport> {
    let bills = parse_bill_csv(csv_path)?;
    let bills: Vec<BillRecord> = bills
        .into_iter()
        .filter(|b| b.direction != "不计收支")
        .collect();
    if bills.is_empty() {
        return Err(CoreError::Invalid(
            "账单里没有可对账的收支记录（请确认选择的是微信/支付宝账单 CSV）".into(),
        ));
    }

    let mut date_from = bills[0].date.clone();
    let mut date_to = bills[0].date.clone();
    for b in &bills {
        if b.date < date_from {
            date_from = b.date.clone();
        }
        if b.date > date_to {
            date_to = b.date.clone();
        }
    }

    // 范围内凭证（金额非空）——行映射与列清单复用 voucher_repo 唯一口径
    let vouchers = ctx.db.with_conn(|conn| {
        let sql = format!(
            "SELECT {} FROM vouchers
             WHERE amount_cents IS NOT NULL
               AND voucher_date BETWEEN ?1 AND ?2
             ORDER BY voucher_date, voucher_no",
            voucher_repo::COLS
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                rusqlite::params![date_from, date_to],
                voucher_repo::row_to_voucher,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    })?;

    // 匹配：同日期 + 同金额（带方向的绝对值匹配；每张凭证只配一条账单）
    let mut matched = Vec::new();
    let mut used_vouchers: HashSet<i64> = HashSet::new();
    let mut bill_only = Vec::new();
    for bill in &bills {
        let hit = vouchers.iter().find(|v| {
            !used_vouchers.contains(&v.id)
                && v.voucher_date == bill.date
                && v.amount_cents.map(|c| c.abs()) == Some(bill.amount_cents)
        });
        match hit {
            Some(v) => {
                used_vouchers.insert(v.id);
                matched.push(ReconcileMatch {
                    bill: bill.clone(),
                    voucher: v.clone(),
                });
            }
            None => bill_only.push(bill.clone()),
        }
    }
    let voucher_only: Vec<Voucher> = vouchers
        .into_iter()
        .filter(|v| !used_vouchers.contains(&v.id))
        .collect();

    let mut report = ReconcileReport {
        bill_count: bills.len(),
        matched,
        bill_only,
        voucher_only,
        date_from,
        date_to,
        report_path: None,
    };
    report.report_path = write_report_csv(ctx, &report).ok();
    Ok(report)
}

/// 报告 CSV 落盘（exports/reconcile-<时间戳>.csv；UTF-8 BOM，Excel 可直接打开）
fn write_report_csv(ctx: &CoreContext, r: &ReconcileReport) -> Result<String> {
    use std::io::Write;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let path = ctx.paths.exports.join(format!("reconcile-{stamp}.csv"));
    let mut out = String::new();
    out.push('\u{FEFF}');
    out.push_str("类型,日期,金额（元）,账单对方,商品,凭证号,凭证摘要\n");
    for m in &r.matched {
        push_row(
            &mut out,
            &[
                "✅ 匹配".into(),
                m.bill.date.clone(),
                format_yuan(m.bill.amount_cents),
                m.bill.counterparty.clone(),
                m.bill.subject.clone(),
                m.voucher.voucher_no.clone(),
                m.voucher.description.clone(),
            ],
        );
    }
    for b in &r.bill_only {
        push_row(
            &mut out,
            &[
                "🔵 账单无凭证".into(),
                b.date.clone(),
                format_yuan(b.amount_cents),
                b.counterparty.clone(),
                b.subject.clone(),
                String::new(),
                String::new(),
            ],
        );
    }
    for v in &r.voucher_only {
        push_row(
            &mut out,
            &[
                "🔴 凭证无账单".into(),
                v.voucher_date.clone(),
                v.amount_cents
                    .map(|c| format_yuan(c.abs()))
                    .unwrap_or_default(),
                v.counterparty.clone(),
                v.description.clone(),
                v.voucher_no.clone(),
                v.description.clone(),
            ],
        );
    }
    let mut f = std::fs::File::create(&path)?;
    f.write_all(out.as_bytes())?;
    Ok(path.to_string_lossy().to_string())
}

fn format_yuan(cents: i64) -> String {
    format!("{}.{:02}", cents / 100, cents % 100)
}

fn push_row(out: &mut String, cells: &[String]) {
    let line = cells
        .iter()
        .map(|c| {
            if c.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", c.replace('"', "\"\""))
            } else {
                c.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    out.push_str(&line);
    out.push('\n');
}

// ---------- 账单 CSV 解析 ----------

/// 解析账单 CSV（微信 / 支付宝导出格式自动识别）
pub fn parse_bill_csv(path: &Path) -> Result<Vec<BillRecord>> {
    let bytes = std::fs::read(path)?;
    // 编码：UTF-8 优先（微信新版），失败按 GBK（支付宝常见）
    let text = match std::str::from_utf8(&bytes) {
        Ok(t) => t.trim_start_matches('\u{FEFF}').to_string(),
        Err(_) => {
            let (decoded, _, had_errors) = encoding_rs::GBK.decode(&bytes);
            if had_errors {
                return Err(CoreError::Invalid(
                    "无法识别账单文件编码（支持 UTF-8 与 GBK 的微信/支付宝 CSV）".into(),
                ));
            }
            decoded.trim_start_matches('\u{FEFF}').to_string()
        }
    };

    // 找表头行（含「交易时间」与「金额」）
    let lines: Vec<&str> = text.lines().collect();
    let header_idx = lines
        .iter()
        .position(|l| l.contains("交易时间") && l.contains("金额"))
        .ok_or_else(|| {
            CoreError::Invalid(
                "未找到账单表头（需含「交易时间」和「金额」列的微信/支付宝账单）".into(),
            )
        })?;

    let header = parse_csv_line(lines[header_idx]);
    let col = |names: &[&str]| -> Option<usize> {
        header
            .iter()
            .position(|h| names.iter().any(|n| h.trim().contains(n)))
    };
    let c_date = col(&["交易时间"]).ok_or_else(|| CoreError::Invalid("缺「交易时间」列".into()))?;
    let c_amount = col(&["金额"]).ok_or_else(|| CoreError::Invalid("缺「金额」列".into()))?;
    let c_direction = col(&["收/支", "收支"]);
    let c_party = col(&["交易对方", "对方"]).or(col(&["商户"]));
    let c_subject = col(&["商品", "说明"]).or(col(&["分类"]));

    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate().skip(header_idx + 1) {
        if line.trim().is_empty() {
            continue;
        }
        let cells = parse_csv_line(line);
        let get = |idx: Option<usize>| -> String {
            idx.and_then(|i| cells.get(i).cloned())
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        let date_raw = get(Some(c_date));
        let date = normalize_date(&date_raw);
        let amount = parse_amount(&get(Some(c_amount)));
        if date.is_empty() || amount.is_none() {
            continue; // 汇总行 / 空行跳过
        }
        out.push(BillRecord {
            date,
            amount_cents: amount.unwrap_or(0),
            direction: get(c_direction),
            counterparty: get(c_party),
            subject: get(c_subject),
            line_no: i + 1,
        });
    }
    Ok(out)
}

/// RFC 4180 行解析（带引号转义）
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            cells.push(cur.clone());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    cells.push(cur);
    cells
}

/// 日期归一：2026-08-20 12:30:45 / 2026-08-20 / 2026/8/20 / 2026年8月20日 → 2026-08-20
fn normalize_date(s: &str) -> String {
    let t = s.trim();
    // 取第一个空白分隔段（去掉时间部分），再按多种格式解析
    let first = t.split_whitespace().next().unwrap_or("");
    for fmt in ["%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d", "%Y年%m月%d日"] {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(first, fmt) {
            return d.format("%Y-%m-%d").to_string();
        }
    }
    String::new()
}

/// 金额解析：¥123.45 / 1,234.56 / 123.4 → 分
fn parse_amount(s: &str) -> Option<i64> {
    let t: String = s
        .trim()
        .trim_start_matches(['¥', '￥'])
        .chars()
        .filter(|c| *c != ',')
        .collect();
    let yuan: f64 = t.parse().ok()?;
    Some((yuan * 100.0).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(content: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("bill-test-{}.csv", std::process::id()));
        std::fs::write(&p, content.as_bytes()).unwrap();
        p
    }

    #[test]
    fn parse_wechat_style_csv() {
        let csv = "微信支付账单明细,,,,,,,,,\n,,,,,,,,,,\n\
                   交易时间,交易类型,交易对方,商品,收/支,金额(元),支付方式,当前状态,交易单号,商户单号,备注\n\
                   2026-08-20 12:30:45,商户消费,文一办公用品店,办公耗材,支出,¥123.45,零钱,支付成功,1,2,\n\
                   2026-08-21 09:00:00,转账,张三,报销款,收入,¥50.00,银行卡,已收转账,3,4,\n\
                   2026-08-22 10:00:00,理财,零钱通,转入,不计收支,1000.00,零钱,支付成功,5,6,\n";
        let p = write_tmp(csv);
        let bills = parse_bill_csv(&p).unwrap();
        assert_eq!(bills.len(), 3);
        assert_eq!(bills[0].date, "2026-08-20");
        assert_eq!(bills[0].amount_cents, 12345);
        assert_eq!(bills[0].direction, "支出");
        assert_eq!(bills[0].counterparty, "文一办公用品店");
        assert_eq!(bills[1].direction, "收入");
        assert_eq!(bills[2].direction, "不计收支");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parse_csv_line_handles_quotes() {
        let cells = parse_csv_line("a,\"b,1\",c,\"d\"\"e\"");
        assert_eq!(cells, vec!["a", "b,1", "c", "d\"e"]);
    }

    #[test]
    fn amount_and_date_normals() {
        assert_eq!(parse_amount("¥1,234.56"), Some(123456));
        assert_eq!(parse_amount("50"), Some(5000));
        assert_eq!(parse_amount("0.5"), Some(50));
        assert_eq!(parse_amount("abc"), None);
        assert_eq!(normalize_date("2026-08-20 12:30:45"), "2026-08-20");
        assert_eq!(normalize_date("2026年8月20日 12:00"), "2026-08-20");
        assert_eq!(normalize_date("bad"), "");
    }
}
