//! OCR 版面分析（v5）：几何键值配对 + 文档类型分类 + 字段置信度
//!
//! 与 ocr_fields（纯文本抽取）互补：有版面数据（坐标框）时走本模块，
//! 几何关系（同行右邻 / 跨行缩进 / 栏位 x 坐标）作为字段配对依据，
//! 解决纯字符串 find 的「内容与字段不符」问题（左右分栏、错行文本被
//! 当成标签值等）。无版面（云端引擎 / 旧数据）回退 ocr_fields。
//!
//! 置信度分级（v0.20 起自动填单按此门控，`ocr_fields::should_autofill`）：
//! - High  ：几何 KV 配对 / 标签命中 + 版面佐证 / 大小写金额互验一致
//! - Medium：纯文本标签命中（无几何佐证）
//! - Low   ：启发式兜底（首行店名、全文唯一金额）——不自动填，仅识别结果卡人工确认

use crate::models::OcrLine;

use super::ocr_fields::{self, Confidence, FieldConfidence};

/// 文档类型（关键词 + 版面特征分类，供抽取策略选择 / 评测报表统计）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    /// 增值税发票：价税合计 / 购买方 / 销售方 / 发票代码
    VatInvoice,
    /// 定额/卷式发票：发票代码 + 无合计标签（金额即票面大字）
    QuotaInvoice,
    /// 收银小票：门店 / 收银员 / 实收 / 支付方式
    Receipt,
    /// 出租车票：车费 / 上下车时间
    Taxi,
    /// 转账/支付截图：转账 / 收款 / 付款
    Transfer,
    /// 未识别（默认保守策略）
    Unknown,
}

impl DocKind {
    /// 报表用中文名（评测报表按类型统计用）
    pub fn label(self) -> &'static str {
        match self {
            DocKind::VatInvoice => "增值税发票",
            DocKind::QuotaInvoice => "定额/卷式发票",
            DocKind::Receipt => "收银小票",
            DocKind::Taxi => "出租车票",
            DocKind::Transfer => "转账/支付截图",
            DocKind::Unknown => "未识别",
        }
    }
}

/// 按识别文本判文档类型（纯文本关键词规则；版面路径与评测报表共用同一实现）
pub fn classify_text(text: &str) -> DocKind {
    let has = |k: &str| text.contains(k);
    if has("价税合计") || (has("购买方") && has("销售方")) || (has("发票代码") && has("税率"))
    {
        DocKind::VatInvoice
    } else if has("发票代码") || has("发票号码") {
        DocKind::QuotaInvoice
    } else if has("实收") || has("收银") || has("小票") || has("门店") {
        DocKind::Receipt
    } else if has("出租") || has("车费") {
        DocKind::Taxi
    } else if has("转账") || has("已收款") || has("付款") {
        DocKind::Transfer
    } else {
        DocKind::Unknown
    }
}

fn classify(lines: &[OcrLine]) -> DocKind {
    let all: String = lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    classify_text(&all)
}

/// 有版面数据的字段抽取（入口）。置信度随字段返回；
/// 任何字段抽不到就 None——宁可留空也不填错。
pub fn extract_with_layout(lines: &[OcrLine]) -> ocr_fields::ExtractedFields {
    // 纯文本路径的结果作为底座（金额千分位解析、日期合法性校验复用）
    let text: String = lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut base = ocr_fields::extract(&text);

    let mut conf = FieldConfidence::default();
    let _kind = classify(lines);

    // ---- 日期：标签行配对（开票日期/交易日期/日期，排除打印时间等干扰）> 全文唯一 > 放弃 ----
    match pair_date_value(lines) {
        Some((v, _geometric)) => {
            if let Some(d) = ocr_fields::extract_date(&v) {
                base.date = Some(d);
                conf.date = Some(Confidence::High);
            }
        }
        None => {
            // 无标签：唯一日期才取（Medium）；多个候选放弃——
            // 现状「取第一个」会命中打印日期/有效期等干扰日期
            let dates = all_dates(lines);
            if dates.len() == 1 {
                base.date = dates.first().cloned();
                conf.date = Some(Confidence::Medium);
            } else {
                base.date = None;
            }
        }
    }

    // ---- 金额：几何配对（两栏布局——标签与值分成两个框各自成行，
    //      如支付成功页「支付金额 / 2,088.00」；2026-10-09 用户实报场景）
    //      > 底座文本逻辑（标签同行 / 唯一兜底）；最后大写互验提置信 ----
    if let Some((v, geometric)) = pair_label_value(lines, &ocr_fields::AMOUNT_LABELS, &|v| {
        ocr_fields::first_money_in(v).is_some()
    }) {
        if let Some(cents) = ocr_fields::first_money_in(&v) {
            base.amount_cents = Some(cents);
            base.amount_source = Some(ocr_fields::AmountSource::Labeled);
            conf.amount = Some(if geometric {
                Confidence::High
            } else {
                Confidence::Medium
            });
        }
    }
    if conf.amount.is_none() && base.amount_cents.is_some() {
        // 未配对：底座文本逻辑（extract 已算好），按来源分级（标签 Medium / 唯一兜底 Low）
        conf.amount = base.amount_source.map(|s| match s {
            ocr_fields::AmountSource::Labeled => Confidence::Medium,
            ocr_fields::AmountSource::UniqueOnly => Confidence::Low,
        });
    }
    if base.amount_cents.is_some() {
        if let Some(cap_cents) = uppercase_amount_in(lines) {
            // 大写与小写一致 → High（双重来源互证）；不一致 → 放弃金额（可能读错）
            if cap_cents == base.amount_cents.unwrap_or(0) {
                conf.amount = Some(Confidence::High);
            } else {
                base.amount_cents = None;
                base.amount_source = None;
                conf.amount = None;
            }
        }
    }

    // ---- 对方账户：几何 KV 配对 > 纯文本标签（已有）> 商家回退 ----
    // 配对值清洗后不合格（空/噪音词/过短）→ 视为未配对，继续走商家回退
    // （邻居已改为返回原文，清洗在此完成）
    let cp_pair = pair_label_value(lines, &COUNTERPARTY_LABELS, &|v| {
        !v.is_empty() && contains_name_char(v)
    })
    .and_then(|(v, geometric)| {
        let name = clean_name(&v, 30);
        (name.chars().count() >= 2).then_some((name, geometric))
    });
    match cp_pair {
        Some((name, geometric)) => {
            base.counterparty = Some(name);
            conf.counterparty = Some(if geometric {
                Confidence::High
            } else {
                Confidence::Medium
            });
        }
        None => {
            // 回退商家（发票销售方 / 小票店名）时置信度沿用商家策略
            if let Some((name, c)) = merchant_with_confidence(lines, &text) {
                base.merchant = Some(name.clone());
                conf.merchant = Some(c);
                if base.counterparty.is_none() {
                    base.counterparty = Some(name);
                    conf.counterparty = Some(c);
                }
            }
        }
    }

    // ---- 商家：独立判定（摘要候选）----
    if base.merchant.is_none() {
        if let Some((name, c)) = merchant_with_confidence(lines, &text) {
            base.merchant = Some(name);
            conf.merchant = Some(c);
        }
    }

    base.confidence = conf;
    // v0.12：后验校验层（年份窗口 / 金额多数互验 / 账号模式），改写 confidence
    {
        use chrono::Datelike;
        let now_year = chrono::Local::now().date_naive().year();
        crate::services::ocr_validate::apply(&text, &mut base, now_year);
    }
    base
}

const DATE_LABELS: [&str; 4] = ["开票日期", "交易日期", "转账日期", "日期"];
/// 日期负面标签：含这些词的行不是凭证日期（打印时间/有效期等干扰源）
const DATE_NEGATIVE_LABELS: [&str; 4] = ["打印时间", "打印日期", "有效期", "到期日"];
const COUNTERPARTY_LABELS: [&str; 6] = ["收款方", "对方账户", "商户", "收款人", "销售方", "购买方"];

/// 日期标签配对：过滤负面标签行后走通用 KV 配对
fn pair_date_value(lines: &[OcrLine]) -> Option<(String, bool)> {
    let filtered: Vec<OcrLine> = lines
        .iter()
        .filter(|l| !DATE_NEGATIVE_LABELS.iter().any(|n| l.text.contains(n)))
        .cloned()
        .collect();
    pair_label_value(&filtered, &DATE_LABELS, &|v| !v.is_empty())
}

/// 标签 → 值配对（版面核心）。三种命中形态，按可信度降序：
/// 1. 同行右邻：标签行 x1 之后同一水平线上的相邻行（左右分栏 KV）
/// 2. 同行冒号：标签行内冒号后的剩余文本（单栏 KV）
/// 3. 跨行缩进：标签独立成行，下一行 x0 对齐且 y 紧邻
///
/// 返回 (值文本, 是否几何佐证[形态 1/3 为 true])。
///
/// `valid`：候选值判定器——名称类要求含字母/中文（排除纯数字/符号），
/// 日期类只要求非空，金额类要求值里**含金额数字**（「价税合计（大写）」后的
/// 大写值会被自然跳过，继续找下一处）。
fn pair_label_value(
    lines: &[OcrLine],
    labels: &[&str],
    valid: &dyn Fn(&str) -> bool,
) -> Option<(String, bool)> {
    for label in labels {
        for (i, line) in lines.iter().enumerate() {
            let Some(idx) = line.text.find(*label) else {
                continue;
            };
            // 子标签处理（v0.20 评测修复）：复合标签（收款方备注…）本次命中作废；
            // 名称类子标签（收款人全称：…）剥掉后取余下为值
            let Some(rest) = ocr_fields::strip_sub_label(&line.text[idx + label.len()..]) else {
                continue;
            };

            // 形态 2：行内冒号（无几何佐证——值就在标签行里）
            let colon_stripped = rest
                .trim()
                .trim_start_matches(['：', ':', ' ', '　'])
                .trim();
            if valid(colon_stripped) {
                return Some((colon_stripped.to_string(), false));
            }

            // 形态 1：同行右邻（x 坐标在标签行右侧、y 水平重叠）
            if let Some((v, _)) = right_neighbor(lines, i) {
                if valid(&v) {
                    return Some((v, true));
                }
            }

            // 形态 3：跨行（下一行 x0 与标签 x0 接近、y 间隙小于 1.5 倍行高）
            if rest.trim().is_empty() {
                if let Some(v) = below_neighbor(lines, i) {
                    if valid(&v) {
                        return Some((v, true));
                    }
                }
            }
        }
    }
    None
}

/// 同一水平线上、x 在 line i 右侧最近的行**原文**。
/// 取值不做清洗——判定与清洗交给调用方（名称类走 `clean_name`，金额类需要保留数字：
/// 曾因在此处清洗把「2,088.00」剥成空串，两栏布局的金额永远配不上对，2026-10-09 修复）。
fn right_neighbor(lines: &[OcrLine], i: usize) -> Option<(String, usize)> {
    let me = &lines[i];
    let my_h = me.height().max(1.0);
    let mut best: Option<(f32, usize)> = None;
    for (j, other) in lines.iter().enumerate() {
        if j == i {
            continue;
        }
        // 水平重叠：中心 y 差小于较小行高的一半
        if (other.cy() - me.cy()).abs() > my_h.min(other.height().max(1.0)) / 2.0 {
            continue;
        }
        if other.x0() > me.x1() - my_h * 0.5 {
            let gap = other.x0() - me.x1();
            if best.map(|(g, _)| gap < g).unwrap_or(true) {
                best = Some((gap, j));
            }
        }
    }
    best.map(|(_, j)| (lines[j].text.trim().to_string(), j))
        .filter(|(v, _)| !v.is_empty())
}

/// 下一行（y 紧邻、x0 对齐标签行 x0 ± 行高）的**原文**（清洗同样交给调用方）
fn below_neighbor(lines: &[OcrLine], i: usize) -> Option<String> {
    let me = &lines[i];
    let my_h = me.height().max(1.0);
    let mut best: Option<(f32, usize)> = None;
    for (j, other) in lines.iter().enumerate() {
        if j == i {
            continue;
        }
        let dy = other.box_xyxy[1] - me.box_xyxy[3]; // 顶边 - 我的底边
        if dy < -my_h * 0.2 || dy > my_h * 1.5 {
            continue;
        }
        if (other.x0() - me.x0()).abs() <= my_h * 1.2 && best.map(|(d, _)| dy < d).unwrap_or(true) {
            best = Some((dy, j));
        }
    }
    best.map(|(_, j)| lines[j].text.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// 商家（含置信度）：KV 标签 High > 纯文本标签 Medium > 首行大字号 Low
fn merchant_with_confidence(lines: &[OcrLine], text: &str) -> Option<(String, Confidence)> {
    // KV 配对：销售方等标签
    if let Some((v, geometric)) = pair_label_value(lines, &MERCHANT_LABELS, &|v| {
        !v.is_empty() && contains_name_char(v)
    }) {
        let name = clean_name(&v, 30);
        if name.chars().count() >= 2 {
            return Some((
                name,
                if geometric {
                    Confidence::High
                } else {
                    Confidence::Medium
                },
            ));
        }
    }
    // 纯文本兜底（沿用 ocr_fields 逻辑，Medium）
    let base = ocr_fields::extract(text);
    if let Some(m) = base.merchant {
        return Some((m, Confidence::Medium));
    }
    // 版面启发式：首行（小票顶部店名）且行高 ≥ 全图中位行高（大字号=店名）Low
    if let Some(first) = lines
        .iter()
        .filter(|l| l.text.chars().count() >= 4 && !l.text.contains('：') && !l.text.contains(':'))
        .min_by_key(|l| l.box_xyxy[1] as i64)
    {
        let median_h = median_line_height(lines);
        if first.height() >= median_h {
            let name = clean_name(&first.text, 30);
            if name.chars().count() >= 2 && contains_name_char(&name) {
                return Some((name, Confidence::Low));
            }
        }
    }
    None
}

const MERCHANT_LABELS: [&str; 4] = ["销售方", "开票方", "销售单位", "商家名称"];

/// 行高中位数（识别店名大字号的参照）
fn median_line_height(lines: &[OcrLine]) -> f32 {
    let mut hs: Vec<f32> = lines
        .iter()
        .map(|l| l.height())
        .filter(|h| *h > 0.0)
        .collect();
    if hs.is_empty() {
        return 0.0;
    }
    hs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    hs[hs.len() / 2]
}

/// 全文所有合法日期（YYYY-MM-DD 规整）
fn all_dates(lines: &[OcrLine]) -> Vec<String> {
    let mut out = Vec::new();
    for l in lines {
        if let Some(d) = ocr_fields::extract_date(&l.text) {
            if !out.contains(&d) {
                out.push(d);
            }
        }
    }
    out
}

/// 大写金额识别与转换：逐行解析（实现收在 ocr_fields::parse_uppercase_amount，
/// 与纯文本互验共用，保证两条路径口径一致）。OCR 错认时返回 None。
fn uppercase_amount_in(lines: &[OcrLine]) -> Option<i64> {
    let text: String = lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    ocr_fields::parse_uppercase_amount(&text)
}

/// 清洗为名称候选：去标点/空白前缀、截掉金额尾巴；界面噪音词（户名/账号/备注…）→ 空串
/// （v0.20：版面配对也可能抓到相邻的界面标签，此处是两条路径共同的最后关口）
fn clean_name(s: &str, max: usize) -> String {
    let cleaned = s
        .trim()
        .trim_start_matches(['：', ':', ' ', '　', '-', '—'])
        .trim();
    let name: String = cleaned
        .chars()
        .take_while(|c| !c.is_ascii_digit() && *c != '¥' && *c != '￥')
        .collect();
    let name = name.trim();
    if ocr_fields::is_value_noise(name) {
        return String::new();
    }
    name.chars().take(max).collect()
}

/// 是否含「名称性字符」（中文/字母——排除纯数字、纯符号被当名称）
fn contains_name_char(s: &str) -> bool {
    s.chars().any(|c| c.is_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, x0: f32, y0: f32, x1: f32, y1: f32) -> OcrLine {
        OcrLine {
            text: text.into(),
            score: 0.9,
            box_xyxy: [x0, y0, x1, y1],
        }
    }

    /// 增值税发票典型版面：购买方/销售方左右分栏 + 价税合计 + 开票日期
    /// 纯文本 find 的失效场景：销售方标签行的右侧同行是购买方的值（错栏配对）
    #[test]
    fn vat_invoice_right_neighbor_pairing() {
        let lines = vec![
            line("购买方", 40.0, 100.0, 110.0, 130.0),
            line("某某科技有限公司", 160.0, 100.0, 320.0, 130.0),
            line("销售方", 40.0, 150.0, 110.0, 180.0),
            line("顺丰速运有限公司", 160.0, 150.0, 320.0, 180.0),
            line("价税合计（大写）", 40.0, 400.0, 200.0, 430.0),
            line("壹佰壹拾叁元整", 220.0, 400.0, 360.0, 430.0),
            line("开票日期：2026-08-20", 40.0, 500.0, 400.0, 530.0),
        ];
        let f = extract_with_layout(&lines);
        // 销售方右邻 = 顺丰（不是同行的"某某科技"——那是购买方的值）
        assert_eq!(f.counterparty.as_deref(), Some("顺丰速运有限公司"));
        // 大写金额 113 元整与标签金额互验（无小写金额时金额仍 None——此版面无小写行）
        assert_eq!(f.date.as_deref(), Some("2026-08-20"));
        assert_eq!(f.confidence.date, Some(Confidence::High));
    }

    /// 销售方标签 + 右侧小写金额同时存在：大写互验通过
    #[test]
    fn uppercase_amount_cross_validation() {
        let lines = vec![
            line("价税合计（大写）", 40.0, 400.0, 200.0, 430.0),
            line("壹佰壹拾叁元整", 220.0, 400.0, 360.0, 430.0),
            line("（小写）", 400.0, 400.0, 460.0, 430.0),
            line("¥113.00", 470.0, 400.0, 560.0, 430.0),
        ];
        let f = extract_with_layout(&lines);
        assert_eq!(f.amount_cents, Some(11300));
        assert_eq!(f.confidence.amount, Some(Confidence::High));
    }

    /// 大写与小写不一致（OCR 读错其一）→ 放弃金额（宁空勿错）
    #[test]
    fn mismatched_uppercase_rejected() {
        let lines = vec![
            line("价税合计（大写）", 40.0, 400.0, 200.0, 430.0),
            line("壹佰元整", 220.0, 400.0, 320.0, 430.0),
            line("合计：¥113.00", 40.0, 440.0, 300.0, 470.0),
        ];
        let f = extract_with_layout(&lines);
        assert_eq!(f.amount_cents, None);
    }

    /// 打印时间是干扰（负面标签排除），乘车日期是真实业务日期（标签命中）
    #[test]
    fn print_time_excluded_ride_date_wins() {
        let lines = vec![
            line("打印时间 2026-08-19 23:50", 40.0, 40.0, 400.0, 70.0),
            line("出租车票", 40.0, 100.0, 200.0, 140.0),
            line("45.00元", 300.0, 200.0, 400.0, 230.0),
            line("乘车日期 2026-08-20", 40.0, 300.0, 400.0, 330.0),
        ];
        let f = extract_with_layout(&lines);
        // 负面标签「打印时间」被排除；「日期」标签命中乘车日期 → 取 08-20 而非 08-19
        assert_eq!(f.date.as_deref(), Some("2026-08-20"));
        assert_eq!(f.confidence.date, Some(Confidence::High));
    }

    /// 多个候选日期且无任何标签 → 放弃（宁空勿错）
    #[test]
    fn multiple_unlabeled_dates_give_up() {
        let lines = vec![
            line("2026-08-19 23:50", 40.0, 40.0, 400.0, 70.0),
            line("出租车票", 40.0, 100.0, 200.0, 140.0),
            line("45.00元", 300.0, 200.0, 400.0, 230.0),
            line("2026-08-20 08:15", 40.0, 300.0, 400.0, 330.0),
        ];
        let f = extract_with_layout(&lines);
        assert_eq!(f.date, None);
    }

    /// 小票顶部店名：首行 + 大字号 → 店名正确（纯文本「首个中文行」兜底命中，Medium）
    #[test]
    fn receipt_header_merchant() {
        let lines = vec![
            line("老娘舅餐饮管理有限公司", 40.0, 40.0, 500.0, 100.0), // 高 60（大字）
            line("门店号 021", 40.0, 120.0, 300.0, 140.0),            // 高 20
            line("实付 32.50 元", 40.0, 200.0, 300.0, 220.0),         // 高 20
        ];
        let f = extract_with_layout(&lines);
        assert_eq!(f.merchant.as_deref(), Some("老娘舅餐饮管理有限公司"));
        assert_eq!(f.confidence.merchant, Some(Confidence::Medium));
        // 金额走标签「实付」
        assert_eq!(f.amount_cents, Some(3250));
    }

    /// 跨行 KV：标签独立行，值在下一行对齐（发票抬头的场景）
    #[test]
    fn below_row_pairing() {
        let lines = vec![
            line("销售方", 40.0, 150.0, 110.0, 180.0),
            line("肯德基有限公司", 42.0, 190.0, 250.0, 220.0),
            line("合计：45.00元", 40.0, 400.0, 300.0, 430.0),
        ];
        let f = extract_with_layout(&lines);
        assert_eq!(f.counterparty.as_deref(), Some("肯德基有限公司"));
        assert_eq!(f.confidence.counterparty, Some(Confidence::High));
    }

    /// 空版面 / 干扰行不崩（鲁棒性）
    #[test]
    fn empty_and_noise_lines_safe() {
        assert_eq!(extract_with_layout(&[]).date, None);
        let noise = vec![
            line("12345", 0.0, 0.0, 100.0, 30.0),
            line("", 0.0, 40.0, 10.0, 60.0),
        ];
        let f = extract_with_layout(&noise);
        assert_eq!(f.date, None);
        assert_eq!(f.counterparty, None);
    }

    /// 2026-10-09 用户实报：两栏支付成功页——「支付金额」标签与「2,088.00」值分成两个框、
    /// 各自成行。修复前金额漏抽（金额无几何配对 + 唯一兜底被同值重复击穿）；
    /// 修复后金额 High（右邻配对）、对方 High、日期 Medium。
    #[test]
    fn two_column_payment_page() {
        let lines = vec![
            line("支付成功", 499.0, 812.0, 716.0, 877.0),
            line("￥2,088.00", 409.0, 927.0, 812.0, 1028.0),
            line("收款方", 42.0, 1232.0, 190.0, 1291.0),
            line(
                "中国太平洋财产保险股份有限公司",
                485.0,
                1234.0,
                1170.0,
                1289.0,
            ),
            line("支付号", 42.0, 1337.0, 191.0, 1396.0),
            line("70260161981822", 801.0, 1341.0, 1172.0, 1390.0),
            line("支付金额", 43.0, 1441.0, 237.0, 1499.0),
            line("2,088.00", 984.0, 1443.0, 1174.0, 1501.0),
            line("支付时间", 43.0, 1545.0, 235.0, 1603.0),
            line("2026-10-09 11:21:30", 721.0, 1550.0, 1171.0, 1598.0),
        ];
        let f = extract_with_layout(&lines);
        assert_eq!(f.amount_cents, Some(208800));
        assert_eq!(f.confidence.amount, Some(Confidence::High));
        assert_eq!(
            f.counterparty.as_deref(),
            Some("中国太平洋财产保险股份有限公司")
        );
        assert_eq!(f.date.as_deref(), Some("2026-10-09"));
    }

    /// 两栏日期的右邻取值（邻居改为返回原文后新增能力：纯数字日期不再被名称清洗剥空）
    #[test]
    fn two_column_date_right_neighbor() {
        let lines = vec![
            line("开票日期", 40.0, 100.0, 160.0, 130.0),
            line("2026-08-20", 300.0, 100.0, 460.0, 130.0),
            line("合计：113.00元", 40.0, 400.0, 300.0, 430.0),
        ];
        let f = extract_with_layout(&lines);
        assert_eq!(f.date.as_deref(), Some("2026-08-20"));
        assert_eq!(f.confidence.date, Some(Confidence::High));
        assert_eq!(f.amount_cents, Some(11300));
    }

    /// v0.20 评测修复：右邻是界面标签词（户名/账号…）→ 不作为对方值
    #[test]
    fn ui_label_right_neighbor_rejected() {
        let lines = vec![
            line("收款方", 40.0, 100.0, 110.0, 130.0),
            line("户名", 160.0, 100.0, 210.0, 130.0),
            line("合计：45.00元", 40.0, 400.0, 300.0, 430.0),
        ];
        let f = extract_with_layout(&lines);
        assert_eq!(f.counterparty, None);
    }

    /// 文档分类冒烟（确保分支可判）
    #[test]
    fn classify_smoke() {
        assert_eq!(
            classify(&[line("价税合计 113.00", 0.0, 0.0, 1.0, 1.0)]),
            DocKind::VatInvoice
        );
        assert_eq!(
            classify(&[line("收银员 001 门店 02", 0.0, 0.0, 1.0, 1.0)]),
            DocKind::Receipt
        );
        assert_eq!(
            classify(&[line("出租车 车费 45.00", 0.0, 0.0, 1.0, 1.0)]),
            DocKind::Taxi
        );
        assert_eq!(
            classify(&[line("随便的文本", 0.0, 0.0, 1.0, 1.0)]),
            DocKind::Unknown
        );
    }
}
