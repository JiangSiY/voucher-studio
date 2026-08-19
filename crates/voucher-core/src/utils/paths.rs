//! 目录与路径（对应 v4.3 的 `paths.py`）

use std::path::{Path, PathBuf};

use crate::{CoreError, Result};

use directories::ProjectDirs;

/// 应用目录布局（base 即系统数据目录：Windows %APPDATA%\VoucherStudio\data）
///
/// ```text
/// <base>/
/// ├── voucher.db             # SQLite 数据库
/// ├── .lock                  # 文件锁（防多实例并发写）
/// ├── attachments/yyyy/mm/   # 附件图片按月归档
/// ├── backups/               # 备份 zip
/// ├── exports/               # 导出 PDF
/// ├── models/                # 本地 OCR 模型（首次启动从安装源复制）
/// ├── config/app.json        # 用户配置（不含密钥）
/// ├── logs/
/// └── temp/
/// ```
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub base: PathBuf,
    pub attachments: PathBuf,
    pub backups: PathBuf,
    pub exports: PathBuf,
    pub models: PathBuf,
    pub config_dir: PathBuf,
    pub logs: PathBuf,
    pub temp: PathBuf,
}

impl AppPaths {
    /// base 为 None 时使用系统数据目录
    pub fn resolve(base: Option<PathBuf>) -> Result<Self> {
        let base = match base {
            Some(b) => b,
            None => ProjectDirs::from("", "", "VoucherStudio")
                .map(|d| d.data_dir().to_path_buf())
                .ok_or_else(|| CoreError::Invalid("无法确定系统数据目录".into()))?,
        };
        Ok(Self {
            attachments: base.join("attachments"),
            backups: base.join("backups"),
            exports: base.join("exports"),
            models: base.join("models"),
            config_dir: base.join("config"),
            logs: base.join("logs"),
            temp: base.join("temp"),
            base,
        })
    }

    /// 创建全部目录（已存在则跳过）
    pub fn ensure_all(&self) -> Result<()> {
        for dir in [
            &self.base,
            &self.attachments,
            &self.backups,
            &self.exports,
            &self.models,
            &self.config_dir,
            &self.logs,
            &self.temp,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn db_file(&self) -> PathBuf {
        self.base.join("voucher.db")
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("app.json")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.base.join(".lock")
    }

    /// 附件的磁盘绝对路径
    pub fn attachment_abs(&self, rel: &str) -> PathBuf {
        self.attachments.join(rel)
    }

    /// 把一张图片复制进附件目录（按 yyyy/mm + 内容哈希命名），返回相对路径
    pub fn attachment_rel_path(&self, hash: &str, ext: &str, year_month: &str) -> String {
        format!("{}/{}.{}", year_month, hash, ext)
    }

    pub fn attachment_dir_for(&self, year_month: &str) -> PathBuf {
        self.attachments.join(year_month)
    }

    /// 校验路径位于附件根目录之内（防目录穿越）
    pub fn is_inside_attachments(&self, path: &Path) -> bool {
        let canon_root = self
            .attachments
            .canonicalize()
            .unwrap_or(self.attachments.clone());
        match path.canonicalize() {
            Ok(p) => p.starts_with(&canon_root),
            Err(_) => false,
        }
    }

    /// 校验路径位于备份目录之内
    pub fn is_inside_backups(&self, path: &Path) -> bool {
        let canon_root = self.backups.canonicalize().unwrap_or(self.backups.clone());
        match path.canonicalize() {
            Ok(p) => p.starts_with(&canon_root),
            Err(_) => false,
        }
    }
}

/// 当前本地时间 "YYYY-MM"
pub fn current_year_month() -> String {
    chrono::Local::now().format("%Y/%m").to_string()
}

/// 当前本地日期 "YYYY-MM-DD"
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// 当前 UTC 时间戳（ISO 8601，数据库统一用它）
pub fn now_iso() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}
