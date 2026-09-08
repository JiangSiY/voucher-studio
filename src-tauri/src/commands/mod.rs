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
mod settings;
mod vouchers;

pub use backup::*;
pub use enhance::*;
pub use export::*;
pub use ingest::*;
pub use settings::*;
pub use vouchers::*;

pub(super) use std::collections::HashMap;
pub(super) use std::path::{Path, PathBuf};
pub(super) use std::sync::Arc;

pub(super) use serde::Serialize;
pub(super) use tauri::{AppHandle, State};

pub(super) use voucher_core::{CoreContext, Result};

pub(super) type Ctx<'a> = State<'a, Arc<CoreContext>>;

pub(super) fn err_string<T>(r: Result<T>) -> std::result::Result<T, String> {
    r.map_err(|e| e.to_string())
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
pub fn open_path(app: AppHandle, path: String) -> std::result::Result<(), String> {
    crate::platform::open_path(&app, &path)
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
