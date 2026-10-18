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
/// ├── logs/                # 空壳（保留给 `paths.logs` 诊断字段；真实日志见下方说明）
/// └── temp/
/// ```
/// 真实日志位置：`~/Library/Logs/<identifier>/`（`tauri-plugin-log` 的 `TargetKind::LogDir`，
/// 由壳层 `src-tauri/src/main.rs` 配置），**不在**数据目录的 `logs/` 下——排查问题时别去那里找。
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
    ///
    /// # 契约
    /// `rel` 必须是本程序生成的可信相对路径（`attachment_rel_path` 产物，
    /// 形如 `2026/09/<md5>.jpg`）。**DB / 外部数据来源的 rel 必须走
    /// [`attachment_abs_checked`](Self::attachment_abs_checked)**
    /// （体检 Q-20：DB 被篡改 / 恢复出损坏数据时，rel 可能携带穿越片段）。
    pub fn attachment_abs(&self, rel: &str) -> PathBuf {
        self.attachments.join(rel)
    }

    /// 附件相对路径的词法安全校验：拒绝绝对路径 / `..` / 反斜杠 / 盘符。
    /// 纯词法判定（不依赖文件存在——canonicalize 对缺失文件失效，
    /// 而「文件不存在」恰是多个读路径的正常态）。
    pub fn is_safe_rel(rel: &str) -> bool {
        if rel.is_empty() || rel.contains('\\') || rel.contains(':') {
            return false;
        }
        let p = std::path::Path::new(rel);
        if p.is_absolute() {
            return false;
        }
        p.components().all(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    }

    /// DB / 外部数据来源的 rel → 绝对路径（带穿越防御，体检 Q-20 收口）。
    /// 非法 rel 报错而非拼出附件目录外的路径。
    pub fn attachment_abs_checked(&self, rel: &str) -> Result<PathBuf> {
        if Self::is_safe_rel(rel) {
            Ok(self.attachments.join(rel))
        } else {
            Err(CoreError::Invalid(format!(
                "非法的附件相对路径（疑似路径穿越）：{rel:?}"
            )))
        }
    }

    /// 把一张图片复制进附件目录（按 yyyy/mm + 内容哈希命名），返回相对路径
    pub fn attachment_rel_path(&self, hash: &str, ext: &str, year_month: &str) -> String {
        format!("{}/{}.{}", year_month, hash, ext)
    }

    /// rel → 绝对路径字符串（非法 rel 返回 None）。
    /// 供**界面下发**用：由 Rust 侧用 `PathBuf` 拼出平台原生分隔符，前端不要再自行拼接。
    /// 不判存在性——文件缺失交给前端的回退阶梯，否则「原图在、增强缺失」会连原图都不显示。
    pub fn abs_string_checked(&self, rel: &str) -> Option<String> {
        self.attachment_abs_checked(rel)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Q-20：词法校验——程序生成的正常 rel 通过，穿越/绝对/盘符/反斜杠全拒
    #[test]
    fn is_safe_rel_accepts_generated_and_rejects_traversal() {
        // 程序生成形态（attachment_rel_path 产物）
        assert!(AppPaths::is_safe_rel("2026/09/abc123.jpg"));
        assert!(AppPaths::is_safe_rel("2026/09/abc123.enhanced.jpg"));
        // 穿越与异常形态
        assert!(!AppPaths::is_safe_rel("../outside.txt"));
        assert!(!AppPaths::is_safe_rel("2026/../../outside.jpg"));
        assert!(!AppPaths::is_safe_rel("/etc/passwd"));
        assert!(!AppPaths::is_safe_rel("C:\\Windows\\evil.jpg"));
        assert!(!AppPaths::is_safe_rel("C:/Windows/evil.jpg"));
        assert!(!AppPaths::is_safe_rel("a\\b.jpg"), "反斜杠拒绝");
        assert!(!AppPaths::is_safe_rel(""));
    }

    #[test]
    fn attachment_abs_checked_rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::resolve(Some(dir.path().to_path_buf())).unwrap();
        assert!(paths.attachment_abs_checked("2026/09/a.jpg").is_ok());
        let err = paths
            .attachment_abs_checked("../secrets.txt")
            .expect_err("穿越 rel 应报错");
        assert!(err.to_string().contains("路径穿越"));
    }
}
