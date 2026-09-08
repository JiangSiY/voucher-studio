//! Voucher Studio 业务核心
//!
//! 不依赖 Tauri，可独立测试（对应 v4.3 的 `voucher/` 业务核心）。

pub mod db;
pub mod models;
pub mod services;
pub mod utils;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

pub use db::Database;
pub use services::ocr_local::LocalOcr;
pub use utils::config::{AppConfig, SECRET_OCR_API_KEY, SECRET_WEBDAV_PASSWORD};
pub use utils::crypto::SecretStore;
pub use utils::paths::AppPaths;

/// 全局错误类型
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("数据库错误: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("凭据库错误: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("网络错误: {0}")]
    Http(#[from] reqwest::Error),
    #[error("PDF 错误: {0}")]
    Pdf(#[from] printpdf::Error),
    #[error("压缩包错误: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("Excel 错误: {0}")]
    Xlsx(#[from] rust_xlsxwriter::XlsxError),
    #[error("未实现: {0}")]
    NotImplemented(&'static str),
    #[error("本地 OCR: {0}")]
    LocalOcr(String),
    #[error("数据未找到: {0}")]
    NotFound(String),
    #[error("无效操作: {0}")]
    Invalid(String),
    #[error("数据目录已被其他实例锁定")]
    LockHeld,
}

pub type Result<T> = std::result::Result<T, CoreError>;

/// 应用上下文：贯穿各服务层的共享状态
pub struct CoreContext {
    pub paths: AppPaths,
    pub db: Database,
    pub config: RwLock<AppConfig>,
    pub secrets: SecretStore,
    /// 本地 OCR 引擎（首次使用时加载；Session::run 需 &mut，用 Mutex 串行化）
    local_ocr: RwLock<Option<Arc<Mutex<LocalOcr>>>>,
    /// 备份互斥：手动/定时/关闭前备份可能并发触发，
    /// VACUUM INTO 写同一固定 temp 路径 + zip 时间戳秒级可碰撞——串行化整个备份流程
    backup_lock: Mutex<()>,
}

impl CoreContext {
    /// 初始化上下文：解析目录、打开数据库、建表、加载配置。
    /// `base_dir` 为 None 时使用系统默认数据目录（Windows: %APPDATA%\VoucherStudio）。
    pub fn init(base_dir: Option<PathBuf>) -> Result<Self> {
        let paths = AppPaths::resolve(base_dir)?;
        paths.ensure_all()?;

        let conn = db::connection::open(&paths.db_file())?;
        db::schema::init(&conn)?;

        let config = AppConfig::load(&paths.config_file())?;

        Ok(Self {
            db: Database::new(conn),
            config: RwLock::new(config),
            secrets: SecretStore::new("VoucherStudio"),
            local_ocr: RwLock::new(None),
            backup_lock: Mutex::new(()),
            paths,
        })
    }

    /// 读取当前配置（克隆）
    pub fn config(&self) -> AppConfig {
        self.config.read().unwrap().clone()
    }

    /// 保存配置（写盘 + 更新内存）
    pub fn set_config(&self, cfg: AppConfig) -> Result<()> {
        cfg.save(&self.paths.config_file())?;
        *self.config.write().unwrap() = cfg;
        Ok(())
    }

    /// 备份流程互斥（手动/定时/关闭前共享；抢不到锁 = 已有备份在跑，本次直接排队等）
    pub fn backup_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.backup_lock.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 本地 OCR 引擎（懒加载 + 共享）。模型未安装时报错并提示处理方式。
    pub fn local_ocr(&self) -> Result<Arc<Mutex<LocalOcr>>> {
        if let Some(e) = self.local_ocr.read().unwrap().clone() {
            return Ok(e);
        }
        if !LocalOcr::is_installed(&self.paths.models) {
            return Err(CoreError::LocalOcr(format!(
                "本地 OCR 模型未安装（期望位于 {}）",
                self.paths.models.display()
            )));
        }
        let engine = Arc::new(Mutex::new(LocalOcr::load(&self.paths.models)?));
        *self.local_ocr.write().unwrap() = Some(engine.clone());
        Ok(engine)
    }

    /// 本地 OCR 是否可用（模型已安装）
    pub fn local_ocr_ready(&self) -> bool {
        LocalOcr::is_installed(&self.paths.models)
    }
}
