//! OCR 字段抽取与自动填单（一图一凭证需求的核心增值能力）
//!
//! 识别文本 → 保守抽取 日期 / 金额 / 对方账户 → 回填凭证空字段 → 附件语义化重命名。
//! 抽取原则：宁可留空也不填错——多义场景（多个无标签金额）直接放弃。
//!
//! v0.20 自动填写门控（宁空勿错落地，`should_autofill`）：
//! - **只自动填 日期 / 金额 / 对方账户 三个字段**，摘要永远人工填写
//! - 金额最严：只填 High（大写互验 / 多数互验升级后才算）；中低置信进识别结果卡等人工点
//! - 日期 / 对方：Medium 及以上填，Low 不填

use std::path::Path;

use rusqlite::params;

use crate::db::repos::{attachment_repo, snapshot_repo, voucher_repo};
use crate::models::{Attachment, VoucherUpdate};
use crate::services::{naming, voucher_mgr};
use crate::{CoreContext, Result};

/// 填单模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillMode {
    /// 刚导入新凭证：OCR 先于用户编辑，按置信度门控填 日期/金额/对方账户
    Import,
    /// 事后识别/重识别：只填仍为空的字段，绝不覆盖用户已录入内容（日期不动）
    Manual,
}

/// 金额提取来源（供前端展示置信度）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AmountSource {
    /// 命中「价税合计/合计」等标签——高置信
    Labeled,
    /// 全文唯一金额兜底——低置信（无标签，仅凭唯一性）
    UniqueOnly,
}

/// 字段置信度（v5 版面分析引入；前端可按级别提示用户复核）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// 几何 KV 配对 / 标签命中 + 版面佐证 / 大小写金额互验一致
    High,
    /// 纯文本标签命中（无几何佐证）/ 唯一日期
    Medium,
    /// 启发式兜底（首行店名 / 全文唯一金额）
    Low,
}

/// 各字段置信度（None = 字段未抽出）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldConfidence {
    pub date: Option<Confidence>,
    pub amount: Option<Confidence>,
    pub counterparty: Option<Confidence>,
    pub merchant: Option<Confidence>,
}

/// 从 OCR 文本中抽取的结构化字段
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedFields {
    /// YYYY-MM-DD
    pub date: Option<String>,
    pub amount_cents: Option<i64>,
    pub counterparty: Option<String>,
    /// 商家/销售方（发票「销售方」、小票顶部商家行）
    pub merchant: Option<String>,
    /// 金额提取来源：标签命中（可信）还是全文唯一兜底（低置信）
    pub amount_source: Option<AmountSource>,
    /// 字段级置信度（纯文本路径按来源标档；版面路径由几何佐证改写，
    /// 两路径都会经 ocr_validate 升降级——自动填单按此门控）
    pub confidence: FieldConfidence,
}

/// 金额标签优先级：靠前的优先命中。注意「价税合计」必须排在「合计」之前——
/// `find("合计")` 会命中「价税合计」内部，若顺序颠倒则更具体的标签永远轮不到。
const AMOUNT_LABELS: [&str; 7] = ["价税合计", "合计", "总计", "实付", "实收", "小计", "金额"];
const COUNTERPARTY_LABELS: [&str; 4] = ["收款方", "对方账户", "商户", "收款人"];
const MERCHANT_LABELS: [&str; 4] = ["销售方", "开票方", "销售单位", "商家名称"];

/// 从识别文本抽取字段（保守策略）。
/// 置信度按来源标档：标签命中 Medium、启发式兜底 Low、金额按 amount_source 映射——
/// 版面路径（extract_with_layout）会在几何佐证后整体改写 confidence。
pub fn extract(text: &str) -> ExtractedFields {
    let (amount_cents, amount_source) = extract_amount(text);
    let (merchant, merchant_labeled) = extract_merchant_with_source(text)
        .map(|(m, labeled)| (Some(m), labeled))
        .unwrap_or((None, false));
    let cp_by_label = extract_counterparty(text);
    // 微信/支付宝截图专属模式（标签缺失时的第二顺位；实测截图无「收款方」标签）
    let cp_transfer = extract_transfer_target(text);
    // 对方账户回退：收款场景标签（收款方/商户…）缺失时，商家即实际收款对象——
    // 发票「销售方」与小票顶部店名都满足此语义（购买方是记账人自己，不算对方）
    let counterparty = cp_by_label
        .clone()
        .or(cp_transfer.clone())
        .or_else(|| merchant.clone());
    let date = extract_date(text);

    let mut confidence = FieldConfidence::default();
    if merchant.is_some() {
        confidence.merchant = Some(if merchant_labeled {
            Confidence::Medium
        } else {
            Confidence::Low
        });
    }
    if counterparty.is_some() {
        // 标签命中 Medium；「转给」模式 Medium；商家回退沿用商家档（标签 Medium / 首行启发式 Low）
        confidence.counterparty = Some(if cp_by_label.is_some() || cp_transfer.is_some() {
            Confidence::Medium
        } else {
            confidence.merchant.unwrap_or(Confidence::Low)
        });
    }
    if date.is_some() {
        confidence.date = Some(Confidence::Medium); // 合法日历日期唯一取值
    }
    confidence.amount = amount_source.map(|s| match s {
        AmountSource::Labeled => Confidence::Medium,
        AmountSource::UniqueOnly => Confidence::Low,
    });

    ExtractedFields {
        date,
        amount_cents,
        counterparty,
        merchant,
        amount_source,
        confidence,
    }
}

/// v0.12：抽取 + 后验校验（纯文本路径的出口；版面路径在 extract_with_layout 内调用）。
/// 校验层只改写 confidence / 丢弃超窗日期，抽取逻辑本身不动。
pub fn extract_validated(text: &str) -> ExtractedFields {
    use chrono::Datelike;
    let mut f = extract(text);
    let now_year = chrono::Local::now().date_naive().year();
    crate::services::ocr_validate::apply(text, &mut f, now_year);
    f
}

/// 日期：支持 2026-08-20 / 2026.08.20 / 2026/8/20 / 2026年8月20日，取第一个合法日期
/// （v5：pub(crate) 供 ocr_layout 按行解析候选日期）
pub(crate) fn extract_date(text: &str) -> Option<String> {
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

/// 金额：优先按标签优先级取「标签行内跟随后」的数字；无标签时全文仅一个金额才采用。
/// 返回 (金额分, 来源)；来源供前端区分「标签命中（可信）」与「唯一兜底（低置信）」。
fn extract_amount(text: &str) -> (Option<i64>, Option<AmountSource>) {
    for label in AMOUNT_LABELS {
        for line in text.lines() {
            if let Some(idx) = line.find(label) {
                let rest = &line[idx + label.len()..];
                if let Some(cents) = first_money_in(rest) {
                    return (Some(cents), Some(AmountSource::Labeled));
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
        return (Some(all[0]), Some(AmountSource::UniqueOnly));
    }
    (None, None)
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

/// 大写金额识别与转换（壹贰叁…元 / 角 / 分），逐行扫描取第一个可解析的。
/// 从行内第一个大写数字起解析（容忍「价税合计（大写）：」等标签前缀）；
/// OCR 错认大写（缺字/错字）时返回 None。pub(crate)：ocr_layout（版面互验）与
/// ocr_validate（纯文本互验）共用同一解析，保证两条路径口径一致。
pub(crate) fn parse_uppercase_amount(text: &str) -> Option<i64> {
    const DIGITS: [(&str, u32); 10] = [
        ("零", 0),
        ("壹", 1),
        ("贰", 2),
        ("叁", 3),
        ("肆", 4),
        ("伍", 5),
        ("陆", 6),
        ("柒", 7),
        ("捌", 8),
        ("玖", 9),
    ];
    const UNITS: [(&str, u64); 4] = [("拾", 10), ("佰", 100), ("仟", 1000), ("万", 10_000)];

    for line in text.lines() {
        let t = line.trim();
        if !t.contains('元') {
            continue; // 候选行必须含「元」
        }
        let chars: Vec<char> = t.chars().collect();
        // 起点：第一个大写数字（跳过行首的标签/括号等杂字）
        let Some(start) = chars
            .iter()
            .position(|c| DIGITS.iter().any(|(d, _)| d.starts_with(*c)))
        else {
            continue;
        };
        // 终点：起点后的第一个「元」（含）；「元」后可能跟「整/正」或角分
        let Some(rel_end) = chars[start..].iter().position(|c| *c == '元') else {
            continue;
        };
        let end = start + rel_end + 1;
        let seg = &chars[start..end];
        let mut total: u64 = 0;
        let mut current: u64 = 0;
        let mut ok = true;
        for c in seg {
            if let Some((_, v)) = DIGITS.iter().find(|(d, _)| d.starts_with(*c)) {
                current = *v as u64;
            } else if let Some((_, u)) = UNITS.iter().find(|(d, _)| d.starts_with(*c)) {
                // 万位：清零累计
                if *u == 10_000 {
                    total = (total + current) * 10_000;
                } else {
                    total += current.max(1) * u;
                }
                current = 0;
            } else if *c == '元' {
                total += current;
                break;
            } else {
                ok = false;
                break;
            }
        }
        if !ok || total == 0 {
            continue;
        }
        let mut cents = (total as i64) * 100;
        // 角分：「元」后的「X角Y分」
        let rest: String = chars[end..].iter().collect();
        for (label, mult) in [("角", 10i64), ("分", 1)] {
            if let Some(pos) = rest.find(label) {
                let before: String = rest.chars().take(pos).collect();
                if let Some((_, v)) = DIGITS.iter().find(|(d, _)| before.contains(d)) {
                    cents += *v as i64 * mult;
                }
            }
        }
        return Some(cents);
    }
    None
}

/// 主标签命中后，处理紧跟其后的「子标签」（v0.20 评测发现：微信/银行截图里
/// 「收款方备注」「收款方全称」这类复合标签会把主标签截胡，值被抽成「备注」「全称」等界面词）。
///
/// - 子标签是**名称类**（全称/简称/户名/名称）→ 剥掉，余下才是值（「收款人全称：张三」→ 张三）
/// - 子标签是**异字段**（备注/账号/卡号/单号/记账日…）→ 主标签只是复合词的前半 → 放弃本次命中
/// - 其余情况原样返回（含空串——版面路径的「跨行取值」形态还要用空判定）
pub(crate) fn strip_sub_label(rest: &str) -> Option<&str> {
    const NAME_SUBS: [&str; 4] = ["全称", "简称", "户名", "名称"];
    const OTHER_FIELDS: [&str; 11] = [
        "备注",
        "账号",
        "卡号",
        "单号",
        "單号",
        "序号",
        "记账日",
        "交易日",
        "明细",
        "开户行",
        "开户银行",
    ];
    let trimmed = rest.trim_start_matches(['：', ':', ' ', '　', '-']);
    // 「子标签后紧跟分隔符或行尾」才算复合词（「备注夹」这种就按普通文本处理）
    let is_compound = |word: &str| {
        trimmed.starts_with(word)
            && trimmed[word.len()..]
                .chars()
                .next()
                .is_none_or(|c| matches!(c, '：' | ':' | ' ' | '　'))
    };
    if OTHER_FIELDS.iter().any(|w| is_compound(w)) {
        return None; // 复合词是别的字段（收款方备注/账户单号…），本次命中作废
    }
    if let Some(w) = NAME_SUBS.iter().find(|w| is_compound(w)) {
        return Some(trimmed[w.len()..].trim_start_matches(['：', ':', ' ', '　']));
    }
    Some(trimmed)
}

/// 界面文字黑名单：截图/短信里的 UI 标签词被当「值」抽出时一律丢弃
/// （2026-10-09 评测两轮：微信/银行截图实填错误全为这类词；第二轮补 服务/开户行；
/// 换 PP-OCRv5 后补**繁体变体**——v5 词表含繁体字，会把「備註」等认出来）
const VALUE_NOISE: [&str; 30] = [
    "户名",
    "账号",
    "卡号",
    "备注",
    "全称",
    "简称",
    "单号",
    "單号",
    "序号",
    "记账日",
    "交易日",
    "明细",
    "交易详情",
    "账单详情",
    "支付成功",
    "交易成功",
    "当前状态",
    "转账说明",
    "收款方备注",
    "服务",
    "开户行",
    "开户银行",
    "備註",
    "備注",
    "戶名",
    "帳號",
    "卡號",
    "全稱",
    "簡稱",
    "開戶行",
];

/// 值是否是界面噪音词或残片：
/// - 等值命中黑名单（服务/开户行…）；
/// - 以「的」开头的残片（「商户的交易」被截出的「的交易」）；
/// - 以字段标签词开头（「收款方开户行」整串被当名字——第三轮评测同型 5 例）
pub(crate) fn is_value_noise(s: &str) -> bool {
    const LABEL_PREFIX: [&str; 12] = [
        "收款方",
        "收款人",
        "对方账户",
        "商户",
        "销售方",
        "购买方",
        "户名",
        "账号",
        "卡号",
        "开户行",
        "备注",
        "全称",
    ];
    let t = s.trim();
    VALUE_NOISE.contains(&t) || t.starts_with('的') || LABEL_PREFIX.iter().any(|w| t.starts_with(w))
}

/// 微信/支付宝截图的对方提法：「转账-转给XXX」「扫二维码付款-给XXX」——
/// 这比标签路径更贴近用户账面（实测截图里没有「收款方」标签，只有这两个模式）。
/// 返回名字 + 是否命中。
fn extract_transfer_target(text: &str) -> Option<String> {
    const MARKERS: [&str; 2] = ["转给", "付款-给"];
    for line in text.lines() {
        for marker in MARKERS {
            if let Some(idx) = line.find(marker) {
                let rest = &line[idx + marker.len()..];
                let name: String = rest
                    .chars()
                    .take_while(|c| !matches!(c, '／' | '/' | ' ' | '\t' | '，' | ',' | '。' | '|'))
                    .collect();
                let name = name.trim();
                // 至少 1 个名称性字符（微信昵称可单字，如「冰」；纯数字/标点残片拒绝）
                if !name.is_empty() && name.chars().any(|c| c.is_alphabetic()) {
                    return Some(name.chars().take(30).collect());
                }
            }
        }
    }
    None
}

/// 对方账户：标签后同行内容（去除标点/空白，截断到 30 字符）；
/// 标签不中 → 微信/支付宝「转给」模式；再回退商家。
fn extract_counterparty(text: &str) -> Option<String> {
    for label in COUNTERPARTY_LABELS {
        for line in text.lines() {
            if let Some(idx) = line.find(label) {
                let rest = &line[idx + label.len()..];
                let Some(start) = strip_sub_label(rest) else {
                    continue; // 复合标签是别的字段（收款方备注…）→ 换下一处
                };
                let cleaned = start
                    .trim()
                    .trim_start_matches(['：', ':', ' ', '　', '-'])
                    .trim();
                // 去掉金额尾巴（如「美团平台 ¥35.00」→「美团平台」）
                let name: String = cleaned
                    .chars()
                    .take_while(|c| !c.is_ascii_digit() && *c != '¥' && *c != '￥')
                    .collect();
                let name = name.trim();
                if !name.is_empty() && !is_value_noise(name) {
                    return Some(name.chars().take(30).collect());
                }
            }
        }
    }
    None
}

/// 商家/销售方：标签行内容优先；无标签时取小票顶部首个「纯中文行」（首行大概率是店名）。
/// 与对方账户独立：发票场景两者常同时存在（购买方 vs 销售方）。
fn extract_merchant_with_source(text: &str) -> Option<(String, bool)> {
    for label in MERCHANT_LABELS {
        for line in text.lines() {
            if let Some(idx) = line.find(label) {
                let rest = &line[idx + label.len()..];
                let Some(start) = strip_sub_label(rest) else {
                    continue; // 复合标签（销售方备注…）→ 换下一处
                };
                let cleaned = start
                    .trim()
                    .trim_start_matches(['：', ':', ' ', '　', '-'])
                    .trim();
                // 去掉「名称：xxx」二次标签与数字尾巴
                let cleaned = cleaned.trim_start_matches(['：', ':', ' ', '　']);
                let name: String = cleaned
                    .chars()
                    .take_while(|c| !c.is_ascii_digit() && *c != '¥' && *c != '￥')
                    .collect();
                let name = name.trim();
                if name.chars().count() >= 2 && !is_value_noise(name) {
                    return Some((name.chars().take(30).collect(), true));
                }
            }
        }
    }
    // 兜底：首个「以中文开头且长度≥4」的行视为店名（小票顶部布局）。
    // 行首是字段标签或界面噪音词的行不算店名（「收款方备注 …」曾被当成店名、
    // 再经商家回退污染对方账户——2026-09 基准评测同型）
    for line in text.lines() {
        let t = line.trim();
        let starts_with_label = COUNTERPARTY_LABELS
            .iter()
            .chain(MERCHANT_LABELS.iter())
            .any(|w| t.starts_with(w));
        if t.chars().count() >= 4
            && t.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() && !c.is_ascii())
            && !t.contains('：')
            && !t.contains(':')
            && !starts_with_label
            && !VALUE_NOISE.iter().any(|w| t.starts_with(w))
        {
            // 去掉金额尾巴（「出租车票 45.00元」→「出租车票」）——
            // 商家会回退为对方账户，带金额会污染凭证字段
            let name: String = t
                .chars()
                .take_while(|c| !c.is_ascii_digit() && *c != '¥' && *c != '￥')
                .collect();
            let name = name.trim();
            if name.chars().count() >= 2 && !is_value_noise(name) {
                return Some((name.chars().take(30).collect(), false));
            }
        }
    }
    None
}

// ---------- 填单 + 重命名 ----------

/// 识别完成后应用：抽取字段回填凭证 + 附件语义化重命名。
/// 任何一步失败都只告警，不影响已完成的 OCR 结果。
/// v5：优先用版面数据（坐标框）做几何配对；无版面回退纯文本抽取。
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

    // v5：有版面数据走几何配对（准确率更高），否则回退纯文本抽取（旧路径不变）
    let layout = ctx
        .db
        .with_conn(|conn| crate::db::repos::ocr_layout_repo::get_layout(conn, attachment_id))?;
    let fields = match layout {
        Some(lines) if !lines.is_empty() => {
            crate::services::ocr_layout::extract_with_layout(&lines)
        }
        _ => extract_validated(text),
    };
    apply_fields(ctx, voucher_id, &fields, mode)?;
    semantic_rename(ctx, voucher_id, &attachment)?;
    Ok(())
}

/// 自动填写门控（v0.20「宁空勿错」落地）。
///
/// - `strict`（金额）：只填 High——大写互验 / 标签行多数互验升级后才自动填，
///   中低置信一律不填（识别结果卡里人工点「填入」仍可补）；
/// - 非 strict（日期 / 对方）：Medium 及以上填，Low 不填；
/// - `None`（纯文本路径未抽出以外的缺失态）不填。
pub(crate) fn should_autofill(conf: Option<Confidence>, strict: bool) -> bool {
    match conf {
        Some(Confidence::High) => true,
        Some(Confidence::Medium) => !strict,
        Some(Confidence::Low) | None => false,
    }
}

/// 把抽到的字段回填凭证（事务 + 快照，与其他写操作口径一致）。
/// 只填 日期/金额/对方账户 三字段（按 `should_autofill` 门控），摘要永远人工填写。
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
                // 新导入：OCR 先于用户编辑，但只填过门控的三字段
                if let Some(d) = &fields.date {
                    if *d != current.voucher_date && should_autofill(fields.confidence.date, false)
                    {
                        update.voucher_date = Some(d.clone());
                        date_refilled = true;
                    }
                }
                if fields.amount_cents.is_some() && should_autofill(fields.confidence.amount, true)
                {
                    update.amount_cents = fields.amount_cents;
                }
                if let Some(c) = &fields.counterparty {
                    if should_autofill(fields.confidence.counterparty, false) {
                        update.counterparty = Some(c.clone());
                    }
                }
            }
            FillMode::Manual => {
                // 事后识别：只填空字段，绝不覆盖已录入内容；同样过门控
                if current.voucher_date.is_empty()
                    && fields.date.is_some()
                    && should_autofill(fields.confidence.date, false)
                {
                    update.voucher_date = fields.date.clone();
                    date_refilled = true;
                }
                if current.amount_cents.is_none()
                    && fields.amount_cents.is_some()
                    && should_autofill(fields.confidence.amount, true)
                {
                    update.amount_cents = fields.amount_cents;
                }
                if current.counterparty.is_empty() {
                    if let Some(c) = &fields.counterparty {
                        if should_autofill(fields.confidence.counterparty, false) {
                            update.counterparty = Some(c.clone());
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
    // **纯词法校验**：目标路径此刻必然还不存在，而 `is_inside_attachments` 依赖
    // `canonicalize()` 成功（对缺失文件返回 false）——旧写法
    // `!is_inside_attachments(&new_abs)` 恒成立并直接 return，于是整条语义化重命名链路
    // 是**死代码**（2026-09-23 审计 B6）。这里与读侧 `is_safe_rel` 同口径
    // （拒 `\` / `:` / 绝对路径 / `..`）。
    if !crate::utils::paths::AppPaths::is_safe_rel(&attachment.file_path) {
        log::warn!("语义化重命名：源相对路径非法，已跳过 {new_rel:?}");
        return Ok(());
    }
    if !crate::utils::paths::AppPaths::is_safe_rel(&new_rel) {
        log::warn!("语义化重命名：目标相对路径非法，已跳过 {new_rel:?}");
        return Ok(());
    }
    let old_abs = ctx.paths.attachment_abs(&attachment.file_path);
    let new_abs = ctx.paths.attachment_abs(&new_rel);

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
    fn extracts_merchant_from_label() {
        let f = extract("销售方：肯德基有限公司\n合计：45.00元");
        assert_eq!(f.merchant, Some("肯德基有限公司".into()));
    }

    #[test]
    fn extracts_merchant_from_receipt_header() {
        // 无标签小票：首个中文行（≥4 字、无冒号）视为店名
        let f = extract("老娘舅餐饮管理\n门店号 021\n实付 32.50 元");
        assert_eq!(f.merchant, Some("老娘舅餐饮管理".into()));
    }

    #[test]
    fn merchant_label_beats_header() {
        // 标签命中的销售方优先于首行兜底
        let f = extract("某某购物中心\n销售方：星巴克咖啡\n合计：38.00元");
        assert_eq!(f.merchant, Some("星巴克咖啡".into()));
    }

    #[test]
    fn amount_source_labeled_vs_unique() {
        let labeled = extract("合计：128.50元");
        assert_eq!(labeled.amount_source, Some(AmountSource::Labeled));
        let unique = extract("出租车票 45.00元");
        assert_eq!(unique.amount_source, Some(AmountSource::UniqueOnly));
        let none = extract("无金额文本");
        assert_eq!(none.amount_source, None);
    }

    #[test]
    fn counterparty_falls_back_to_invoice_seller() {
        // 发票场景：只有「销售方」标签（购买方是记账人自己，不算对方）——
        // 收款场景标签缺失时，销售方即实际收款对象
        let f = extract("购买方：某某科技有限公司\n销售方：顺丰速运有限公司\n价税合计：113.00元");
        assert_eq!(f.counterparty, Some("顺丰速运有限公司".into()));
        assert_eq!(f.merchant, Some("顺丰速运有限公司".into()));
    }

    #[test]
    fn counterparty_falls_back_to_receipt_merchant() {
        // 无标签小票：顶部店名兜底为商家，进而回退为对方账户
        let f = extract("老娘舅餐饮管理\n门店号 021\n实付 32.50 元");
        assert_eq!(f.counterparty, Some("老娘舅餐饮管理".into()));
        assert_eq!(f.merchant, Some("老娘舅餐饮管理".into()));
    }

    #[test]
    fn counterparty_label_beats_merchant_fallback() {
        // 收款场景标签存在时直接命中，不触发商家回退（两者语义重合也以标签为准）
        let f = extract("收款方：美团平台\n销售方：肯德基有限公司\n合计：45.00元");
        assert_eq!(f.counterparty, Some("美团平台".into()));
        assert_eq!(f.merchant, Some("肯德基有限公司".into()));
    }

    #[test]
    fn counterparty_empty_when_no_merchant_either() {
        // 既无收款标签也无可用商家 → 对方账户留空（保守原则）
        let f = extract("2026-08-20 转账 100.00元");
        assert_eq!(f.counterparty, None);
        assert_eq!(f.merchant, None);
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

    // ---------- v0.20：纯文本路径置信度标档 + 自动填单门控 ----------

    #[test]
    fn text_path_confidence_by_source() {
        // 金额：标签命中 Medium；全文唯一兜底 Low
        assert_eq!(
            extract("合计：128.50元").confidence.amount,
            Some(Confidence::Medium)
        );
        assert_eq!(
            extract("出租车票 45.00元").confidence.amount,
            Some(Confidence::Low)
        );

        // 日期：合法日期一律 Medium（纯文本无法区分标签与否）
        assert_eq!(
            extract("2026年8月5日").confidence.date,
            Some(Confidence::Medium)
        );

        // 对方：标签命中 Medium
        assert_eq!(
            extract("收款方：美团平台\n合计：35.00元")
                .confidence
                .counterparty,
            Some(Confidence::Medium)
        );
        // 对方：商家回退（首行店名启发式）→ 沿用商家 Low
        assert_eq!(
            extract("老娘舅餐饮管理\n门店号 021\n实付 32.50 元")
                .confidence
                .counterparty,
            Some(Confidence::Low)
        );
        // 商家：标签 Medium / 首行启发式 Low
        assert_eq!(
            extract("销售方：肯德基有限公司\n合计：45.00元")
                .confidence
                .merchant,
            Some(Confidence::Medium)
        );
        assert_eq!(
            extract("老娘舅餐饮管理\n门店号 021\n实付 32.50 元")
                .confidence
                .merchant,
            Some(Confidence::Low)
        );
    }

    #[test]
    fn autofill_gate_matrix() {
        // 金额（strict）：只有 High 自动填——中低置信进识别结果卡人工确认
        assert!(should_autofill(Some(Confidence::High), true));
        assert!(!should_autofill(Some(Confidence::Medium), true));
        assert!(!should_autofill(Some(Confidence::Low), true));
        assert!(!should_autofill(None, true));
        // 日期/对方（非 strict）：Medium 及以上填，Low 不填
        assert!(should_autofill(Some(Confidence::High), false));
        assert!(should_autofill(Some(Confidence::Medium), false));
        assert!(!should_autofill(Some(Confidence::Low), false));
        assert!(!should_autofill(None, false));
    }

    #[test]
    fn parse_uppercase_amount_basics() {
        assert_eq!(
            parse_uppercase_amount("价税合计（大写）\n壹佰壹拾叁元整"),
            Some(11300)
        );
        assert_eq!(parse_uppercase_amount("壹佰元整"), Some(10000));
        assert_eq!(parse_uppercase_amount("伍元叁角"), Some(530));
        assert_eq!(parse_uppercase_amount("合计：50.00元"), None); // 无大写
        assert_eq!(parse_uppercase_amount("没有任何金额"), None);
    }

    #[test]
    fn extract_validated_uppercase_cross_check() {
        // 纯文本路径：大写与小写一致 → 金额升 High；不一致 → 放弃金额（宁空勿错）
        let ok = extract_validated("价税合计（大写）：壹佰壹拾叁元整\n（小写）¥113.00");
        assert_eq!(ok.amount_cents, Some(11300));
        assert_eq!(ok.confidence.amount, Some(Confidence::High));
        let bad = extract_validated("（大写）壹佰元整\n合计：¥113.00");
        assert_eq!(bad.amount_cents, None);
    }

    // ---------- v0.20 评测修复：复合标签 / 界面噪音词 / 微信「转给」模式 ----------

    #[test]
    fn compound_label_not_hijacked() {
        // 「收款方备注」是别的字段——不得把主标签的后半截（备注）当对方名
        // （2026-09 基准评测：50 例实填错误全是这一模式）
        let f = extract("收款方备注 二维码收款\n金额：35.00元");
        assert_eq!(f.counterparty, None);
        // 「对方账户单号」同理
        let f2 = extract("对方账户单号：12345\n合计：45.00元");
        assert_eq!(f2.counterparty, None);
        // 第二轮（评测 #143/#158 同型）：「收款方开户行」也是复合词
        let f3 = extract("收款方开户行：中国银行\n合计：45.00元");
        assert_eq!(f3.counterparty, None);
    }

    #[test]
    fn name_sub_label_stripped() {
        // 名称类子标签（全称/名称）剥掉后，余下才是值
        let f = extract("收款人全称：张三\n合计：50.00元");
        assert_eq!(f.counterparty, Some("张三".into()));
        let f2 = extract("销售方名称：肯德基有限公司\n合计：45.00元");
        assert_eq!(f2.merchant, Some("肯德基有限公司".into()));
    }

    #[test]
    fn ui_noise_value_rejected() {
        // 抽出的「值」是界面标签词本身（户名/账号…）→ 丢弃，不是真名字
        let f = extract("收款方：户名\n合计：45.00元");
        assert_eq!(f.counterparty, None);
        let f2 = extract("商户：账号\n合计：45.00元");
        assert_eq!(f2.counterparty, None);
        // 等值判定不让真名字受害
        assert!(is_value_noise("户名"));
        assert!(is_value_noise(" 账号 "));
        assert!(is_value_noise("服务"));
        assert!(is_value_noise("开户行"));
        assert!(is_value_noise("的交易")); // 「商户的交易」被截出的残片
        assert!(is_value_noise("收款方开户行")); // 整串界面词被当名字（第三轮评测同型）
        assert!(is_value_noise("户名张")); // 以标签词开头同样拒
                                           // 繁体变体（换 PP-OCRv5 后评测 #207 同型：v5 词表含繁体，「備註」被当名字）
        assert!(is_value_noise("備註"));
        assert!(is_value_noise("戶名"));
        assert!(is_value_noise("帳號"));
        assert!(!is_value_noise("肯德基"));
        assert!(!is_value_noise("服务部"));
        assert!(!is_value_noise("招商银行"));
    }

    #[test]
    fn wechat_transfer_target_extracted() {
        // 微信转账截图（基准里的主要凭证形态）：无「收款方」标签，用「转给」模式
        let f = extract("转账-转给AA001保利盛钢管～丁圆辉\n-5760.00\n当前状态\n等待对方确认收钱");
        assert_eq!(f.counterparty, Some("AA001保利盛钢管～丁圆辉".into()));
        assert_eq!(f.confidence.counterparty, Some(Confidence::Medium));
        let f2 = extract("扫二维码付款-给冰\n-2000.00\n支付成功");
        assert_eq!(f2.counterparty, Some("冰".into()));
    }
}
