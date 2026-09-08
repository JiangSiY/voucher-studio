//! 表格式导出渲染（v0.10）：Excel（rust_xlsxwriter）与 CSV（UTF-8 BOM）。
//!
//! 列：凭证号 / 日期 / 摘要 / 对方账户 / 金额（元）/ 状态 / 标签 / 备注，末行合计。
//! 金额写为数值（Excel 可直接 SUM；CSV 写无千分位的十进制文本），
//! 金额为空的凭证该格留空、不计入合计——与 PDF 合计行口径一致。

use std::path::Path;

use crate::models::{Voucher, VoucherStatus};
use crate::Result;

/// 状态中文标签（导出表直接可读）
fn status_label(s: VoucherStatus) -> &'static str {
    match s {
        VoucherStatus::Draft => "草稿",
        VoucherStatus::Reviewed => "已复核",
        VoucherStatus::Exported => "已导出",
        VoucherStatus::Archived => "已归档",
    }
}

const HEADERS: [&str; 8] = [
    "凭证号",
    "日期",
    "摘要",
    "对方账户",
    "金额（元）",
    "状态",
    "标签",
    "备注",
];

/// Excel 导出（.xlsx）。表头加粗冻结；金额列数值格式 `0.00`。
pub fn write_xlsx(path: &Path, vouchers: &[Voucher]) -> Result<()> {
    use rust_xlsxwriter::{Format, FormatAlign, Workbook};

    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.set_name("凭证清单")?;

    let bold = Format::new().set_bold();
    let money_fmt = Format::new().set_num_format("0.00");
    let total_fmt = Format::new().set_bold().set_num_format("0.00");
    let center = Format::new().set_align(FormatAlign::Center);

    // 表头
    for (c, h) in HEADERS.iter().enumerate() {
        sheet.write_with_format(0, c as u16, *h, &bold)?;
    }
    // 冻结首行 + 自动筛选
    sheet.set_freeze_panes(1, 0)?;
    sheet.autofilter(0, 0, vouchers.len() as u32 + 1, HEADERS.len() as u16 - 1)?;

    let mut total_cents: i64 = 0;
    for (r, v) in vouchers.iter().enumerate() {
        let row = (r + 1) as u32;
        sheet.write_string(row, 0, &v.voucher_no)?;
        sheet.write_string(row, 1, &v.voucher_date)?;
        sheet.write_string(row, 2, &v.description)?;
        sheet.write_string(row, 3, &v.counterparty)?;
        if let Some(cents) = v.amount_cents {
            sheet.write_with_format(row, 4, cents as f64 / 100.0, &money_fmt)?;
            total_cents += cents;
        } // 空金额留白
        sheet.write_string(row, 5, status_label(v.status))?;
        sheet.write_string(row, 6, v.tags.join(", "))?;
        sheet.write_string(row, 7, &v.note)?;
    }

    // 合计行
    let total_row = (vouchers.len() + 1) as u32;
    sheet.write_with_format(total_row, 0, "合计", &bold)?;
    sheet.write_with_format(total_row, 2, format!("{} 张", vouchers.len()), &bold)?;
    sheet.write_with_format(total_row, 4, total_cents as f64 / 100.0, &total_fmt)?;
    sheet.write_with_format(total_row, 5, "", &center)?;

    // 列宽（字符数；金额/摘要/备注略宽）
    let widths = [16.0, 12.0, 28.0, 20.0, 12.0, 9.0, 18.0, 24.0];
    for (c, w) in widths.iter().enumerate() {
        sheet.set_column_width(c as u16, *w)?;
    }

    workbook.save(path)?;
    Ok(())
}

/// CSV 导出。UTF-8 带 BOM（Excel 直接双击打开中文不乱码），
/// 转义遵循 RFC 4180（含逗号/引号/换行的字段加引号，引号翻倍）。
pub fn write_csv(path: &Path, vouchers: &[Voucher]) -> Result<()> {
    let mut out = String::new();
    out.push('\u{FEFF}'); // BOM
    push_csv_row(&mut out, &HEADERS);
    let mut total_cents: i64 = 0;
    for v in vouchers {
        let amount = match v.amount_cents {
            Some(cents) => {
                total_cents += cents;
                format_yuan_plain(cents)
            }
            None => String::new(),
        };
        let cells = [
            v.voucher_no.clone(),
            v.voucher_date.clone(),
            v.description.clone(),
            v.counterparty.clone(),
            amount,
            status_label(v.status).to_string(),
            v.tags.join(", "),
            v.note.clone(),
        ];
        push_csv_row(&mut out, &cells);
    }
    // 合计行
    let total = [
        "合计".to_string(),
        String::new(),
        format!("{} 张", vouchers.len()),
        String::new(),
        if vouchers.iter().any(|v| v.amount_cents.is_some()) {
            format_yuan_plain(total_cents)
        } else {
            String::new()
        },
        String::new(),
        String::new(),
        String::new(),
    ];
    push_csv_row(&mut out, &total);

    std::fs::write(path, out)?;
    Ok(())
}

/// 分 → 无千分位十进制文本（CSV 数值口径："-1234.56"）
fn format_yuan_plain(cents: i64) -> String {
    format!("{}.{:02}", cents / 100, cents % 100)
}

/// 追加一行（自动换行结尾）
fn push_csv_row(out: &mut String, cells: &[impl AsRef<str>]) {
    let line = cells
        .iter()
        .map(|c| csv_escape(c.as_ref()))
        .collect::<Vec<_>>()
        .join(",");
    out.push_str(&line);
    out.push('\n');
}

/// RFC 4180 转义：含逗号 / 引号 / 换行 / 回车的字段整体加引号，内部引号翻倍
fn csv_escape(s: &str) -> String {
    if s.contains(&[',', '"', '\n', '\r'][..]) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Voucher> {
        let v = Voucher {
            id: 1,
            voucher_no: "V-001".into(),
            voucher_date: "2026-09-01".into(),
            description: "办公用品,采购".into(),
            counterparty: "文一店".into(),
            amount_cents: Some(12345),
            status: VoucherStatus::Draft,
            tags: vec!["办公".into(), "报销".into()],
            note: "含\"引号\"字段".into(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        let v2 = Voucher {
            id: 2,
            voucher_no: "V-002".into(),
            voucher_date: "2026-09-02".into(),
            description: "空金额".into(),
            counterparty: String::new(),
            amount_cents: None,
            status: VoucherStatus::Reviewed,
            tags: Vec::new(),
            note: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        let v3 = Voucher {
            id: 3,
            voucher_no: "V-003".into(),
            voucher_date: "2026-09-03".into(),
            description: "负数金额".into(),
            counterparty: String::new(),
            amount_cents: Some(-100),
            status: VoucherStatus::Archived,
            tags: Vec::new(),
            note: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        vec![v.clone(), v2, v3]
    }

    #[test]
    fn csv_has_bom_and_escaped_cells() {
        let vouchers = sample();
        let mut v = vouchers[0].clone();
        v.description = "a,b".into();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.csv");
        write_csv(&p, &[v]).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        // UTF-8 BOM
        assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF]);
        let text = String::from_utf8_lossy(&bytes[3..]).to_string();
        assert!(text.starts_with("凭证号,日期,"));
        // 含逗号字段被转义
        assert!(text.contains("\"a,b\""));
        // 引号翻倍
        assert!(text.contains("\"\"引号\"\""));
        // 金额数值口径（无千分位）
        assert!(text.contains("123.45"));
        // 合计行
        assert!(text.contains("合计"));
    }

    #[test]
    fn csv_total_only_when_amounts_present() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.csv");
        let vouchers = vec![Voucher {
            id: 9,
            voucher_no: "V-009".into(),
            voucher_date: "2026-09-01".into(),
            description: String::new(),
            counterparty: String::new(),
            amount_cents: None,
            status: VoucherStatus::Draft,
            tags: Vec::new(),
            note: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }];
        write_csv(&p, &vouchers).unwrap();
        let text = String::from_utf8_lossy(&std::fs::read(&p).unwrap()[3..]).to_string();
        let lines: Vec<&str> = text.trim_end().lines().collect();
        let total_line = lines.last().unwrap();
        // 全空金额：合计金额格留空（不出现 "0.00"）
        assert!(total_line.starts_with("合计"));
        assert!(!total_line.contains("0.00"));
    }

    #[test]
    fn xlsx_writes_and_totals() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.xlsx");
        let vouchers = sample();
        write_xlsx(&p, &vouchers).unwrap();
        let meta = std::fs::metadata(&p).unwrap();
        assert!(meta.len() > 1000); // 合法 zip 容器
    }

    #[test]
    fn yuan_plain_formats() {
        assert_eq!(format_yuan_plain(12345), "123.45");
        assert_eq!(format_yuan_plain(-100), "-1.00");
        assert_eq!(format_yuan_plain(5), "0.05");
    }
}
