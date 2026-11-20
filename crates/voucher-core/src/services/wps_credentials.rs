//! WPS 专用凭据入口（S0-01～03 的代码边界）。
//!
//! 与 `SecretStore` 的关键差异：
//! - service 固定为 `voucher-studio-wps`，不复用 `VoucherStudio` 的通用凭据；
//! - **绝不**读取 `CENTER_SECRETS_FILE`，没有文件后端，钥匙串不可用即报错；
//! - 对外只暴露 `token_set` / `token_expires_at`，不提供读回明文的命令。

use std::sync::Arc;

use keyring::Entry;
use serde::Serialize;

use crate::services::wps::{WpsCredentialProvider, WpsError};
use crate::{CoreError, Result};

pub const WPS_KEYRING_SERVICE: &str = "voucher-studio-wps";
const TOKEN_ACCOUNT: &str = "access_token";
const EXPIRES_ACCOUNT: &str = "token_expires_at";

/// WPS 凭据状态（不包含 token 明文）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WpsTokenStatus {
    pub token_set: bool,
    pub token_expires_at: Option<String>,
}

trait WpsCredentialBackend: Send + Sync {
    fn set(&self, account: &str, value: &str) -> Result<()>;
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn delete(&self, account: &str) -> Result<bool>;
    #[cfg(test)]
    fn kind(&self) -> &'static str;
}

struct OsKeyring {
    service: String,
}

impl WpsCredentialBackend for OsKeyring {
    fn set(&self, account: &str, value: &str) -> Result<()> {
        Entry::new(&self.service, account)
            .map_err(unavailable)?
            .set_password(value)
            .map_err(unavailable)
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        match Entry::new(&self.service, account)
            .map_err(unavailable)?
            .get_password()
        {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(unavailable(e)),
        }
    }

    #[cfg(test)]
    fn kind(&self) -> &'static str {
        "keychain"
    }

    fn delete(&self, account: &str) -> Result<bool> {
        match Entry::new(&self.service, account)
            .map_err(unavailable)?
            .delete_credential()
        {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(unavailable(e)),
        }
    }
}

fn unavailable(e: keyring::Error) -> CoreError {
    CoreError::KeyringUnavailable(format!(
        "系统钥匙串不可用（{e}）；WPS 凭据禁止回落到文件后端"
    ))
}

/// 只走系统钥匙串的 WPS 凭据存储。
pub struct WpsCredentialStore {
    backend: Arc<dyn WpsCredentialBackend>,
}

impl Default for WpsCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

impl WpsCredentialProvider for WpsCredentialStore {
    /// 请求时取 token；只做认证，不把 token 暴露给命令层。
    ///
    /// 过期时间不可解析或已过期时拒绝请求；钥匙串不可用同样拒绝，
    /// 绝不回落到 `CENTER_SECRETS_FILE` 或文件后端。
    fn access_token(&self) -> std::result::Result<String, WpsError> {
        let token = self
            .backend
            .get(TOKEN_ACCOUNT)
            .map_err(|e| WpsError::Credentials(e.to_string()))?
            .ok_or_else(|| WpsError::Credentials("WPS token 未设置，拒绝请求".into()))?;
        if let Some(raw) = self
            .backend
            .get(EXPIRES_ACCOUNT)
            .map_err(|e| WpsError::Credentials(e.to_string()))?
        {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                let expires = chrono::DateTime::parse_from_rfc3339(trimmed)
                    .map_err(|_| WpsError::Credentials("WPS token 过期时间格式非法".into()))?;
                if expires <= chrono::Utc::now() {
                    return Err(WpsError::AuthExpired);
                }
            }
        }
        Ok(token)
    }
}

impl WpsCredentialStore {
    pub fn new() -> Self {
        Self {
            backend: Arc::new(OsKeyring {
                service: WPS_KEYRING_SERVICE.to_string(),
            }),
        }
    }

    /// 仅供测试：注入内存/失败后端，不改变生产构造路径。
    #[cfg(test)]
    fn with_backend(backend: Arc<dyn WpsCredentialBackend>) -> Self {
        Self { backend }
    }

    #[cfg(test)]
    fn backend_kind(&self) -> &'static str {
        self.backend.kind()
    }

    /// 写入 token（覆盖）。expiresAt 可空；不返回、不打印 token。
    pub fn set_token(&self, token: &str, expires_at: Option<&str>) -> Result<()> {
        if token.trim().is_empty() {
            return Err(CoreError::Invalid("WPS token 不能为空".into()));
        }
        self.backend.set(TOKEN_ACCOUNT, token)?;
        match expires_at.map(str::trim).filter(|v| !v.is_empty()) {
            Some(value) => self.backend.set(EXPIRES_ACCOUNT, value)?,
            None => {
                let _ = self.backend.delete(EXPIRES_ACCOUNT)?;
            }
        }
        Ok(())
    }

    /// 删除 token 与过期时间；返回是否确实删除了 token。
    pub fn delete_token(&self) -> Result<bool> {
        let removed = self.backend.delete(TOKEN_ACCOUNT)?;
        let _ = self.backend.delete(EXPIRES_ACCOUNT)?;
        Ok(removed)
    }

    /// 状态接口：只返回是否已设置与过期时间，不返回明文。
    pub fn status(&self) -> Result<WpsTokenStatus> {
        let token_set = self.backend.get(TOKEN_ACCOUNT)?.is_some();
        let expires = if token_set {
            self.backend
                .get(EXPIRES_ACCOUNT)?
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        } else {
            None
        };
        Ok(WpsTokenStatus {
            token_set,
            token_expires_at: expires,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct MemoryBackend {
        data: Mutex<BTreeMap<String, String>>,
    }

    impl WpsCredentialBackend for MemoryBackend {
        fn set(&self, account: &str, value: &str) -> Result<()> {
            self.data
                .lock()
                .map_err(|_| CoreError::Invalid("lock poisoned".into()))?
                .insert(account.to_string(), value.to_string());
            Ok(())
        }

        fn get(&self, account: &str) -> Result<Option<String>> {
            Ok(self
                .data
                .lock()
                .map_err(|_| CoreError::Invalid("lock poisoned".into()))?
                .get(account)
                .cloned())
        }

        fn delete(&self, account: &str) -> Result<bool> {
            Ok(self
                .data
                .lock()
                .map_err(|_| CoreError::Invalid("lock poisoned".into()))?
                .remove(account)
                .is_some())
        }

        fn kind(&self) -> &'static str {
            "memory"
        }
    }

    struct FailingBackend;

    impl WpsCredentialBackend for FailingBackend {
        fn set(&self, _: &str, _: &str) -> Result<()> {
            Err(CoreError::KeyringUnavailable("钥匙串不可用".into()))
        }
        fn get(&self, _: &str) -> Result<Option<String>> {
            Err(CoreError::KeyringUnavailable("钥匙串不可用".into()))
        }
        fn delete(&self, _: &str) -> Result<bool> {
            Err(CoreError::KeyringUnavailable("钥匙串不可用".into()))
        }

        fn kind(&self) -> &'static str {
            "failing"
        }
    }

    fn memory_store() -> WpsCredentialStore {
        WpsCredentialStore::with_backend(Arc::new(MemoryBackend::default()))
    }

    #[test]
    fn service_is_dedicated_and_does_not_use_generic_secret_store() {
        assert_eq!(WPS_KEYRING_SERVICE, "voucher-studio-wps");
        assert_eq!(WpsCredentialStore::new().backend_kind(), "keychain");
    }

    #[test]
    fn status_exposes_only_token_set_and_expiry_and_never_reads_file_backend() {
        let store = memory_store();
        assert_eq!(
            store.status().unwrap(),
            WpsTokenStatus {
                token_set: false,
                token_expires_at: None
            }
        );
        store
            .set_token(
                "synthetic-wps-token-do-not-echo",
                Some("2026-12-31T23:59:59+08:00"),
            )
            .unwrap();
        let status = store.status().unwrap();
        assert!(status.token_set);
        assert_eq!(
            status.token_expires_at.as_deref(),
            Some("2026-12-31T23:59:59+08:00")
        );
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("synthetic-wps-token-do-not-echo"));
    }

    #[test]
    fn delete_is_idempotent_and_clears_expiry() {
        let store = memory_store();
        store.set_token("synthetic", Some("2026-12-01")).unwrap();
        assert!(store.status().unwrap().token_set);
        assert!(store.delete_token().unwrap());
        assert_eq!(
            store.status().unwrap(),
            WpsTokenStatus {
                token_set: false,
                token_expires_at: None
            }
        );
        assert!(!store.delete_token().unwrap());
    }

    #[test]
    fn unavailable_keychain_errors_without_file_fallback() {
        let store = WpsCredentialStore::with_backend(Arc::new(FailingBackend));
        let err = store.status().unwrap_err().to_string();
        assert!(err.contains("钥匙串不可用"), "{err}");
        assert!(store.delete_token().is_err());
        assert!(store.set_token("x", None).is_err());
    }

    #[test]
    fn empty_token_is_rejected_before_keychain_write() {
        let store = memory_store();
        assert!(store.set_token("  ", None).is_err());
        assert!(!store.status().unwrap().token_set);
    }

    #[test]
    fn access_token_is_available_only_for_unexpired_synthetic_token() {
        let store = memory_store();
        assert!(matches!(
            store.access_token(),
            Err(WpsError::Credentials(_))
        ));
        store
            .set_token("synthetic-token", Some("2099-12-31T23:59:59+08:00"))
            .unwrap();
        assert_eq!(store.access_token().unwrap(), "synthetic-token");
        store
            .set_token("expired-token", Some("2020-01-01T00:00:00+08:00"))
            .unwrap();
        assert!(matches!(store.access_token(), Err(WpsError::AuthExpired)));
        store
            .set_token("bad-expiry-token", Some("not-a-date"))
            .unwrap();
        assert!(matches!(
            store.access_token(),
            Err(WpsError::Credentials(_))
        ));
    }

    #[test]
    fn unavailable_keychain_cannot_yield_a_token() {
        let store = WpsCredentialStore::with_backend(Arc::new(FailingBackend));
        assert!(matches!(
            store.access_token(),
            Err(WpsError::Credentials(_))
        ));
    }

    #[test]
    fn center_secrets_file_env_does_not_change_backend_kind() {
        let path = std::env::temp_dir().join(format!(
            "voucher-wps-secrets-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, r#"{"access_token":"file-should-not-be-read"}"#).unwrap();
        std::env::set_var("CENTER_SECRETS_FILE", &path);
        let store = WpsCredentialStore::new();
        assert_eq!(store.backend_kind(), "keychain");
        std::env::remove_var("CENTER_SECRETS_FILE");
        let _ = std::fs::remove_file(path);
    }
}
