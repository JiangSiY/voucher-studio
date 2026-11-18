//! 命名规则（对应 v4.3 的 `naming.py` + naming_rules.json）
//!
//! 模板令牌：
//! - `{date}`  → YYYYMMDD（紧凑日期，如 20260819）
//! - `{date-}` → YYYY-MM-DD（连字符日期，如 2026-08-19；对齐档案档号样式）
//! - `{seq}`   → 当日三位序号（001 起）
//! - `{n}`     → 当日两位序号（01 起）
//!
//! 默认模板 `{date-}-{n}` 生成 2026-08-19-01（参照原始凭证档案版式）。

use rusqlite::Connection;

use crate::db::repos::voucher_repo;
use crate::{CoreError, Result};

pub const DEFAULT_TEMPLATE: &str = "{date-}-{n}";

pub struct NamingEngine {
    pub template: String,
}

impl NamingEngine {
    pub fn new(template: &str) -> Self {
        let t = template.trim();
        Self {
            template: if t.is_empty() {
                DEFAULT_TEMPLATE.into()
            } else {
                t.to_string()
            },
        }
    }

    /// 模板校验：未知令牌报错；无序号令牌（{seq}/{n}）时自动在末尾追加保证唯一性
    pub fn validate(template: &str) -> Result<String> {
        let t = template.trim();
        if t.is_empty() {
            return Ok(DEFAULT_TEMPLATE.into());
        }
        let mut rest = t;
        let mut has_seq = false;
        while let Some(start) = rest.find('{') {
            let end = rest
                .find('}')
                .ok_or_else(|| CoreError::Invalid("命名模板存在未闭合的 {".into()))?;
            let token = &rest[start + 1..end];
            match token {
                "date" | "date-" => {}
                "seq" | "n" => has_seq = true,
                other => {
                    return Err(CoreError::Invalid(format!(
                    "命名模板不支持令牌 {{{other}}}（可用：{{date}}、{{date-}}、{{seq}}、{{n}}）"
                )))
                }
            }
            rest = &rest[end + 1..];
        }
        if has_seq {
            Ok(t.to_string())
        } else {
            Ok(format!("{t}-{{n}}"))
        }
    }

    /// 生成下一个凭证编号（同前缀计数 + 1；若编号已被占用则继续递增，防删除空洞碰撞）
    pub fn next_no(&self, conn: &Connection, date: &str) -> Result<String> {
        let compact = date.replace(['-', '/'], "");
        let hyphen = date.replace('/', "-");
        let prefix = self
            .template
            .replace("{date-}", &hyphen)
            .replace("{date}", &compact)
            .replace("{seq}", "")
            .replace("{n}", "");
        let mut seq = voucher_repo::count_with_prefix(conn, &prefix)? + 1;
        loop {
            let candidate = self.render(&compact, &hyphen, seq);
            // 中途删除会产生编号空洞：count+1 可能撞上已存在编号，递增直到可用
            if voucher_repo::get_by_no(conn, &candidate)?.is_none() {
                return Ok(candidate);
            }
            seq += 1;
        }
    }

    /// 渲染预览（表单里实时展示给用户）
    pub fn preview(&self, date: &str, seq: u32) -> String {
        let compact = date.replace(['-', '/'], "");
        let hyphen = date.replace('/', "-");
        self.render(&compact, &hyphen, seq as i64)
    }

    fn render(&self, compact_date: &str, hyphen_date: &str, seq: i64) -> String {
        self.template
            // 先替换 {date-} 再替换 {date}（前者是更长的字面量）
            .replace("{date-}", hyphen_date)
            .replace("{date}", compact_date)
            .replace("{seq}", &format!("{:03}", seq))
            .replace("{n}", &format!("{:02}", seq))
    }
}

/// V5「附件名称」公式的复刻（Center v0.26）。
///
/// 台账里每条单据/流水/电费/项目都有一个由公式算出的附件名，人工把图片拖进 WPS 时
/// 就按这个名字保存。凭证库要在「重命名」这一步产出一模一样的名字，回写台账、日后
/// 人工核对才不用二次翻译。公式原文见 `docs/开发方案.md`「附件命名」一节。
///
/// 三个约定，全部来自公式而不是我们的发明：
/// - 日期前缀是 `yyMMdd`（不是凭证号那种 YYYYMMDD）；
/// - 项目取「第一个 `-` 之前」的段（`张帅-585kW-自投电站` → `张帅`），空则 `公司费用`；
/// - 序号一律补零到 4 位（单据号/流水序号都是 AutoNumber，可能上千）。
pub mod archive {
    /// `2026-07-16` / `2026/07/16` → `260716`；无法解析时返回空串（公式里 ISBLANK 分支）
    pub fn compact_date(date: &str) -> String {
        let digits: String = date.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 8 {
            let (y, m, d) = (&digits[0..4], &digits[4..6], &digits[6..8]);
            format!("{}{}{}", &y[2..], m, d)
        } else {
            String::new()
        }
    }

    /// 日期前缀：有日期则 `260716-`，否则空串
    pub fn date_prefix(date: &str) -> String {
        let c = compact_date(date);
        if c.is_empty() {
            String::new()
        } else {
            format!("{c}-")
        }
    }

    /// 项目简称：第一个 `-` 之前的段；空项目 → `公司费用`
    pub fn project_short(project: &str) -> String {
        let p = project.trim();
        if p.is_empty() {
            return "公司费用".to_string();
        }
        match p.find('-') {
            Some(i) => p[..i].to_string(),
            None => p.to_string(),
        }
    }

    /// 「3 单据」附件名：`{yyMMdd}-{项目简称}-{应收|应付|仓}{单据号:0000}[-{对方|物料}]`
    ///
    /// 尾段规则与公式一致：应付看对方，其余（应收/仓库）看物料——应收单的对方已经
    /// 隐含在项目简称里，再拼一次反而重名。
    pub fn doc(
        date: &str,
        project: &str,
        doc_type: &str,
        doc_no: i64,
        counterparty: &str,
        material: &str,
    ) -> String {
        let kind = match doc_type {
            "应付" => "应付",
            "应收" => "应收",
            _ => "仓",
        };
        let tail = if doc_type == "应付" {
            counterparty.trim()
        } else {
            material.trim()
        };
        let mut s = format!(
            "{}{}-{}{:04}",
            date_prefix(date),
            project_short(project),
            kind,
            doc_no
        );
        if !tail.is_empty() {
            s.push('-');
            s.push_str(tail);
        }
        s
    }

    /// 「4 资金流水」附件名：`{yyMMdd}-{项目简称}-{收|付}{流水序号:0000}[-{对方}]`
    ///
    /// 只有付款拼对方（收款单的对方在台账里就是客户，项目简称已能定位）。
    pub fn cash(date: &str, project: &str, flow_dir: &str, seq: i64, counterparty: &str) -> String {
        let kind = if flow_dir == "付" { "付" } else { "收" };
        let mut s = format!(
            "{}{}-{}{:04}",
            date_prefix(date),
            project_short(project),
            kind,
            seq
        );
        let tail = counterparty.trim();
        if kind == "付" && !tail.is_empty() {
            s.push('-');
            s.push_str(tail);
        }
        s
    }

    /// 「6 电费」附件名：`{yyMMdd}-{电表}-抄{电费月度 yyMM}-{负?}{金额}`
    ///
    /// 逐字复刻 V5 公式：日期空则省前缀；电表空则不写表号；电费为负加「负」；
    /// 金额按整数 / 一位小数 / 两位小数三档格式化（`TEXT(x,"0"/"0.0"/"0.00")`）。
    pub fn meter(date: &str, meter_no: &str, meter_month: &str, amount_cents: i64) -> String {
        let mut s = String::new();
        s.push_str(&date_prefix(date));
        s.push_str(meter_no.trim());
        s.push_str("-抄");
        s.push_str(&compact_month(meter_month));
        s.push('-');
        if amount_cents < 0 {
            s.push('负');
        }
        s.push_str(&money_abs(amount_cents));
        s
    }

    /// `2026-11` / `2026年11月` / `2026/11` → `2611`；解析不出返回空串
    pub fn compact_month(month: &str) -> String {
        let digits: String = month.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() < 6 {
            return String::new();
        }
        format!("{}{}", &digits[2..4], &digits[4..6])
    }

    /// 金额绝对值按 V5 的三档格式：整数不带小数、10 的倍数一位小数、其余两位
    pub fn money_abs(amount_cents: i64) -> String {
        let abs = amount_cents.unsigned_abs();
        let yuan = abs / 100;
        let cent = abs % 100;
        if cent == 0 {
            format!("{yuan}")
        } else if cent.is_multiple_of(10) {
            format!("{yuan}.{}", cent / 10)
        } else {
            format!("{yuan}.{cent:02}")
        }
    }

    /// 「2 项目」附件名：`{客户}-{业务类型}[-{签约日期 yyMMdd}]`
    pub fn project(customer: &str, biz_type: &str, sign_date: &str) -> String {
        let mut s = format!("{}-{}", customer.trim(), biz_type.trim());
        let c = compact_date(sign_date);
        if !c.is_empty() {
            s.push('-');
            s.push_str(&c);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_doc_name_matches_v5_formula() {
        use archive::*;
        // V5 例子：260716-张帅-应付0001-某供应商
        assert_eq!(
            doc(
                "2026-07-16",
                "张帅-585kW-自投电站",
                "应付",
                1,
                "某供应商",
                ""
            ),
            "260716-张帅-应付0001-某供应商"
        );
        // 应收：尾段取物料（这里为空，故只有两段）
        assert_eq!(
            doc("2026/09/02", "张帅", "应收", 4, "张帅", ""),
            "260902-张帅-应收0004"
        );
        // 仓库：kind=仓，尾段取物料
        assert_eq!(
            doc("2026-09-02", "", "仓库", 12, "", "电缆"),
            "260902-公司费用-仓0012-电缆"
        );
        // 无日期：公式的 ISBLANK 分支 → 无前缀
        assert_eq!(doc("", "张帅", "应收", 4, "", ""), "张帅-应收0004");
    }

    #[test]
    fn archive_cash_name_matches_v5_formula() {
        use archive::*;
        assert_eq!(
            cash("2025-11-09", "凯诗蓝德-1MW", "收", 1, "谁都不拼"),
            "251109-凯诗蓝德-收0001"
        );
        assert_eq!(
            cash("2025-11-09", "凯诗蓝德", "付", 43, "某供应商"),
            "251109-凯诗蓝德-付0043-某供应商"
        );
        assert_eq!(
            cash("2025-11-09", "", "付", 2, ""),
            "251109-公司费用-付0002"
        );
    }

    #[test]
    fn archive_meter_name_matches_v5_formula() {
        use archive::*;
        // 整数金额：不带小数
        assert_eq!(
            meter("2026-09-03", "表A-01", "2026-10", 123_400),
            "260903-表A-01-抄2610-1234"
        );
        // 一位小数（10 的倍数）
        assert_eq!(
            meter("2026-09-03", "表A-01", "2026-10", 123_450),
            "260903-表A-01-抄2610-1234.5"
        );
        // 两位小数
        assert_eq!(
            meter("2026-09-03", "表A-01", "2026-10", 123_456),
            "260903-表A-01-抄2610-1234.56"
        );
        // 负数加「负」；无日期、无表号也不能留悬挂的「-」
        assert_eq!(meter("", "", "2026-01", -5_000), "-抄2601-负50");
    }

    #[test]
    fn archive_project_name_matches_v5_formula() {
        use archive::*;
        assert_eq!(
            project("张帅", "工商业光伏", "2026-07-16"),
            "张帅-工商业光伏-260716"
        );
        assert_eq!(project("张帅", "运维", ""), "张帅-运维");
    }

    #[test]
    fn validate_accepts_known_tokens() {
        assert_eq!(
            NamingEngine::validate("{date}-{seq}").unwrap(),
            "{date}-{seq}"
        );
        assert_eq!(
            NamingEngine::validate("{date-}-{n}").unwrap(),
            "{date-}-{n}"
        );
        assert_eq!(NamingEngine::validate("").unwrap(), DEFAULT_TEMPLATE);
        // 无序号令牌自动补（统一补新样式 {n}）
        assert_eq!(NamingEngine::validate("{date}").unwrap(), "{date}-{n}");
        assert_eq!(NamingEngine::validate("{date-}").unwrap(), "{date-}-{n}");
        assert!(NamingEngine::validate("{year}-{seq}").is_err());
        assert!(NamingEngine::validate("{date").is_err());
    }

    #[test]
    fn preview_renders() {
        let legacy = NamingEngine::new("{date}-{seq}");
        assert_eq!(legacy.preview("2026-08-19", 1), "20260819-001");
        assert_eq!(legacy.preview("2026-08-19", 42), "20260819-042");

        let archive = NamingEngine::new("{date-}-{n}");
        assert_eq!(archive.preview("2026-08-19", 1), "2026-08-19-01");
        assert_eq!(archive.preview("2026-08-19", 12), "2026-08-19-12");
    }
}
