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
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)?;
    let v = serde_json::from_str(&content).map_err(CoreError::Json)?;
    Ok(Some(v))
}
