//! 集成测试共享辅助：临时 CoreContext、测试图片生成器、计数器。
//! （各测试文件按需取用，不要求全部使用）

#![allow(dead_code)]

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Once};

use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi, CredentialPersistence};
use keyring::{Error as KeyringError, Result as KeyringResult};

use voucher_core::{CoreContext, CoreError, Result};

// ============================ 内存凭据后端 ============================

/// 进程内共享的凭据表：(service, account) → secret 字节
type MemoryStore = Arc<Mutex<HashMap<(String, String), Vec<u8>>>>;

/// 内存凭据，对应一个 keyring `Entry`
#[derive(Debug)]
struct MemoryCredential {
    store: MemoryStore,
    service: String,
    account: String,
}

impl MemoryCredential {
    /// 取共享表；中毒自动恢复（对齐本仓「锁中毒恢复」的既有约定）
    fn table(&self) -> MutexGuard<'_, HashMap<(String, String), Vec<u8>>> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn key(&self) -> (String, String) {
        (self.service.clone(), self.account.clone())
    }
}

impl CredentialApi for MemoryCredential {
    fn set_secret(&self, secret: &[u8]) -> KeyringResult<()> {
        let key = self.key();
        self.table().insert(key, secret.to_vec());
        Ok(())
    }

    fn get_secret(&self) -> KeyringResult<Vec<u8>> {
        let key = self.key();
        self.table().get(&key).cloned().ok_or(KeyringError::NoEntry)
    }

    fn delete_credential(&self) -> KeyringResult<()> {
        let key = self.key();
        // 首次删除 Ok、重复删除 NoEntry（与 keyring 各原生后端一致，SecretStore::delete 依赖此语义）
        if self.table().remove(&key).is_some() {
            Ok(())
        } else {
            Err(KeyringError::NoEntry)
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// 内存凭据库构造器。持有共享表，故同一进程内**所有** Entry 看到同一份数据。
///
/// 这正是与 keyring 自带 `mock` 的关键差异：那个 `MockCredentialBuilder` 是无字段
/// 单元结构体，`build()` 每次新建一份独立 `MockData`，于是「往一个 Entry 写入、
/// 从另一个 Entry 读取」永远读不到（get 恒为 NoEntry）。
#[derive(Debug)]
struct MemoryCredentialBuilder {
    store: MemoryStore,
}

impl CredentialBuilderApi for MemoryCredentialBuilder {
    fn build(
        &self,
        _target: Option<&str>,
        service: &str,
        user: &str,
    ) -> KeyringResult<Box<Credential>> {
        Ok(Box::new(MemoryCredential {
            store: Arc::clone(&self.store),
            service: service.to_string(),
            account: user.to_string(),
        }))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// 数据只活在进程内存中，进程退出即消失（如实声明，不谎报 UntilDelete）
    fn persistence(&self) -> CredentialPersistence {
        CredentialPersistence::ProcessOnly
    }
}

static INSTALL: Once = Once::new();

/// 全进程装一次：把 keyring 的默认凭据后端换成上方的共享内存实现。
///
/// 为什么需要：本仓 `keyring` 依赖只启用 `windows-native` / `apple-native`，
/// 在 Linux（CI runner）未启用任何后端。keyring 3.x 遇到「目标平台无后端」时会
/// **静默**回退到自带 mock，而该 mock 的 Entry 之间互不可见——于是「写入备份密码 →
/// 另一处读取」的链路在 Linux 上必然失败（`encrypted_backup_roundtrip`、
/// `webdav_mock` 全套都因此红），且失败表现为业务错误码，容易误判为产品缺陷。
///
/// 附带收益：测试不再读写真实系统凭据库（Windows 凭据管理器 / macOS Keychain），
/// 避免测试密码（如 `test-enc-pwd`）残留在用户真实凭据里——生产代码读取备份密码
/// 用的正是同一个服务名 `VoucherStudio`。
///
/// 必须在创建任何 Entry 之前调用（`build_default_credential` 会先查全局 builder，
/// 未装才回退到平台默认），故由 [`test_ctx`] 在开库前统一调用。
pub fn install_memory_keyring() {
    INSTALL.call_once(|| {
        keyring::set_default_credential_builder(Box::new(MemoryCredentialBuilder {
            store: Arc::new(Mutex::new(HashMap::new())),
        }));
    });
}

// ============================ 上下文与素材 ============================

pub fn test_ctx() -> (tempfile::TempDir, CoreContext) {
    install_memory_keyring();
    let dir = tempfile::tempdir().expect("tempdir");
    let ctx = CoreContext::init(Some(dir.path().to_path_buf())).expect("init");
    (dir, ctx)
}

pub fn make_png(path: &std::path::Path) {
    let img = image::RgbImage::from_pixel(64, 48, image::Rgb([180u8, 60, 60]));
    img.save(path).expect("save png");
}

/// 不同内容的图片生成器（同批每张内容必须不同，否则会被 MD5 去重）
pub fn make_png_variant(path: &std::path::Path, shade: u8) {
    let img =
        image::RgbImage::from_pixel(64, 48, image::Rgb([shade, 60, 180u8.saturating_sub(shade)]));
    img.save(path).expect("save png");
}

pub fn voucher_count(ctx: &CoreContext) -> Result<i64> {
    ctx.db.with_conn(|conn| {
        conn.query_row("SELECT COUNT(*) FROM vouchers", [], |r| r.get(0))
            .map_err(CoreError::from)
    })
}
