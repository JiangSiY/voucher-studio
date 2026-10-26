//! Voucher Studio 业务核心
//!
//! 不依赖 Tauri，可独立测试（对应 v4.3 的 `voucher/` 业务核心）。

pub mod db;
pub mod models;
pub mod services;
pub mod utils;

use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

pub use db::Database;
pub use models::OcrHealth;
pub use utils::config::{
    AppConfig, SECRET_BACKUP_PASSWORD, SECRET_OCR_API_KEY, SECRET_WEBDAV_PASSWORD,
};
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
    // printpdf 0.12 不再提供统一的 `Error` 类型（0.7 → 0.12.8 迁移时改为字符串载荷，
    // 保留变体名以免上层 match 分支变动；当前生成路径不再产生 printpdf 错误）
    #[error("PDF 错误: {0}")]
    Pdf(String),
    #[error("压缩包错误: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("Excel 错误: {0}")]
    Xlsx(#[from] rust_xlsxwriter::XlsxError),
    #[error("未实现: {0}")]
    NotImplemented(&'static str),
    #[error("本地 OCR: {0}")]
    LocalOcr(String),
    #[error("OpenCV 错误: {0}")]
    Cv(#[from] opencv::Error),
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
    /// 最近一次 OCR 的实际引擎与云端失败原因（设置页展示；进程内状态，不落盘）
    ocr_health: RwLock<OcrHealth>,
    /// 备份互斥：手动/定时/关闭前备份可能并发触发，
    /// VACUUM INTO 写同一固定 temp 路径 + zip 时间戳秒级可碰撞——串行化整个备份流程
    backup_lock: Mutex<()>,
}

impl CoreContext {
    /// 初始化上下文：解析目录、打开数据库、建表、加载配置。
    /// `base_dir` 为 None 时使用系统默认数据目录（macOS：`~/Library/Application Support/VoucherStudio`）。
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
            ocr_health: RwLock::new(OcrHealth::default()),
            backup_lock: Mutex::new(()),
            paths,
        })
    }

    /// 读取当前配置（克隆）。
    ///
    /// # 注意
    /// 锁中毒会自动恢复（对齐 [`CoreContext::backup_guard`] 的写法）；std `RwLock` 写锁非重入，
    /// 持锁期间不得回调会再次取同一把锁的逻辑。
    pub fn config(&self) -> AppConfig {
        self.read_config().clone()
    }

    /// 保存配置（写盘 + 更新内存）。
    ///
    /// # 注意
    /// 持写锁期间不得回调会再次取 `config` 锁的逻辑。
    pub fn set_config(&self, mut cfg: AppConfig) -> Result<()> {
        cfg.normalize();
        cfg.save(&self.paths.config_file())?;
        *self.write_config() = cfg;
        Ok(())
    }

    /// 备份流程互斥（手动/定时/关闭前共享；抢不到锁 = 已有备份在跑，本次直接排队等）
    pub fn backup_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.backup_lock.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// **非阻塞**取备份互斥：拿不到返回 `None`。
    ///
    /// 给「关闭前备份」「退出前备份」这类不能让用户干等的路径用——备份可能正持锁做
    /// 多轮 WebDAV 上传（单次超时上限 30 分钟），阻塞等待会让窗口**关不掉**且毫无提示
    /// （2026-09-23 审计 B13）。数据不会因此丢：备份是增量的，调度器与下次启动都会再备份。
    pub fn try_backup_guard(&self) -> Option<std::sync::MutexGuard<'_, ()>> {
        self.backup_lock.try_lock().ok()
    }

    /// 最近一次 OCR 状态（设置页展示：实际走的云端还是本地、云端为何没用上）
    pub fn ocr_health(&self) -> OcrHealth {
        self.read_ocr_health().clone()
    }

    /// 记录一次 OCR 结果。
    /// `engine` 是**实际产出结果**的引擎；`cloud_error` 仅当云端尝试失败时传入
    /// （云端成功传 None 会清空上一条失败原因，避免设置页一直挂着过期的红字）。
    pub fn record_ocr_run(&self, engine: &str, cloud_error: Option<&str>) {
        let mut guard = self.write_ocr_health();
        guard.last_engine = Some(engine.to_string());
        guard.last_cloud_error = cloud_error.map(|s| s.to_string());
        guard.last_at = Some(chrono::Utc::now().to_rfc3339());
    }

    // ---------- 锁中毒恢复辅助（体检 Q-2：中毒=上一持锁线程 panic 过，不静默恢复） ----------

    fn read_config(&self) -> std::sync::RwLockReadGuard<'_, AppConfig> {
        self.config.read().unwrap_or_else(|poisoned| {
            log::error!("配置读锁中毒：上一持锁线程 panic，已恢复继续使用（配置可能不完整）");
            poisoned.into_inner()
        })
    }

    fn write_config(&self) -> std::sync::RwLockWriteGuard<'_, AppConfig> {
        self.config.write().unwrap_or_else(|poisoned| {
            log::error!("配置写锁中毒：上一持锁线程 panic，已恢复继续使用（配置可能不完整）");
            poisoned.into_inner()
        })
    }

    fn read_ocr_health(&self) -> std::sync::RwLockReadGuard<'_, OcrHealth> {
        self.ocr_health.read().unwrap_or_else(|poisoned| {
            log::error!("OCR 状态锁中毒：上一持锁线程 panic，已恢复继续使用");
            poisoned.into_inner()
        })
    }

    fn write_ocr_health(&self) -> std::sync::RwLockWriteGuard<'_, OcrHealth> {
        self.ocr_health.write().unwrap_or_else(|poisoned| {
            log::error!("OCR 状态锁中毒：上一持锁线程 panic，已恢复继续使用");
            poisoned.into_inner()
        })
    }
}
