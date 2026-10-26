//! 导入 / 附件 / OCR 命令（文件选择在 Rust 侧完成，绕开 WebView JS 事件桥的已知竞态）。
//!
//! 越层约束（体检 #11）：附件查询 / OCR 预览经 `services::query` 门面取数，
//! 命令层不直连仓储层或裸数据库连接（CI grep 门禁固化）。

use super::*;
use voucher_core::services::query as qsvc;
use voucher_core::services::{ingestion, ocr, voucher_mgr};
// ---------- 导入 / 附件 / OCR（文件选择在 Rust 侧完成，绕开 WebView JS 事件桥的已知竞态） ----------

const IMAGE_FILTER: &[&str] = &["jpg", "jpeg", "png", "bmp", "webp"];

/// 系统对话框选图并直接导入（一图一凭证）。返回 None = 用户取消选择。
#[tauri::command]
pub async fn pick_and_import(
    app: AppHandle,
    ctx: Ctx<'_>,
    run_ocr: Option<bool>,
) -> std::result::Result<Option<ingestion::ImportSummary>, ApiError> {
    // 模态对话框必须 blocking 打开，放进专用线程以免挂起 async runtime
    let picked = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        app.dialog()
            .file()
            .add_filter("图片", IMAGE_FILTER)
            .blocking_pick_files()
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

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
        err_api(ingestion::import_images(
            &core,
            &files,
            None,
            run_ocr.unwrap_or(true),
        ))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))??;
    // 逐条记失败原因（2026-09-23）：原先只记数量，日志里看不出"为什么失败"
    for f in &summary.failed {
        log::warn!("导入失败：{} —— {}", f.file_name, f.reason);
    }
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
) -> std::result::Result<Option<voucher_mgr::VoucherDetail>, ApiError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        app.dialog()
            .file()
            .add_filter("图片", IMAGE_FILTER)
            .blocking_pick_file()
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

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
        err_api(ingestion::replace_attachment(&core, voucher_id, &path))?;
        err_api(voucher_mgr::get_detail(&core, voucher_id))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))??;
    Ok(Some(detail))
}

#[tauri::command]
pub fn attachment_abs_path(ctx: Ctx, id: i64) -> std::result::Result<String, ApiError> {
    let att = err_api(qsvc::attachment(&ctx, id))?;
    match att {
        Some(a) => Ok(ctx
            .paths
            .attachment_abs(&a.file_path)
            .to_string_lossy()
            .to_string()),
        None => Err(ApiError::plain("not_found", format!("附件 {id} 不存在"))),
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
) -> std::result::Result<voucher_core::models::Attachment, ApiError> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        err_api(voucher_core::services::crop::crop_attachment(
            &core,
            attachment_id,
            rect,
            rotation_deg,
        ))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
}

/// v0.13：系统对话框选账单 CSV → 对账（只读匹配，不写库）→ 报告落 exports/。
/// 返回 None = 用户取消选择。
#[tauri::command]
pub async fn pick_and_reconcile(
    app: AppHandle,
    ctx: Ctx<'_>,
) -> std::result::Result<Option<voucher_core::services::reconcile::ReconcileReport>, ApiError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        app.dialog()
            .file()
            .add_filter("账单 CSV", &["csv"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let path = match picked.and_then(|f| f.into_path().ok()) {
        Some(p) => p,
        None => {
            log::info!("对账对话框：用户取消");
            return Ok(None);
        }
    };
    let core = ctx.inner().clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        err_api(voucher_core::services::reconcile::reconcile(&core, &path))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))??;
    Ok(Some(report))
}

/// 识别附件并自动填单（Manual 模式：只填空字段，不覆盖已录入内容）。
/// 附件可能被语义化重命名，故返回整份凭证详情供前端刷新。
///
/// 这是界面上「重新识别」按钮的入口：**跳过缓存真重跑**（`ocr::rerun_for_attachment`）——
/// 命中缓存直接返回旧文本会在换引擎后让人以为「识别成功但什么都没变」。
#[tauri::command]
pub async fn run_ocr(
    ctx: Ctx<'_>,
    attachment_id: i64,
) -> std::result::Result<voucher_mgr::VoucherDetail, ApiError> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let att = err_api(ocr::rerun_for_attachment(&core, attachment_id))?;
        err_api(voucher_core::services::ocr_fields::apply_for_attachment(
            &core,
            att.voucher_id,
            att.id,
            voucher_core::services::ocr_fields::FillMode::Manual,
        ))?;
        err_api(voucher_mgr::get_detail(&core, att.voucher_id))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
}

/// OCR 字段提取预览（只读）：从附件已识别文本抽取结构化字段，不写库。
/// 前端「识别结果卡片」用——用户确认/勾选后走 update_voucher 应用。
/// v5：有版面数据走几何配对（含置信度），否则回退纯文本。
#[tauri::command]
pub fn ocr_preview_fields(
    ctx: Ctx,
    attachment_id: i64,
) -> std::result::Result<voucher_core::services::ocr_fields::ExtractedFields, ApiError> {
    err_api(qsvc::ocr_preview(&ctx, attachment_id))
}

#[tauri::command]
pub async fn run_ocr_pending(
    ctx: Ctx<'_>,
    app: AppHandle,
    limit: Option<u32>,
) -> std::result::Result<usize, ApiError> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 逐张推送进度事件（ocr:progress {done, total}），积压量大时前端显示进度条
        err_api(ocr::run_pending(
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
    .map_err(|e| ApiError::internal(e.to_string()))?
}

/// v0.8：批量重新增强存量附件（autocrop 修复后按新算法重算增强版）。
/// 逐张推送 enhance:progress {done, total}；返回 { ok, failed }。
#[tauri::command]
pub async fn re_enhance_all(
    ctx: Ctx<'_>,
    app: AppHandle,
) -> std::result::Result<serde_json::Value, ApiError> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (ok, failed) = err_api(voucher_core::services::enhance::re_enhance_all(
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
    .map_err(|e| ApiError::internal(e.to_string()))?
}

// ---------- OCR 引擎状态 ----------

/// 本地引擎能力（v0.23 起本地识别用 macOS 原生 Vision，不再需要下载模型）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOcrStatus {
    /// 本地引擎是否可用（仅 macOS 有 Vision）
    pub available: bool,
    /// 引擎名称（展示用）
    pub engine_name: String,
    /// 说明文案（无需安装模型 / 仅 macOS 可用）
    pub note: String,
}

#[tauri::command]
pub fn local_ocr_status() -> LocalOcrStatus {
    let macos = cfg!(target_os = "macos");
    LocalOcrStatus {
        available: macos,
        engine_name: "Apple Vision".to_string(),
        note: if macos {
            "本地识别使用 macOS 原生 Vision 框架，无需下载模型".to_string()
        } else {
            "本地识别仅 macOS 可用；其它平台请使用云端识别".to_string()
        },
    }
}

/// 最近一次 OCR 的实际引擎与云端失败原因（设置页显示，判断「为什么没走云端」）
#[tauri::command]
pub fn ocr_health(ctx: Ctx) -> voucher_core::OcrHealth {
    ctx.ocr_health()
}
