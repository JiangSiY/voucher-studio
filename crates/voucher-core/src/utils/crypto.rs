//! 凭据存储（替代 v4.3 的 crypto.py）
//!
//! 定稿决策（2026-08-19）：不兼容旧 Fernet/DPAPI/Keychain 格式。
//! 敏感信息（WebDAV 密码、OCR API Key）直接存入操作系统原生凭据库：
//! - Windows：凭据管理器
//! - macOS：Keychain
//!
//! 密钥永不落盘为明文。

use keyring::Entry;

use crate::{CoreError, Result};

/// 已知凭据别名（前端密钥管理界面逐项展示）
pub const KNOWN_ALIASES: &[(&str, &str)] = &[
    ("ocr_api_key", "云端 OCR API Key"),
    ("webdav_password", "WebDAV 密码"),
    ("backup_password", "备份加密密码"),
];

/// 系统凭据库包装
pub struct SecretStore {
    service: String,
}

impl SecretStore {
    pub fn new(service: &str) -> Self {
        Self {
            service: service.to_string(),
        }
    }

    fn entry(&self, account: &str) -> Result<Entry> {
        Entry::new(&self.service, account).map_err(CoreError::Keyring)
    }

    /// 写入（覆盖）
    pub fn set(&self, account: &str, value: &str) -> Result<()> {
        self.entry(account)?
            .set_password(value)
            .map_err(CoreError::Keyring)
    }

    /// 读取；不存在返回 None
    pub fn get(&self, account: &str) -> Result<Option<String>> {
        match self.entry(account)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CoreError::Keyring(e)),
        }
    }

    /// 删除；返回是否确实删除了
    pub fn delete(&self, account: &str) -> Result<bool> {
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

/// 凭据状态（给前端密钥管理页）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretStatus {
    pub alias: String,
    pub label: String,
    pub exists: bool,
}
