//! Tauri commands：对前端的全部 API（对应 v4.3 的 GUI → services 调用链）
//!
//! 约定：数据库快速操作直接在命令线程执行；PDF/OCR/备份等重活走
//! spawn_blocking，避免阻塞窗口线程。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::AppHandle;
use tauri::State;

use voucher_core::db::repos::attachment_repo;
use voucher_core::db::repos::voucher_repo::VoucherQuery;
use voucher_core::models::VoucherUpdate;
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
pub fn restart_app(app: AppHandle, ctx: Ctx) {
    // 计划内重启（恢复备份生效流程）：先清会话 marker，避免下次启动误报「异常退出」
    let _ = voucher_core::services::recovery::mark_clean_exit(&ctx);
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
    // D-1：列表缩略图（每张凭证首附件，enhanced 优先）。空列表时跳过 SQL
    let thumbs: HashMap<i64, VoucherThumb> = if items.is_empty() {
        HashMap::new()
    } else {
        let ids: Vec<i64> = items.iter().map(|v| v.id).collect();
        let rows = err_string(
            ctx.db
                .with_conn(|conn| attachment_repo::first_thumb_for_vouchers(conn, &ids)),
        )?;
        rows.into_iter()
            .map(|(vid, fp, ep, src)| {
                (
                    vid,
                    VoucherThumb {
                        file_path: fp,
                        enhanced_path: ep,
                        enhanced_source: src,
                    },
                )
            })
            .collect()
    };
    Ok(VoucherListResult {
        items,
        total,
        thumbs,
    })
}

/// D-1 列表缩略图信息：相对附件根目录的路径，前端用 convertFileSrc 转换
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct VoucherThumb {
    pub file_path: String,
    pub enhanced_path: Option<String>,
    pub enhanced_source: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherListResult {
    pub items: Vec<voucher_core::models::Voucher>,
    pub total: u64,
    /// D-1：当前页所有凭证的缩略图信息，key = voucher.id
    pub thumbs: HashMap<i64, VoucherThumb>,
}

#[tauri::command]
pub fn get_voucher(ctx: Ctx, id: i64) -> std::result::Result<voucher_mgr::VoucherDetail, String> {
    err_string(voucher_mgr::get_detail(&ctx, id))
}

#[tauri::command]
pub fn update_voucher(
    ctx: Ctx,
    id: i64,
    update: VoucherUpdate,
) -> std::result::Result<voucher_core::models::Voucher, String> {
    err_string(voucher_mgr::update_voucher(&ctx, id, update))
}

/// 批量删除（单事务 + 附件文件清理），返回实际删除数
#[tauri::command]
pub fn delete_vouchers(ctx: Ctx, ids: Vec<i64>) -> std::result::Result<u64, String> {
    err_string(voucher_mgr::delete_vouchers(&ctx, &ids))
}

/// 批量变更状态（单事务逐张快照，reason=batch_status），返回实际变更数
/// （已是目标状态/不存在的跳过不计入）
#[tauri::command]
pub fn batch_update_status(
    ctx: Ctx,
    ids: Vec<i64>,
    status: voucher_core::models::VoucherStatus,
) -> std::result::Result<usize, String> {
    err_string(voucher_mgr::batch_set_status(&ctx, &ids, status))
}

/// 批量编辑（单事务逐张快照，reason=batch_edit）：日期/备注/标签，返回实际变更数
#[tauri::command]
pub fn batch_edit(
    ctx: Ctx,
    ids: Vec<i64>,
    patch: voucher_mgr::BatchEditPatch,
) -> std::result::Result<usize, String> {
    err_string(voucher_mgr::batch_edit(&ctx, &ids, &patch))
}

#[tauri::command]
pub fn voucher_stats(
    ctx: Ctx,
) -> std::result::Result<voucher_core::db::repos::voucher_repo::VoucherStats, String> {
    err_string(voucher_mgr::stats(&ctx))
}

// ---------- 报表 ----------

/// 近 N 月逐月汇总（默认 6，范围 1–24）
#[tauri::command]
pub fn monthly_summary(
    ctx: Ctx,
    months: Option<u32>,
) -> std::result::Result<Vec<voucher_core::services::reports::MonthlySummary>, String> {
    let n = months.unwrap_or(6).clamp(1, 24);
    err_string(voucher_core::services::reports::monthly_summary(&ctx, n))
}

/// Top 对方账户排行（按金额降序；months=0 表示全部历史，limit 默认 10、范围 1–50）
#[tauri::command]
pub fn top_counterparties(
    ctx: Ctx,
    months: Option<u32>,
    limit: Option<u32>,
) -> std::result::Result<Vec<voucher_core::services::reports::CounterpartyRow>, String> {
    err_string(voucher_core::services::reports::top_counterparties(
        &ctx,
        months.unwrap_or(0),
        limit.unwrap_or(10),
    ))
}

/// 标签分类统计（按金额降序；months=0 表示全部历史，limit 默认 10、范围 1–50；
/// 「未分类」行 tag 为空串、固定排末尾且不占 limit 名额）
#[tauri::command]
pub fn tag_stats(
    ctx: Ctx,
    months: Option<u32>,
    limit: Option<u32>,
) -> std::result::Result<Vec<voucher_core::services::reports::TagRow>, String> {
    err_string(voucher_core::services::reports::tag_stats(
        &ctx,
        months.unwrap_or(0),
        limit.unwrap_or(10),
    ))
}

// ---------- 崩溃恢复 ----------

/// 读取启动时的恢复检查报告（marker 检测与健康检查在 main.rs setup 中完成并缓存于此）
#[tauri::command]
pub fn get_recovery_report(
    state: tauri::State<'_, voucher_core::services::recovery::RecoveryReport>,
) -> voucher_core::services::recovery::RecoveryReport {
    state.inner().clone()
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

// ---------- 导入 / 附件 / OCR（文件选择在 Rust 侧完成，绕开 WebView JS 事件桥的已知竞态） ----------

const IMAGE_FILTER: &[&str] = &["jpg", "jpeg", "png", "bmp", "webp"];

/// 系统对话框选图并直接导入（一图一凭证）。返回 None = 用户取消选择。
#[tauri::command]
pub async fn pick_and_import(
    app: AppHandle,
    ctx: Ctx<'_>,
    run_ocr: Option<bool>,
) -> std::result::Result<Option<ingestion::ImportSummary>, String> {
    // 模态对话框必须 blocking 打开，放进专用线程以免挂起 async runtime
    let picked = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        app.dialog()
            .file()
            .add_filter("图片", IMAGE_FILTER)
            .blocking_pick_files()
    })
    .await
    .map_err(|e| e.to_string())?;

    let files: Vec<PathBuf> = picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| f.into_path().ok())
        .collect();
    if files.is_empty() {
        log::info!("导入对话框：用户取消或未选出图片");
        return Ok(None);
    }
    log::info!("点击导入：选择了 {} 张图片", files.len());
    let core = ctx.inner().clone();
    let started = std::time::Instant::now();
    let summary = tauri::async_runtime::spawn_blocking(move || {
        err_string(ingestion::import_images(
            &core,
            &files,
            None,
            run_ocr.unwrap_or(true),
        ))
    })
    .await
    .map_err(|e| e.to_string())??;
    log::info!(
        "点击导入完成：新建 {} 张，跳过重复 {}，失败 {}，耗时 {:?}",
        summary.vouchers.len(),
        summary.skipped_duplicates.len(),
        summary.failed.len(),
        started.elapsed()
    );
    Ok(Some(summary))
}

/// 系统对话框选一张图片更换凭证图片（换图不换卡）。返回 None = 用户取消选择。
#[tauri::command]
pub async fn pick_and_replace_image(
    app: AppHandle,
    ctx: Ctx<'_>,
    voucher_id: i64,
) -> std::result::Result<Option<voucher_mgr::VoucherDetail>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        app.dialog()
            .file()
            .add_filter("图片", IMAGE_FILTER)
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let path = match picked.and_then(|f| f.into_path().ok()) {
        Some(p) => p,
        None => {
            log::info!("换图对话框：用户取消");
            return Ok(None);
        }
    };
    log::info!("更换图片：凭证 {voucher_id} ← {}", path.display());
    let core = ctx.inner().clone();
    let detail = tauri::async_runtime::spawn_blocking(move || {
        err_string(ingestion::replace_attachment(&core, voucher_id, &path))?;
        err_string(voucher_mgr::get_detail(&core, voucher_id))
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(Some(detail))
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

/// 识别附件并自动填单（Manual 模式：只填空字段，不覆盖已录入内容）。
/// 附件可能被语义化重命名，故返回整份凭证详情供前端刷新。
#[tauri::command]
pub async fn run_ocr(
    ctx: Ctx<'_>,
    attachment_id: i64,
) -> std::result::Result<voucher_mgr::VoucherDetail, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let att = err_string(ocr::run_for_attachment(&core, attachment_id))?;
        err_string(voucher_core::services::ocr_fields::apply_for_attachment(
            &core,
            att.voucher_id,
            att.id,
            voucher_core::services::ocr_fields::FillMode::Manual,
        ))?;
        err_string(voucher_mgr::get_detail(&core, att.voucher_id))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// OCR 字段提取预览（只读）：从附件已识别文本抽取结构化字段，不写库。
/// 前端「识别结果卡片」用——用户确认/勾选后走 update_voucher 应用。
/// v5：有版面数据走几何配对（含置信度），否则回退纯文本。
#[tauri::command]
pub fn ocr_preview_fields(
    ctx: Ctx,
    attachment_id: i64,
) -> std::result::Result<voucher_core::services::ocr_fields::ExtractedFields, String> {
    let fields = ctx
        .db
        .with_conn(|conn| {
            let text = attachment_repo::get(conn, attachment_id)?
                .map(|a| a.ocr_text)
                .ok_or_else(|| {
                    voucher_core::CoreError::NotFound(format!("附件 {attachment_id} 不存在"))
                })?;
            let layout = voucher_core::db::repos::ocr_layout_repo::get_layout(conn, attachment_id)?;
            Ok(match layout {
                Some(lines) if !lines.is_empty() => {
                    voucher_core::services::ocr_layout::extract_with_layout(&lines)
                }
                _ => voucher_core::services::ocr_fields::extract(text.as_deref().unwrap_or("")),
            })
        })
        .map_err(|e: voucher_core::CoreError| e.to_string())?;
    Ok(fields)
}

#[tauri::command]
pub async fn run_ocr_pending(
    ctx: Ctx<'_>,
    app: AppHandle,
    limit: Option<u32>,
) -> std::result::Result<usize, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 逐张推送进度事件（ocr:progress {done, total}），积压量大时前端显示进度条
        err_string(ocr::run_pending(
            &core,
            limit.unwrap_or(50),
            &|done, total| {
                use tauri::Emitter;
                let _ = app.emit(
                    "ocr:progress",
                    serde_json::json!({ "done": done, "total": total }),
                );
            },
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// v0.8：批量重新增强存量附件（autocrop 修复后按新算法重算增强版）。
/// 逐张推送 enhance:progress {done, total}；返回 { ok, failed }。
#[tauri::command]
pub async fn re_enhance_all(
    ctx: Ctx<'_>,
    app: AppHandle,
) -> std::result::Result<serde_json::Value, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (ok, failed) = err_string(voucher_core::services::enhance::re_enhance_all(
            &core,
            &|done, total| {
                use tauri::Emitter;
                let _ = app.emit(
                    "enhance:progress",
                    serde_json::json!({ "done": done, "total": total }),
                );
            },
        ))?;
        Ok(serde_json::json!({ "ok": ok, "failed": failed }))
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

/// 按天分组导出：每个日期一份「凭证日报」PDF（一图一凭证的默认导出形态）
#[tauri::command]
pub async fn export_pdf_daily(
    ctx: Ctx<'_>,
    voucher_ids: Vec<i64>,
    options: Option<ExportOptions>,
) -> std::result::Result<Vec<voucher_core::models::ExportBatch>, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || match options {
        Some(o) => err_string(export::export_vouchers_daily(&core, &voucher_ids, &o)),
        None => err_string(export::export_daily_with_defaults(&core, &voucher_ids)),
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

/// 关闭备份进行中时用户点击「跳过备份直接退出」：
/// 写 clean-exit marker（数据无损，仅本次关闭备份可能不完整）后直接销毁窗口。
/// 后台备份线程随后被进程退出终止——其写盘目标全部是 temp 文件，中断不影响数据。
#[tauri::command]
pub fn skip_closing_backup(
    ctx: Ctx,
    window: tauri::WebviewWindow,
) -> std::result::Result<(), String> {
    let _ = voucher_core::services::recovery::mark_clean_exit(&ctx);
    window.destroy().map_err(|e| format!("关闭窗口失败：{e}"))
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

/// 恢复演练：完整校验 + 解包到临时目录 + 数据库可用性验证，不触碰真实数据
#[tauri::command]
pub async fn drill_backup(
    ctx: Ctx<'_>,
    path: Option<String>,
) -> std::result::Result<backup::DrillReport, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        err_string(backup::drill_backup(&core, path.as_deref().map(Path::new)))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 手动上传指定备份到 WebDAV（补传旧备份；新备份随 run_backup 自动上传）
#[tauri::command]
pub async fn upload_backup(ctx: Ctx<'_>, path: String) -> std::result::Result<(), String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        err_string(backup::upload_backup_to_webdav(&core, Path::new(&path)))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// WebDAV 连接测试：返回远端已有备份数（配置错误/网络问题会报错）
#[tauri::command]
pub async fn test_webdav(ctx: Ctx<'_>) -> std::result::Result<u32, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || err_string(backup::test_webdav(&core)))
        .await
        .map_err(|e| e.to_string())?
}

/// 列出 WebDAV 远端备份（新 → 旧）
#[tauri::command]
pub async fn list_webdav_backups(
    ctx: Ctx<'_>,
) -> std::result::Result<Vec<backup::WebdavBackupRow>, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || err_string(backup::list_webdav_backups(&core)))
        .await
        .map_err(|e| e.to_string())?
}

/// 下载 WebDAV 远端备份到本地 backups/ 目录（幂等：已存在直接返回路径）
#[tauri::command]
pub async fn download_webdav_backup(
    ctx: Ctx<'_>,
    file_name: String,
) -> std::result::Result<String, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        err_string(backup::download_webdav_backup(&core, &file_name))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------- v0.6 文档扫描矫正（C 步：增强缓存） ----------

/// 增强缓存统计（设置页显示「N 个增强版，共 X MB」）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceStatus {
    pub total: u32,
    pub total_size: u64,
    pub screenshot_count: u32,
    pub scan_count: u32,
}

#[tauri::command]
pub fn enhance_status(ctx: Ctx) -> EnhanceStatus {
    use voucher_core::db::repos::attachment_repo;
    let atts = ctx
        .db
        .with_conn(attachment_repo::list_enhanced)
        .unwrap_or_default();
    let mut total_size: u64 = 0;
    let mut screenshot_count: u32 = 0;
    let mut scan_count: u32 = 0;
    for a in &atts {
        if let Some(rel) = &a.enhanced_path {
            let abs = ctx.paths.attachment_abs(rel);
            if let Ok(m) = std::fs::metadata(&abs) {
                total_size += m.len();
            }
        }
        match a.enhanced_source {
            voucher_core::models::EnhanceSource::Screenshot => screenshot_count += 1,
            _ => scan_count += 1,
        }
    }
    EnhanceStatus {
        total: atts.len() as u32,
        total_size,
        screenshot_count,
        scan_count,
    }
}

/// 清除所有增强版缓存（DB 字段置 NULL + 物理文件删除）。
/// 返回实际删除的文件数（可能 < DB 记录数：文件已被手动删除）
#[tauri::command]
pub fn clear_enhance_cache(ctx: Ctx) -> std::result::Result<u32, String> {
    use voucher_core::db::repos::attachment_repo;
    let atts = err_string(ctx.db.with_conn(attachment_repo::list_enhanced))?;
    let mut deleted = 0u32;
    for a in &atts {
        if let Some(rel) = &a.enhanced_path {
            let abs = ctx.paths.attachment_abs(rel);
            if ctx.paths.is_inside_attachments(&abs) {
                if std::fs::remove_file(&abs).is_ok() {
                    deleted += 1;
                } else {
                    // 文件已不在也视为清理（保留 DB 同步）
                    deleted += 1;
                }
            }
        }
        err_string(
            ctx.db
                .with_conn(|conn| attachment_repo::clear_enhanced(conn, a.id)),
        )?;
    }
    Ok(deleted)
}

// ---------- v0.6.5 D-2 增强缓存管理 UI 升级 ----------

/// 增强缓存状态 v2（设置页增强缓存面板：摘要 + 命中率）
#[tauri::command]
pub fn enhance_status_v2(
    ctx: Ctx,
) -> std::result::Result<voucher_core::services::enhance_cache::EnhanceStatusV2, String> {
    err_string(voucher_core::services::enhance_cache::status_v2(&ctx))
}

/// 按策略释放增强缓存
/// strategy: "all" | "older_than_days" | "larger_than_mb" | "smart"
/// threshold: 阈值（天数 / MB；all 忽略；smart = 宽限期天数，默认 30）
#[tauri::command]
pub fn clear_enhance_cache_v2(
    ctx: Ctx,
    strategy: String,
    threshold: u32,
) -> std::result::Result<voucher_core::services::enhance_cache::ClearSummary, String> {
    let s = voucher_core::services::enhance_cache::ClearStrategy::parse(&strategy)
        .ok_or_else(|| format!("未知释放策略: {strategy}"))?;
    err_string(voucher_core::services::enhance_cache::clear_v2(
        &ctx, s, threshold,
    ))
}

/// 单条释放某张附件的增强版
#[tauri::command]
pub fn delete_enhanced_one(ctx: Ctx, attachment_id: i64) -> std::result::Result<u32, String> {
    err_string(voucher_core::services::enhance_cache::delete_one(
        &ctx,
        attachment_id,
    ))
}

/// 最近增强列表（按文件 mtime 倒序）
#[tauri::command]
pub fn recent_enhanced(
    ctx: Ctx,
    limit: u32,
) -> std::result::Result<Vec<voucher_core::services::enhance_cache::RecentEnhancedRow>, String> {
    err_string(voucher_core::services::enhance_cache::recent_enhanced(
        &ctx, limit,
    ))
}

/// D-7：查看器打开了某附件的增强版 → 记一条 viewer 命中。
/// 附件无增强版时返回错误（前端静默忽略，不影响查看）。
#[tauri::command]
pub fn record_viewer_hit(ctx: Ctx, attachment_id: i64) -> std::result::Result<(), String> {
    err_string(voucher_core::services::enhance_cache::record_hit(
        &ctx,
        attachment_id,
        voucher_core::db::repos::enhance_hit_repo::HitSource::Viewer,
    ))
}

/// D-10：命中趋势（近 N 天按日聚合，ocr / export / viewer 分列，缺日补零）
#[tauri::command]
pub fn enhance_hit_trend(
    ctx: Ctx,
    days: u32,
) -> std::result::Result<voucher_core::services::enhance_cache::HitTrend, String> {
    err_string(voucher_core::services::enhance_cache::hit_trend(&ctx, days))
}
