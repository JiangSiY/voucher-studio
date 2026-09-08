//! v0.12：OCR 字段校验层（纯函数，密集单测）。
//!
//! 在抽取（extract / extract_with_layout）之后对 `ExtractedFields` 做后验：
//! - **日期**：年份窗口（当前年 ±3）——超窗降置信度（票据上的打印年份/有效期干扰）
//! - **金额**：小写金额多处互验——标签行出现 ≥2 个不同金额时，选定值须与多数一致，
//!   否则降级；一致则提置信
//! - **对方账户**：银行账号模式（16–19 位数字）命中 → 置信度上调；
//!   纯符号/单字符 → 降级为噪声
//!
//! 输出统一改写 `fields.confidence`；不轻易清空字段（宁可低置信提示用户复核）。

use crate::services::ocr_fields::{Confidence, ExtractedFields};

/// 应用全部校验（extract_with_layout 末尾调用）。`now_year` 便于测试注入。
pub fn apply(text: &str, fields: &mut ExtractedFields, now_year: i32) {
    validate_date(fields, now_year);
    validate_amount(text, fields);
    validate_counterparty(fields);
}

/// 日期年份窗口：|年份 - 当前年| > 3 → 置信度降一级（High→Medium→Low→丢弃）。
/// 3 年窗口覆盖跨年票据整理（跨年补录 1–2 年 + 容错）。
fn validate_date(fields: &mut ExtractedFields, now_year: i32) {
    let Some(d) = &fields.date else {
        return;
    };
    let Some(year) = d[..4].parse::<i32>().ok() else {
        return;
    };
    if (year - now_year).abs() > 3 {
        let cur = fields.confidence.date.unwrap_or(Confidence::Low);
        let next = downgrade(cur);
        // 原本无置信度（纯文本路径）也标 Low——超窗日期极可疑
        fields.confidence.date = Some(next);
        if next == Confidence::Low && cur == Confidence::Low {
            // 两次降到底仍超窗：宁可丢弃也不填错（日期错档号全错）
            fields.date = None;
        }
    }
}

/// 金额互验：标签行（合计/小计/总计/实付/实收/金额/价税合计）内出现 ≥2 个不同金额时，
/// 若选定值与多数不一致 → 降级（值可能读错行）；一致 → 升 High（多重互证）。
fn validate_amount(text: &str, fields: &mut ExtractedFields) {
    let Some(chosen) = fields.amount_cents else {
        return;
    };
    // 全量收集标签行金额（含重复——出现次数即票数）
    let mut all: Vec<i64> = Vec::new();
    for line in text.lines() {
        if AMOUNT_HINTS.iter().any(|l| line.contains(l)) {
            all.extend(moneys_in(line));
        }
    }
    if all.len() < 2 {
        return; // 无互证材料
    }
    let mut distinct = all.clone();
    distinct.sort();
    distinct.dedup();
    let max_count = distinct
        .iter()
        .map(|v| all.iter().filter(|x| **x == *v).count())
        .max()
        .unwrap_or(0);
    let majority: Vec<i64> = distinct
        .into_iter()
        .filter(|v| all.iter().filter(|x| *x == v).count() == max_count)
        .collect();

    if majority.contains(&chosen) {
        // 与多数一致：多重互证提置信
        fields.confidence.amount = Some(Confidence::High);
    } else {
        // 少数派：可能读错行（如命中了「小计」而真正总额是「合计」）
        fields.confidence.amount = Some(downgrade(
            fields.confidence.amount.unwrap_or(Confidence::Medium),
        ));
    }
}

/// 对方账户：银行账号（16–19 位纯数字）→ High；少于 2 个有效字符或纯符号 → 降级。
fn validate_counterparty(fields: &mut ExtractedFields) {
    let Some(cp) = &fields.counterparty else {
        return;
    };
    let t = cp.trim();
    let valid_chars = t.chars().filter(|c| c.is_alphanumeric()).count();
    if t.len() >= 16 && t.chars().all(|c| c.is_ascii_digit()) && t.len() <= 19 {
        // 银行账号模式：高置信（结构强特征）
        fields.confidence.counterparty = Some(Confidence::High);
    } else if valid_chars < 2 {
        // 纯符号/单字符：噪声
        fields.confidence.counterparty = Some(downgrade(
            fields.confidence.counterparty.unwrap_or(Confidence::Medium),
        ));
    }
}

const AMOUNT_HINTS: [&str; 7] = ["价税合计", "合计", "总计", "实付", "实收", "小计", "金额"];

/// 行内全部金额（分）。复用与小写金额形态一致的解析（¥/￥ 前缀可选、千分位、两位小数）。
fn moneys_in(line: &str) -> Vec<i64> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            let mut digits = String::new();
            while i < chars.len() {
                if chars[i].is_ascii_digit() {
                    digits.push(chars[i]);
                    i += 1;
                } else if chars[i] == ','
                    && !digits.is_empty()
                    && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit())
                {
                    i += 1; // 千分位
                } else {
                    break;
                }
            }
            // 后随两位小数才算金额（裸数字如账号 16 位会被长度排除——金额位数 ≤12）
            let mut cents: i64 = digits.parse().unwrap_or(0);
            if chars.get(i) == Some(&'.') {
                let mut frac = String::new();
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() && frac.len() < 2 {
                    frac.push(chars[j]);
                    j += 1;
                }
                if !frac.is_empty() {
                    cents = cents * 100
                        + frac.parse::<i64>().unwrap_or(0) * if frac.len() == 1 { 10 } else { 1 };
                    i = j;
                }
                out.push(cents);
            } else if digits.len() <= 12 && start > 0 {
                // 无小数但有货币前缀（¥123 形态）才算
                let prev = chars[start - 1];
                if prev == '¥' || prev == '￥' || prev == '元' {
                    out.push(cents * 100);
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

fn downgrade(c: Confidence) -> Confidence {
    match c {
        Confidence::High => Confidence::Medium,
        Confidence::Medium | Confidence::Low => Confidence::Low,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ocr_fields::FieldConfidence;

    fn fields(date: Option<&str>, amount: Option<i64>, cp: Option<&str>) -> ExtractedFields {
        ExtractedFields {
            date: date.map(|s| s.to_string()),
            amount_cents: amount,
            counterparty: cp.map(|s| s.to_string()),
            merchant: None,
            amount_source: None,
            confidence: FieldConfidence::default(),
        }
    }

    #[test]
    fn date_within_window_untouched() {
        let mut f = fields(Some("2026-08-20"), None, None);
        f.confidence.date = Some(Confidence::High);
        validate_date(&mut f, 2026);
        assert_eq!(f.confidence.date, Some(Confidence::High));
        assert_eq!(f.date.as_deref(), Some("2026-08-20"));
    }

    #[test]
    fn date_outside_window_downgraded_then_dropped() {
        // 高置信超窗 → Medium（保留）
        let mut f = fields(Some("1998-05-01"), None, None);
        f.confidence.date = Some(Confidence::High);
        validate_date(&mut f, 2026);
        assert_eq!(f.confidence.date, Some(Confidence::Medium));
        assert_eq!(f.date.as_deref(), Some("1998-05-01"));

        // 已是 Low 再超窗 → 丢弃
        let mut f2 = fields(Some("1990-01-01"), None, None);
        f2.confidence.date = Some(Confidence::Low);
        validate_date(&mut f2, 2026);
        assert_eq!(f2.date, None, "超窗 + 低置信 → 宁可丢弃");
    }

    #[test]
    fn amount_majority_agreement_upgrades() {
        let mut f = fields(None, Some(12345), None);
        f.confidence.amount = Some(Confidence::Medium);
        let text = "价税合计 123.45\n合计：¥123.45\n小计 12.00";
        validate_amount(text, &mut f);
        assert_eq!(f.confidence.amount, Some(Confidence::High));
    }

    #[test]
    fn amount_minority_downgrades() {
        // 选定 12.00（小计），多数 123.45（合计/价税合计）→ 降级
        let mut f = fields(None, Some(1200), None);
        f.confidence.amount = Some(Confidence::High);
        let text = "价税合计 123.45\n合计：¥123.45\n小计 12.00";
        validate_amount(text, &mut f);
        assert_eq!(f.confidence.amount, Some(Confidence::Medium));
        assert_eq!(f.amount_cents, Some(1200), "值保留，仅降置信");
    }

    #[test]
    fn amount_single_source_no_change() {
        let mut f = fields(None, Some(5000), None);
        f.confidence.amount = Some(Confidence::Medium);
        validate_amount("合计 50.00\n（仅一处金额）", &mut f);
        assert_eq!(f.confidence.amount, Some(Confidence::Medium));
    }

    #[test]
    fn counterparty_bank_account_upgrades() {
        let mut f = fields(None, None, Some("6222020200112233"));
        f.confidence.counterparty = Some(Confidence::Medium);
        validate_counterparty(&mut f);
        assert_eq!(f.confidence.counterparty, Some(Confidence::High));
    }

    #[test]
    fn counterparty_noise_downgrades() {
        let mut f = fields(None, None, Some("**"));
        f.confidence.counterparty = Some(Confidence::Medium);
        validate_counterparty(&mut f);
        assert_eq!(f.confidence.counterparty, Some(Confidence::Low));
    }

    #[test]
    fn moneys_parses_thousands_and_yuan_prefix() {
        assert_eq!(moneys_in("合计 1,234.56"), vec![123456]);
        assert_eq!(moneys_in("实付：¥99"), vec![9900]);
        assert_eq!(moneys_in("账号 6222020200112233"), Vec::<i64>::new());
    }
}
