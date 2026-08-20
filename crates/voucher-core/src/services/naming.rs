//! 命名规则（对应 v4.3 的 `naming.py` + naming_rules.json）
//!
//! 模板令牌：`{date}` → YYYYMMDD；`{seq}` → 当日三位序号（001 起）。
//! 默认模板 `{date}-{seq}` 生成 20260819-001。

use rusqlite::Connection;

use crate::db::repos::voucher_repo;
use crate::{CoreError, Result};

pub const DEFAULT_TEMPLATE: &str = "{date}-{seq}";

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

    /// 模板校验：未知令牌报错；无 {seq} 时自动在末尾追加保证唯一性
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
                "date" => {}
                "seq" => has_seq = true,
                other => {
                    return Err(CoreError::Invalid(format!(
                        "命名模板不支持令牌 {{{other}}}（可用：{{date}}、{{seq}}）"
                    )))
                }
            }
            rest = &rest[end + 1..];
        }
        if has_seq {
            Ok(t.to_string())
        } else {
            Ok(format!("{t}-{{seq}}"))
        }
    }

    /// 生成下一个凭证编号（同前缀计数 + 1；若编号已被占用则继续递增，防删除空洞碰撞）
    pub fn next_no(&self, conn: &Connection, date: &str) -> Result<String> {
        let compact = date.replace(['-', '/'], "");
        let prefix = self
            .template
            .replace("{date}", &compact)
            .replace("{seq}", "");
        let mut seq = voucher_repo::count_with_prefix(conn, &prefix)? + 1;
        loop {
            let candidate = self.render(&compact, seq);
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
        self.render(&compact, seq as i64)
    }

    fn render(&self, compact_date: &str, seq: i64) -> String {
        self.template
            .replace("{date}", compact_date)
            .replace("{seq}", &format!("{:03}", seq))
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
        assert_eq!(NamingEngine::validate("").unwrap(), DEFAULT_TEMPLATE);
        // 无 {seq} 自动补
        assert_eq!(NamingEngine::validate("{date}").unwrap(), "{date}-{seq}");
        assert!(NamingEngine::validate("{year}-{seq}").is_err());
        assert!(NamingEngine::validate("{date").is_err());
    }

    #[test]
    fn preview_renders() {
        let engine = NamingEngine::new("{date}-{seq}");
        assert_eq!(engine.preview("2026-08-19", 1), "20260819-001");
        assert_eq!(engine.preview("2026-08-19", 42), "20260819-042");
    }
}
