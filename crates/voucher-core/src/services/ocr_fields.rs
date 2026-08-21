//! OCR 字段抽取与自动填单（一图一凭证需求的核心增值能力）
//!
//! 识别文本 → 保守抽取 日期 / 金额 / 对方账户 → 回填凭证空字段 → 附件语义化重命名。
//! 抽取原则：宁可留空也不填错——多义场景（多个无标签金额）直接放弃。

use std::path::Path;

use rusqlite::params;

use crate::db::repos::{attachment_repo, snapshot_repo, voucher_repo};
use crate::models::{Attachment, VoucherUpdate};
use crate::services::{naming, voucher_mgr};
use crate::{CoreContext, Result};

/// 填单模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillMode {
    /// 刚导入新凭证：OCR 先于用户编辑，抽到的字段直接填入（日期/金额/对方账户/摘要）
    Import,
    /// 事后识别/重识别：只填仍为空的字段，绝不覆盖用户已录入内容（日期不动）
    Manual,
}

/// 从 OCR 文本中抽取的结构化字段
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedFields {
    /// YYYY-MM-DD
    pub date: Option<String>,
    pub amount_cents: Option<i64>,
    pub counterparty: Option<String>,
}

/// 金额标签优先级：靠前的优先命中。注意「价税合计」必须排在「合计」之前——
/// `find("合计")` 会命中「价税合计」内部，若顺序颠倒则更具体的标签永远轮不到。
const AMOUNT_LABELS: [&str; 7] = ["价税合计", "合计", "总计", "实付", "实收", "小计", "金额"];
const COUNTERPARTY_LABELS: [&str; 4] = ["收款方", "对方账户", "商户", "收款人"];

/// 从识别文本抽取字段（保守策略）
pub fn extract(text: &str) -> ExtractedFields {
    ExtractedFields {
        date: extract_date(text),
        amount_cents: extract_amount(text),
        counterparty: extract_counterparty(text),
    }
}

/// 日期：支持 2026-08-20 / 2026.08.20 / 2026/8/20 / 2026年8月20日，取第一个合法日期
fn extract_date(text: &str) -> Option<String> {
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i + 4 <= chars.len() {
            let y: String = chars[i..i + 4].iter().collect();
            if y.chars().all(|c| c.is_ascii_digit()) {
                let sep = chars.get(i + 4).copied();
                if matches!(sep, Some('-') | Some('.') | Some('/') | Some('年')) {
                    if let Some(d) = parse_date_after(&chars, i, &y) {
                        return Some(d);
                    }
                }
            }
            i += 1;
        }
    }
    None
}

/// 年份起点已知，解析其后的月/日（分隔符混用也容忍，如 2026-8.20）
fn parse_date_after(chars: &[char], i: usize, year: &str) -> Option<String> {
    let mut pos = i + 5; // 跳过 4 位年份 + 1 个分隔符
    let (month, next) = read_number(chars, pos)?;
    if !(1..=12).contains(&month) {
        return None;
    }
    pos = next;
    // 月后的分隔符
    if !matches!(
        chars.get(pos).copied(),
        Some('-') | Some('.') | Some('/') | Some('月')
    ) {
        return None;
    }
    pos += 1;
    let (day, _) = read_number(chars, pos)?;
    if !(1..=31).contains(&day) {
        return None;
    }
    let y: i32 = year.parse().ok()?;
    // 用 chrono 校验真实日历（2 月 30 日等不合法）
    chrono::NaiveDate::from_ymd_opt(y, month as u32, day as u32)
        .map(|d| d.format("%Y-%m-%d").to_string())
}

/// 从 pos 起读一段连续数字（容忍千分位逗号），返回 (数值, 结束位置)
fn read_number(chars: &[char], pos: usize) -> Option<(i64, usize)> {
    let mut i = pos;
    let mut digits = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            digits.push(c);
            i += 1;
        } else if c == ','
            && !digits.is_empty()
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit())
        {
            i += 1; // 千分位
        } else {
            break;
        }
    }
    if digits.is_empty() {
        return None;
    }
    Some((digits.parse().ok()?, i))
}

/// 金额：优先按标签优先级取「标签行内跟随后」的数字；无标签时全文仅一个金额才采用
fn extract_amount(text: &str) -> Option<i64> {
    for label in AMOUNT_LABELS {
        for line in text.lines() {
            if let Some(idx) = line.find(label) {
                let rest = &line[idx + label.len()..];
                if let Some(cents) = first_money_in(rest) {
                    return Some(cents);
                }
            }
        }
    }
    // 无标签兜底：全文金额恰好唯一才采用（多个则放弃，宁可空着不猜）
    let mut all = Vec::new();
    for line in text.lines() {
        all.extend(moneys_in(line));
    }
    if all.len() == 1 {
        return Some(all[0]);
    }
    None
}

/// 行内所有金额（元，含小数与千分位；「元」后缀或小数部分均可），单位：分。
/// 纯数字（编号/日期段）不算金额；日期样式的 `2026.08.20` 会被模式排除。
fn moneys_in(line: &str) -> Vec<i64> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // 整数部分（含千分位分隔：半角/全角逗号、逗号+空格、空格）
        let (int_digits, j) = scan_digits(&chars, i);
        // 小数部分（最多两位才像金额）
        let mut frac_digits = String::new();
        let mut k = j;
        let mut has_decimal = false;
        if chars.get(k) == Some(&'.') && chars.get(k + 1).is_some_and(|c| c.is_ascii_digit()) {
            has_decimal = true;
            k += 1;
            while k < chars.len() && chars[k].is_ascii_digit() {
                frac_digits.push(chars[k]);
                k += 1;
            }
        }
        let has_yuan = matches!(chars.get(k).copied(), Some('元'));
        // 日期/版本样式排除：数字后紧跟 .-/ 再跟数字（如 2026.08.20、1.2.3）
        let date_like = matches!(chars.get(k).copied(), Some('.') | Some('-') | Some('/'))
            && chars.get(k + 1).is_some_and(|c| c.is_ascii_digit());

        if (has_yuan || has_decimal) && !date_like {
            if let Some(cents) = to_cents(&int_digits, &frac_digits) {
                out.push(cents);
            }
            i = k;
        } else {
            i = j.max(i + 1);
        }
    }
    out
}

/// 扫描 pos 起的整数串，识别千分位分隔：半角逗号、全角逗号、逗号+空格、单空格。
/// 仅当模式为「首段 1-3 位 + 分隔符 + 恰好 3 位」的重复时合并（标准千分位分组），
/// 防止把无关的相邻数字误并成一个数。返回 (纯数字串, 结束位置)。
fn scan_digits(chars: &[char], pos: usize) -> (String, usize) {
    let mut i = pos;
    let mut first = String::new();
    while i < chars.len() && chars[i].is_ascii_digit() {
        first.push(chars[i]);
        i += 1;
    }
    // 千分位仅当首段 1-3 位才可能成立
    if !(1..=3).contains(&first.len()) {
        return (first, i);
    }
    let mut digits = first;
    loop {
        let sep_len = match chars.get(i) {
            Some(',') | Some('，') => {
                if chars.get(i + 1) == Some(&' ') {
                    2
                } else {
                    1
                }
            }
            Some(' ') => 1,
            _ => break,
        };
        // 分隔符后必须恰好 3 位数字
        let mut j = i + sep_len;
        let mut group = String::new();
        while j < chars.len() && chars[j].is_ascii_digit() && group.len() < 3 {
            group.push(chars[j]);
            j += 1;
        }
        if group.len() == 3 {
            digits.push_str(&group);
            i = j;
        } else {
            break;
        }
    }
    (digits, i)
}

/// 整数位 + 小数位 → 分；小数超过两位（不像金额）返回 None
fn to_cents(int_digits: &str, frac_digits: &str) -> Option<i64> {
    if int_digits.len() > 12 {
        return None; // 异常大数，不像金额
    }
    let int_part: i64 = int_digits.parse().ok()?;
    let cents = match frac_digits.len() {
        0 => int_part * 100,
        1 => int_part * 100 + frac_digits.parse::<i64>().ok()? * 10,
        2 => int_part * 100 + frac_digits.parse::<i64>().ok()?,
        _ => return None,
    };
    Some(cents)
}

/// 行内第一个金额（分）
fn first_money_in(s: &str) -> Option<i64> {
    moneys_in(s).into_iter().next()
}

/// 对方账户：标签后同行内容（去除标点/空白，截断到 30 字符）
fn extract_counterparty(text: &str) -> Option<String> {
    for label in COUNTERPARTY_LABELS {
        for line in text.lines() {
            if let Some(idx) = line.find(label) {
                let rest = &line[idx + label.len()..];
                let cleaned = rest
                    .trim()
                    .trim_start_matches(['：', ':', ' ', '　', '-'])
                    .trim();
                // 去掉金额尾巴（如「美团平台 ¥35.00」→「美团平台」）
                let name: String = cleaned
                    .chars()
                    .take_while(|c| !c.is_ascii_digit() && *c != '¥' && *c != '￥')
                    .collect();
                let name = name.trim();
                if !name.is_empty() {
                    return Some(name.chars().take(30).collect());
                }
            }
        }
    }
    None
}

// ---------- 填单 + 重命名 ----------

/// 识别完成后应用：抽取字段回填凭证 + 附件语义化重命名。
/// 任何一步失败都只告警，不影响已完成的 OCR 结果。
pub fn apply_for_attachment(
    ctx: &CoreContext,
    voucher_id: i64,
    attachment_id: i64,
    mode: FillMode,
) -> Result<()> {
    let attachment = ctx.db.with_conn(|conn| {
        attachment_repo::get(conn, attachment_id)?
            .ok_or_else(|| crate::CoreError::NotFound(format!("附件 {attachment_id} 不存在")))
    })?;
    let text = attachment.ocr_text.as_deref().unwrap_or("");
    if text.trim().is_empty() {
        return Ok(());
    }

    let fields = extract(text);
    apply_fields(ctx, voucher_id, &fields, mode)?;
    semantic_rename(ctx, voucher_id, &attachment)?;
    Ok(())
}

/// 把抽到的字段回填凭证（事务 + 快照，与其他写操作口径一致）。
/// Import 模式日期回填后按新日期重排档号（档号日期以凭证日期为准）。
fn apply_fields(
    ctx: &CoreContext,
    voucher_id: i64,
    fields: &ExtractedFields,
    mode: FillMode,
) -> Result<()> {
    let cfg = ctx.config();
    let engine = naming::NamingEngine::new(&cfg.naming.template);

    ctx.db.with_tx(|tx| {
        let current = voucher_repo::get(tx, voucher_id)?
            .ok_or_else(|| crate::CoreError::NotFound(format!("凭证 {voucher_id} 不存在")))?;

        let mut update = VoucherUpdate::default();
        let mut date_refilled = false;
        match mode {
            FillMode::Import => {
                // 新导入：OCR 先于用户编辑，抽到的直接填
                if let Some(d) = &fields.date {
                    if *d != current.voucher_date {
                        update.voucher_date = Some(d.clone());
                        date_refilled = true;
                    }
                }
                if fields.amount_cents.is_some() {
                    update.amount_cents = fields.amount_cents;
                }
                if let Some(c) = &fields.counterparty {
                    update.counterparty = Some(c.clone());
                    // 摘要仍是导入时的文件名默认值 → 用对方账户替代（更有意义）
                    if current.description.is_empty() {
                        update.description = Some(c.clone());
                    }
                }
            }
            FillMode::Manual => {
                // 事后识别：只填空字段，日期/摘要/已录金额不动
                if current.amount_cents.is_none() {
                    update.amount_cents = fields.amount_cents;
                }
                if current.counterparty.is_empty() {
                    if let Some(c) = &fields.counterparty {
                        update.counterparty = Some(c.clone());
                        if current.description.is_empty() {
                            update.description = Some(c.clone());
                        }
                    }
                }
            }
        }

        if update == VoucherUpdate::default() {
            return Ok(()); // 没有可填的字段，不产生空快照
        }

        snapshot_repo::insert(
            tx,
            &crate::models::snapshot::NewSnapshot {
                voucher_id,
                payload: serde_json::to_value(&current)?,
                reason: "ocr_fill".into(),
            },
        )?;
        voucher_repo::update(tx, voucher_id, &update)?;

        // 日期被回填 → 档号重排到新日期（已导出/归档的锁定由内部判断；失败则事务回滚）
        if date_refilled {
            let new_date = fields.date.as_deref().unwrap_or_default();
            if let Some(new_no) = voucher_mgr::renumber_for_date(tx, &engine, voucher_id, new_date)?
            {
                log::info!("OCR 回填日期触发档号重排：凭证 {voucher_id} → {new_no}");
            }
        }
        Ok(())
    })
}

/// 附件语义化重命名：{摘要}-{对方}-{金额元}-{hash8}.{ext}
/// （日期已在档号中，文件名不重复；hash8 保留用于唯一性）
/// 物理文件在附件目录内改名（不跨目录移动），DB 路径同事务更新；失败仅告警。
fn semantic_rename(ctx: &CoreContext, voucher_id: i64, attachment: &Attachment) -> Result<()> {
    let voucher = ctx.db.with_conn(|conn| {
        voucher_repo::get(conn, voucher_id)?
            .ok_or_else(|| crate::CoreError::NotFound(format!("凭证 {voucher_id} 不存在")))
    })?;

    let ext = attachment
        .file_path
        .rsplit('.')
        .next()
        .unwrap_or("img")
        .to_ascii_lowercase();
    let hash8: String = attachment.file_hash.chars().take(8).collect();

    let mut segments: Vec<String> = Vec::new();
    segments.push(voucher.description.clone());
    segments.push(voucher.counterparty.clone());
    if let Some(cents) = voucher.amount_cents {
        segments.push(format!("{}元", crate::models::voucher::format_money(cents)));
    }
    segments.push(hash8);

    let name = build_file_name(&segments, &ext);
    if name == attachment.file_name {
        return Ok(()); // 已是目标名（幂等）
    }

    let dir = match Path::new(&attachment.file_path).parent() {
        Some(d) => d.to_path_buf(),
        None => return Ok(()),
    };
    let dir_str = dir.to_string_lossy().replace('\\', "/");
    let new_rel = format!("{dir_str}/{name}");
    if new_rel == attachment.file_path {
        return Ok(());
    }
    let old_abs = ctx.paths.attachment_abs(&attachment.file_path);
    let new_abs = ctx.paths.attachment_abs(&new_rel);
    if !ctx.paths.is_inside_attachments(&new_abs) {
        return Ok(());
    }

    // 物理改名 → DB 更新；DB 失败则改回，保证文件与记录一致
    if let Err(e) = std::fs::rename(&old_abs, &new_abs) {
        log::warn!("附件重命名失败（{}）: {e}", old_abs.display());
        return Ok(()); // 文件被占用等：保留旧名，不影响数据
    }
    let renamed = ctx.db.with_conn(|conn| {
        conn.execute(
            "UPDATE attachments SET file_name = ?1, file_path = ?2 WHERE id = ?3",
            params![name, new_rel, attachment.id],
        )?;
        Ok(())
    });
    if let Err(e) = renamed {
        let _ = std::fs::rename(&new_abs, &old_abs);
        return Err(e);
    }
    Ok(())
}

/// 拼接文件名：逐段清洗非法字符，空段跳过，总长截断 80（不含扩展名）
fn build_file_name(segments: &[String], ext: &str) -> String {
    let parts: Vec<String> = segments
        .iter()
        .map(|s| sanitize_segment(s))
        .filter(|s| !s.is_empty())
        .collect();
    let mut name = parts.join("-");
    if name.chars().count() > 80 {
        name = name.chars().take(80).collect();
    }
    if name.is_empty() {
        name = "voucher".into();
    }
    format!("{name}.{ext}")
}

fn sanitize_segment(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !matches!(
                c,
                '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r' | '\t'
            )
        })
        .collect::<String>()
        .trim()
        .trim_end_matches('.')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_iso_date() {
        assert_eq!(
            extract("开票日期：2026-08-20\n金额：100.00元").date,
            Some("2026-08-20".into())
        );
    }

    #[test]
    fn extracts_chinese_date() {
        assert_eq!(extract("2026年8月5日 午餐").date, Some("2026-08-05".into()));
        assert_eq!(extract("2026.08.20").date, Some("2026-08-20".into()));
        assert_eq!(extract("2026/8/2 快递").date, Some("2026-08-02".into()));
    }

    #[test]
    fn rejects_invalid_calendar_date() {
        assert_eq!(extract("2026-02-30").date, None);
        assert_eq!(extract("2026-13-01").date, None);
    }

    #[test]
    fn extracts_labeled_amount_by_priority() {
        let t = "小计：30.00元\n合计：128.50元";
        assert_eq!(extract(t).amount_cents, Some(12850));
    }

    #[test]
    fn tax_inclusive_total_beats_plain_total() {
        // 发票同时出现「合计（不含税）」与「价税合计」时，应取价税合计
        let t = "合计：100.00元\n价税合计：113.00元";
        assert_eq!(extract(t).amount_cents, Some(11300));
    }

    #[test]
    fn thousands_separator_variants() {
        // OCR 常见千分位变体：此前除半角逗号紧贴外全部被截断（1,234.56 → 234.56）
        assert_eq!(extract("合计：¥1,234.56").amount_cents, Some(123456)); // 半角逗号
        assert_eq!(extract("合计：¥1，234.56").amount_cents, Some(123456)); // 全角逗号
        assert_eq!(extract("合计：¥1, 234.56").amount_cents, Some(123456)); // 逗号+空格
        assert_eq!(extract("合计：¥1 234.56").amount_cents, Some(123456)); // 空格千分位
                                                                           // 多段千分位
        assert_eq!(
            extract("总计：12,345,678.90元").amount_cents,
            Some(1234567890)
        );
    }

    #[test]
    fn non_standard_grouping_not_merged() {
        // 首段 4 位以上 + 分隔符：不是标准千分位，不合并（宁可不识别也不错并）
        assert_eq!(extract("编号 1234,567 未识别金额").amount_cents, None);
    }

    #[test]
    fn extracts_unlabeled_single_amount() {
        assert_eq!(extract("出租车票 45.00元").amount_cents, Some(4500));
    }

    #[test]
    fn gives_up_on_multiple_unlabeled_amounts() {
        let t = "编号1234.56\n另一行 78.90 元";
        assert_eq!(extract(t).amount_cents, None);
    }

    #[test]
    fn pure_numbers_are_not_amounts() {
        // 123456 是编号（无「元」无小数），不该被当金额
        assert_eq!(extract("凭证编号：123456").amount_cents, None);
    }

    #[test]
    fn extracts_counterparty() {
        assert_eq!(
            extract("收款方：美团平台\n金额：35.00元").counterparty,
            Some("美团平台".into())
        );
        assert_eq!(
            extract("商户：肯德基（人民广场店） 45.00").counterparty,
            Some("肯德基（人民广场店）".into())
        );
    }

    #[test]
    fn file_name_building() {
        let name = build_file_name(
            &[
                "午餐报销".into(),
                "美团:平台".into(),
                "35.00元".into(),
                "a1b2c3d4".into(),
            ],
            "jpg",
        );
        assert_eq!(name, "午餐报销-美团平台-35.00元-a1b2c3d4.jpg");

        // 空段跳过
        let name2 = build_file_name(
            &["午餐报销".into(), "".into(), "".into(), "a1b2c3d4".into()],
            "png",
        );
        assert_eq!(name2, "午餐报销-a1b2c3d4.png");
    }

    #[test]
    fn long_name_truncated() {
        let long = "很".repeat(200);
        let name = build_file_name(&[long], "jpg");
        assert!(name.chars().count() <= 84); // 80 + ".jpg"
    }
}
