//! 凭据存储（替代 v4.3 的 crypto.py）
//!
//! 定稿决策（2026-08-19）：不兼容旧 Fernet/DPAPI/Keychain 格式。
//! 敏感信息（WebDAV 密码、OCR API Key）直接存入操作系统原生凭据库：
//! - Windows：凭据管理器
//! - macOS：Keychain
//!
//! 密钥永不落盘为明文。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use keyring::Entry;

use crate::utils::jsonio;
use crate::{CoreError, Result};

/// 已知凭据别名（前端密钥管理界面逐项展示）
pub const KNOWN_ALIASES: &[(&str, &str)] = &[
    ("ocr_api_key", "云端 OCR API Key"),
    ("webdav_password", "WebDAV 密码"),
    ("backup_password", "备份加密密码"),
];

/// 凭据存储：默认走系统凭据库；服务端（无 GUI 的后台进程）可用文件后端。
///
/// 为什么要文件后端：macOS Keychain 在**没有图形会话**的进程里读取会阻塞等待授权，
/// launchd 常驻的 center-server 一旦调 `list_secrets` 就整个卡死（2026-11-18 实测 60s+ 无返回）。
/// 服务端由 `CENTER_SECRETS_FILE` 显式打开文件后端，文件权限 0600；
/// 桌面版不设该变量，行为与以前完全一致（仍走 Keychain，密钥不落明文）。
pub struct SecretStore {
    service: String,
    file: Option<PathBuf>,
}

impl SecretStore {
    pub fn new(service: &str) -> Self {
        Self {
            service: service.to_string(),
            file: env_secrets_file(),
        }
    }

    /// 显式指定文件后端（测试与 center-server 用；不读环境变量）
    pub fn with_file(service: &str, file: PathBuf) -> Self {
        Self {
            service: service.to_string(),
            file: Some(file),
        }
    }

    fn entry(&self, account: &str) -> Result<Entry> {
        Entry::new(&self.service, account).map_err(CoreError::Keyring)
    }

    /// 写入（覆盖）
    pub fn set(&self, account: &str, value: &str) -> Result<()> {
        if let Some(path) = &self.file {
            let mut map = load_file(path)?;
            map.insert(account.to_string(), value.to_string());
            return save_file(path, &map);
        }
        self.entry(account)?
            .set_password(value)
            .map_err(CoreError::Keyring)
    }

    /// 读取；不存在返回 None
    pub fn get(&self, account: &str) -> Result<Option<String>> {
        if let Some(path) = &self.file {
            return Ok(load_file(path)?.get(account).cloned());
        }
        match self.entry(account)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CoreError::Keyring(e)),
        }
    }

    /// 删除；返回是否确实删除了
    pub fn delete(&self, account: &str) -> Result<bool> {
        if let Some(path) = &self.file {
            let mut map = load_file(path)?;
            let removed = map.remove(account).is_some();
            if removed {
                save_file(path, &map)?;
            }
            return Ok(removed);
        }
        match self.entry(account)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(CoreError::Keyring(e)),
        }
    }

    /// 探测已知凭据的存在性（不返回明文）
    pub fn inspect(&self) -> Vec<SecretStatus> {
        KNOWN_ALIASES
            .iter()
            .map(|(alias, label)| SecretStatus {
                alias: alias.to_string(),
                label: label.to_string(),
                exists: matches!(self.get(alias), Ok(Some(_))),
            })
            .collect()
    }
}

fn env_secrets_file() -> Option<PathBuf> {
    std::env::var("CENTER_SECRETS_FILE")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
}

fn load_file(path: &Path) -> Result<BTreeMap<String, String>> {
    Ok(jsonio::read_json::<BTreeMap<String, String>>(path)?.unwrap_or_default())
}

fn save_file(path: &Path, map: &BTreeMap<String, String>) -> Result<()> {
    jsonio::write_json_atomic(path, map)?;
    set_private_perms(path)
}

#[cfg(unix)]
fn set_private_perms(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_perms(_path: &Path) -> Result<()> {
    Ok(())
}

/// 凭据状态（给前端密钥管理页）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretStatus {
    pub alias: String,
    pub label: String,
    pub exists: bool,
}
