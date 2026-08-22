//! JSON 原子读写（对应 v4.3 的 `jsonio.py`）
//!
//! 写入策略：先写临时文件再改名，断电/崩溃不会产生半个配置文件。

use std::path::Path;

use serde::{de::DeserializeOwned, Serialize};

use crate::{CoreError, Result};

/// 原子写入：pretty JSON，tmp + rename
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(value)?;
    std::fs::write(&tmp, content.as_bytes())?;
    // Windows 上 rename 目标存在时需要先移除
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 读取 JSON（文件不存在返回 None）
///
/// G3-1：兼容带 BOM 的 UTF-8（PowerShell 5.1 `Out-File -Encoding utf8`、
/// 记事本保存的文件会带 BOM，serde_json 直接解析会 panic/报错）——
/// 统一剥掉前导 `\u{feff}` 再反序列化。
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let v = serde_json::from_str(content).map_err(CoreError::Json)?;
    Ok(Some(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("voucher-jsonio-{name}.json"));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// G3-1 回归：带 BOM 的 JSON 可正常读取（PowerShell 5.1 / 记事本保存场景）
    #[test]
    fn read_json_strips_bom() {
        let p = tmp_path("bom");
        // 模拟 `Out-File -Encoding utf8`：EF BB BF 前导 + 合法 JSON
        std::fs::write(&p, "\u{feff}{\"a\": 1}").unwrap();
        let v: Option<std::collections::HashMap<String, i64>> = read_json(&p).unwrap();
        assert_eq!(v.as_ref().and_then(|m| m.get("a")), Some(&1));
        let _ = std::fs::remove_file(&p);
    }

    /// 无 BOM 路径不回归
    #[test]
    fn read_json_without_bom_still_works() {
        let p = tmp_path("plain");
        std::fs::write(&p, "{\"a\": 2}").unwrap();
        let v: Option<std::collections::HashMap<String, i64>> = read_json(&p).unwrap();
        assert_eq!(v.as_ref().and_then(|m| m.get("a")), Some(&2));
        let _ = std::fs::remove_file(&p);
    }

    /// 文件不存在 → None（既有语义不变）
    #[test]
    fn read_json_missing_returns_none() {
        let p = tmp_path("absent");
        let v: Option<std::collections::HashMap<String, i64>> = read_json(&p).unwrap();
        assert!(v.is_none());
    }
}
