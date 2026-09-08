//! 导入 / 附件 / OCR 命令（文件选择在 Rust 侧完成，绕开 WebView JS 事件桥的已知竞态）。

use super::*;
use voucher_core::db::repos::attachment_repo;
use voucher_core::services::{ingestion, ocr, voucher_mgr};
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

/// v0.12：手动裁剪/旋转（自动矫正失败的人工补救）。旋转 → 裁剪 → 覆盖写增强版；
/// 返回更新后的附件（enhanced_path 已指向手动版）。
#[tauri::command]
pub async fn crop_attachment(
    ctx: Ctx<'_>,
    attachment_id: i64,
    rect: voucher_core::services::crop::CropRect,
    rotation_deg: i32,
) -> std::result::Result<voucher_core::models::Attachment, String> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        err_string(voucher_core::services::crop::crop_attachment(
            &core,
            attachment_id,
            rect,
            rotation_deg,
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// v0.13：系统对话框选账单 CSV → 对账（只读匹配，不写库）→ 报告落 exports/。
/// 返回 None = 用户取消选择。
#[tauri::command]
pub async fn pick_and_reconcile(
    app: AppHandle,
    ctx: Ctx<'_>,
) -> std::result::Result<Option<voucher_core::services::reconcile::ReconcileReport>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        app.dialog()
            .file()
            .add_filter("账单 CSV", &["csv"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let path = match picked.and_then(|f| f.into_path().ok()) {
        Some(p) => p,
        None => {
            log::info!("对账对话框：用户取消");
            return Ok(None);
        }
    };
    let core = ctx.inner().clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        err_string(voucher_core::services::reconcile::reconcile(&core, &path))
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(Some(report))
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
                _ => voucher_core::services::ocr_fields::extract_validated(
                    text.as_deref().unwrap_or(""),
                ),
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
