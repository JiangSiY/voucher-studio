//! Tauri commands：对前端的全部 API（对应 v4.3 的 GUI → services 调用链）
//!
//! 约定：数据库快速操作直接在命令线程执行；PDF/OCR/备份等重活走
//! spawn_blocking，避免阻塞窗口线程。
//!
//! 子模块划分（v0.9 拆分，按领域分文件）：
//! - vouchers：凭证 CRUD / 批量操作 / 报表 / 搜索
//! - ingest：导入 / 附件 / OCR / 本地 OCR 状态
//! - export：PDF 导出与批次
//! - backup：备份与恢复（含 WebDAV）
//! - settings：配置 / 密钥 / 增强缓存管理

mod backup;
mod enhance;
mod export;
mod ingest;
mod links;
mod settings;
mod vouchers;

pub use backup::*;
pub use enhance::*;
pub use export::*;
pub use ingest::*;
pub use links::*;
pub use settings::*;
pub use vouchers::*;

pub(super) use std::collections::HashMap;
pub(super) use std::path::{Path, PathBuf};
pub(super) use std::sync::Arc;

pub(super) use serde::Serialize;
pub(super) use tauri::{AppHandle, State};

pub(super) use voucher_core::{CoreContext, CoreError};

pub(super) type Ctx<'a> = State<'a, Arc<CoreContext>>;

// ---------- 结构化错误（体检 #10 / A-4） ----------

/// 命令层结构化错误：{ code, message } 序列化给前端。
///
/// - `message`：保持与旧 `err_string` 一致的原文（中文可读文案），
///   前端直接展示不改味；
/// - `code`：CoreError 变体的稳定 kebab-code，前端可按类型差异化处理
///   （如 `lock_held` → 引导用户解锁/检查多实例，`db`/`io` → 提示重试）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    /// CoreError 变体 → 稳定 code；message 沿用 Display（与旧 err_string 完全一致）。
    pub(crate) fn from_core(e: CoreError) -> Self {
        let code = match &e {
            CoreError::Io(_) => "io",
            CoreError::Db(_) => "db",
            CoreError::Json(_) => "json",
            CoreError::Keyring(_) => "keyring",
            CoreError::Http(_) => "http",
            CoreError::Pdf(_) => "pdf",
            CoreError::Zip(_) => "zip",
            CoreError::Xlsx(_) => "xlsx",
            CoreError::NotImplemented(_) => "not_implemented",
            CoreError::LocalOcr(_) => "local_ocr",
            CoreError::Cv(_) => "cv",
            CoreError::NotFound(_) => "not_found",
            CoreError::Invalid(_) => "invalid",
            CoreError::LockHeld => "lock_held",
        };
        Self {
            code,
            message: e.to_string(),
        }
    }

    /// 手工业务错误：显式给稳定 code；message 原样透传（不加前缀）。
    pub(crate) fn plain(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// 非 CoreError（join 异常 / 窗口操作失败 / 字符串业务校验等）：统一归 `internal`。
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal",
            message: message.into(),
        }
    }
}

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        ApiError::from_core(e)
    }
}

/// 把 voucher-core Result（CoreError）转换为命令层 Result（ApiError）。
pub(super) fn err_api<T>(r: voucher_core::Result<T>) -> std::result::Result<T, ApiError> {
    r.map_err(ApiError::from_core)
}
// ---------- 应用信息 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub db_path: String,
    pub attachments_dir: String,
    pub backups_dir: String,
    pub exports_dir: String,
    pub logs_dir: String,
}

#[tauri::command]
pub fn get_app_info(ctx: Ctx) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        data_dir: ctx.paths.base.to_string_lossy().to_string(),
        db_path: ctx.paths.db_file().to_string_lossy().to_string(),
        attachments_dir: ctx.paths.attachments.to_string_lossy().to_string(),
        backups_dir: ctx.paths.backups.to_string_lossy().to_string(),
        exports_dir: ctx.paths.exports.to_string_lossy().to_string(),
        logs_dir: ctx.paths.logs.to_string_lossy().to_string(),
    }
}

#[tauri::command]
pub fn open_path(app: AppHandle, path: String) -> std::result::Result<(), ApiError> {
    crate::platform::open_path(&app, &path).map_err(ApiError::internal)
}

#[tauri::command]
pub fn restart_app(app: AppHandle, ctx: Ctx) {
    // 计划内重启（恢复备份生效流程）：先清会话 marker，避免下次启动误报「异常退出」
    let _ = voucher_core::services::recovery::mark_clean_exit(&ctx);
    app.restart();
}

// ---------- 崩溃恢复 ----------

/// 读取启动时的恢复检查报告（marker 检测与健康检查在 main.rs setup 中完成并缓存于此）
#[tauri::command]
pub fn get_recovery_report(
    state: tauri::State<'_, voucher_core::services::recovery::RecoveryReport>,
) -> voucher_core::services::recovery::RecoveryReport {
    state.inner().clone()
}
