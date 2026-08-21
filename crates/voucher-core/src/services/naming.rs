//! 命名规则（对应 v4.3 的 `naming.py` + naming_rules.json）
//!
//! 模板令牌：
//! - `{date}`  → YYYYMMDD（紧凑日期，如 20260819）
//! - `{date-}` → YYYY-MM-DD（连字符日期，如 2026-08-19；对齐档案档号样式）
//! - `{seq}`   → 当日三位序号（001 起）
//! - `{n}`     → 当日两位序号（01 起）
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
                other => return Err(CoreError::Invalid(format!(
                    "命名模板不支持令牌 {{{other}}}（可用：{{date}}、{{date-}}、{{seq}}、{{n}}）"
                ))),
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

#[cfg(test)]
mod tests {
    use super::*;

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
