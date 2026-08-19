//! Tauri commands：对前端的全部 API（对应 v4.3 的 GUI → services 调用链）
//!
//! 约定：数据库快速操作直接在命令线程执行；PDF/OCR/备份等重活走
//! spawn_blocking，避免阻塞窗口线程。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::AppHandle;
use tauri::State;

use voucher_core::db::repos::voucher_repo::VoucherQuery;
use voucher_core::models::{NewVoucher, VoucherStatus, VoucherUpdate};
use voucher_core::services::export::ExportOptions;
use voucher_core::services::search::SearchHit;
use voucher_core::services::{backup, export, ingestion, naming, ocr, search, voucher_mgr};
use voucher_core::utils::config::{AppConfig, SECRET_OCR_API_KEY, SECRET_WEBDAV_PASSWORD};
use voucher_core::{CoreContext, Result};

type Ctx<'a> = State<'a, Arc<CoreContext>>;

fn err_string<T>(r: Result<T>) -> std::result::Result<T, String> {
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
pub fn restart_app(app: AppHandle) {
    app.restart();
}

// ---------- 配置 ----------

#[tauri::command]
pub fn get_config(ctx: Ctx) -> AppConfig {
    ctx.config()
}

#[tauri::command]
pub fn save_config(ctx: Ctx, config: AppConfig) -> std::result::Result<AppConfig, String> {
    // 命名模板先校验再落盘
    let validated =
        naming::NamingEngine::validate(&config.naming.template).map_err(|e| e.to_string())?;
    let mut config = config;
    config.naming.template = validated;
    err_string(ctx.set_config(config))?;
    Ok(ctx.config())
}

#[tauri::command]
pub fn validate_naming_template(template: String) -> std::result::Result<String, String> {
    naming::NamingEngine::validate(&template).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn preview_voucher_no(ctx: Ctx, date: String) -> std::result::Result<String, String> {
    let engine = naming::NamingEngine::new(&ctx.config().naming.template);
    let d = if date.is_empty() {
        voucher_core::utils::paths::today()
    } else {
        date
    };
    err_string(ctx.db.with_conn(|conn| engine.next_no(conn, &d)))
}

// ---------- 密钥管理（系统凭据库） ----------

#[tauri::command]
pub fn list_secrets(ctx: Ctx) -> Vec<voucher_core::utils::crypto::SecretStatus> {
    ctx.secrets.inspect()
}

#[tauri::command]
pub fn set_secret(ctx: Ctx, alias: String, value: String) -> std::result::Result<(), String> {
    let known = [SECRET_OCR_API_KEY, SECRET_WEBDAV_PASSWORD];
    if !known.contains(&alias.as_str()) {
        return Err(format!("未知凭据别名: {alias}"));
    }
    err_string(ctx.secrets.set(&alias, &value))
}

#[tauri::command]
pub fn delete_secret(ctx: Ctx, alias: String) -> std::result::Result<bool, String> {
    err_string(ctx.secrets.delete(&alias))
}

// ---------- 凭证 ----------

#[tauri::command]
pub fn list_vouchers(
    ctx: Ctx,
    query: VoucherQuery,
) -> std::result::Result<VoucherListResult, String> {
    let (items, total) = err_string(voucher_mgr::list(&ctx, &query))?;
    Ok(VoucherListResult { items, total })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherListResult {
    pub items: Vec<voucher_core::models::Voucher>,
    pub total: u64,
}

#[tauri::command]
pub fn get_voucher(ctx: Ctx, id: i64) -> std::result::Result<voucher_mgr::VoucherDetail, String> {
    err_string(voucher_mgr::get_detail(&ctx, id))
}

#[tauri::command]
pub fn create_voucher(
    ctx: Ctx,
    new_voucher: NewVoucher,
) -> std::result::Result<voucher_core::models::Voucher, String> {
    err_string(voucher_mgr::create_voucher(&ctx, new_voucher))
}

#[tauri::command]
pub fn update_voucher(
    ctx: Ctx,
    id: i64,
    update: VoucherUpdate,
) -> std::result::Result<voucher_core::models::Voucher, String> {
    err_string(voucher_mgr::update_voucher(&ctx, id, update))
}

#[tauri::command]
pub fn delete_voucher(ctx: Ctx, id: i64) -> std::result::Result<(), String> {
    err_string(voucher_mgr::delete_voucher(&ctx, id))
}

#[tauri::command]
pub fn set_voucher_status(ctx: Ctx, id: i64, status: String) -> std::result::Result<(), String> {
    let s = VoucherStatus::parse(&status).ok_or_else(|| format!("未知状态: {status}"))?;
    err_string(voucher_mgr::set_status(&ctx, id, s))
}

#[tauri::command]
pub fn voucher_stats(
    ctx: Ctx,
) -> std::result::Result<voucher_core::db::repos::voucher_repo::VoucherStats, String> {
    err_string(voucher_mgr::stats(&ctx))
}

// ---------- 搜索 ----------

#[tauri::command]
pub fn search_vouchers(
    ctx: Ctx,
    keyword: String,
    limit: Option<u32>,
) -> std::result::Result<Vec<SearchHit>, String> {
    err_string(search::search_vouchers(&ctx, &keyword, limit.unwrap_or(50)))
}

// ---------- 导入 / 附件 / OCR ----------

#[tauri::command]
pub async fn import_images(
    ctx: Ctx<'_>,
    paths: Vec<String>,
    voucher_id: Option<i64>,
    run_ocr: Option<bool>,
) -> std::result::Result<ingestion::ImportSummary, String> {
    let files: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        err_string(ingestion::import_images(
            &core,
            &files,
            voucher_id,
            run_ocr.unwrap_or(false),
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn delete_attachment(ctx: Ctx, id: i64) -> std::result::Result<(), String> {
    err_string(ingestion::delete_attachment(&ctx, id))
}

#[tauri::command]
pub fn attachment_abs_path(ctx: Ctx, id: i64) -> std::result::Result<String, String> {
    let att = err_string(
        ctx.db
            .with_conn(|conn| voucher_core::db::repos::attachment_repo::get(conn, id)),
    )?;
    match att {
        Some(a) => Ok(ctx
            .paths
            .attachment_abs(&a.file_path)
            .to_string_lossy()
            .to_string()),
        None => Err(format!("附件 {id} 不存在")),
    }
}

#[tauri::command]
pub async fn run_ocr(
    ctx: Ctx<'_>,
    attachment_id: i64,
) -> std::result::Result<voucher_core::models::Attachment, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        err_string(ocr::run_for_attachment(&core, attachment_id))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn run_ocr_pending(
    ctx: Ctx<'_>,
    limit: Option<u32>,
) -> std::result::Result<usize, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        err_string(ocr::run_pending(&core, limit.unwrap_or(50)))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------- 本地 OCR 状态 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOcrModelFile {
    pub name: String,
    pub size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOcrStatus {
    pub installed: bool,
    pub models_dir: String,
    pub files: Vec<LocalOcrModelFile>,
    pub total_size: u64,
}

#[tauri::command]
pub fn local_ocr_status(ctx: Ctx) -> LocalOcrStatus {
    let dir = &ctx.paths.models;
    let mut files = Vec::new();
    let mut total = 0u64;
    for name in voucher_core::services::ocr_local::MODEL_FILES {
        if let Ok(meta) = std::fs::metadata(dir.join(name)) {
            total += meta.len();
            files.push(LocalOcrModelFile {
                name: name.to_string(),
                size: meta.len(),
            });
        }
    }
    LocalOcrStatus {
        installed: voucher_core::LocalOcr::is_installed(dir),
        models_dir: dir.to_string_lossy().to_string(),
        files,
        total_size: total,
    }
}

// ---------- 导出（PDF） ----------

#[tauri::command]
pub async fn export_pdf(
    ctx: Ctx<'_>,
    voucher_ids: Vec<i64>,
    options: Option<ExportOptions>,
) -> std::result::Result<voucher_core::models::ExportBatch, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || match options {
        Some(o) => err_string(export::export_vouchers_pdf(&core, &voucher_ids, &o)),
        None => err_string(export::export_with_defaults(&core, &voucher_ids)),
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn list_export_batches(
    ctx: Ctx,
    limit: Option<u32>,
) -> std::result::Result<Vec<voucher_core::models::ExportBatch>, String> {
    err_string(export::list_batches(&ctx, limit.unwrap_or(50)))
}

// ---------- 备份与恢复 ----------

#[tauri::command]
pub async fn run_backup(ctx: Ctx<'_>) -> std::result::Result<backup::BackupInfo, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || err_string(backup::run_backup(&core)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn list_backups(ctx: Ctx) -> std::result::Result<Vec<backup::BackupInfo>, String> {
    err_string(backup::list_backups(&ctx))
}

#[tauri::command]
pub fn inspect_backup(path: String) -> std::result::Result<backup::RestoreSummary, String> {
    err_string(backup::inspect_backup(Path::new(&path)))
}

#[tauri::command]
pub fn restore_backup(
    ctx: Ctx,
    path: String,
) -> std::result::Result<backup::RestoreSummary, String> {
    // 恢复走「待恢复文件 + 重启生效」流程，避免在运行中替换数据库
    crate::platform::stage_restore(&ctx, Path::new(&path))
}
