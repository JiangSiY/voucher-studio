//! 备份与恢复命令（本地备份 / WebDAV 云端 / 演练 / 恢复就绪）。

use super::*;
use voucher_core::services::backup;
// ---------- 备份与恢复 ----------

#[tauri::command]
pub async fn run_backup(ctx: Ctx<'_>) -> std::result::Result<backup::BackupInfo, ApiError> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || err_api(backup::run_backup(&core)))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
}

/// 关闭备份进行中时用户点击「跳过备份直接退出」：
/// 写 clean-exit marker（数据无损，仅本次关闭备份可能不完整）后直接销毁窗口。
/// 后台备份线程随后被进程退出终止——其写盘目标全部是 temp 文件，中断不影响数据。
#[tauri::command]
pub fn skip_closing_backup(
    ctx: Ctx,
    window: tauri::WebviewWindow,
) -> std::result::Result<(), ApiError> {
    let _ = voucher_core::services::recovery::mark_clean_exit(&ctx);
    window
        .destroy()
        .map_err(|e| ApiError::internal(format!("关闭窗口失败：{e}")))
}

#[tauri::command]
pub fn list_backups(ctx: Ctx) -> std::result::Result<Vec<backup::BackupInfo>, ApiError> {
    err_api(backup::list_backups(&ctx))
}

#[tauri::command]
pub fn inspect_backup(
    ctx: Ctx,
    path: String,
) -> std::result::Result<backup::RestoreSummary, ApiError> {
    // 快照会自动与附件包合成全量再校验（v0.9 本地增量透明兼容）
    err_api(backup::inspect_backup(&ctx, Path::new(&path)))
}

/// v0.9 导出完整备份：快照 + 附件包合成单个全量 zip 落入 backups/（供拷走）
#[tauri::command]
pub fn export_full_backup(ctx: Ctx, path: String) -> std::result::Result<String, ApiError> {
    err_api(
        backup::export_full_backup(&ctx, Path::new(&path))
            .map(|p| p.to_string_lossy().into_owned()),
    )
}

#[tauri::command]
pub fn restore_backup(
    ctx: Ctx,
    path: String,
) -> std::result::Result<backup::RestoreSummary, ApiError> {
    // 恢复走「待恢复文件 + 重启生效」流程，避免在运行中替换数据库
    crate::platform::stage_restore(&ctx, Path::new(&path)).map_err(ApiError::internal)
}

/// 恢复演练：完整校验 + 解包到临时目录 + 数据库可用性验证，不触碰真实数据
#[tauri::command]
pub async fn drill_backup(
    ctx: Ctx<'_>,
    path: Option<String>,
) -> std::result::Result<backup::DrillReport, ApiError> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        err_api(backup::drill_backup(&core, path.as_deref().map(Path::new)))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
}

/// 手动上传指定备份到 WebDAV（补传旧备份；新备份随 run_backup 自动上传）
#[tauri::command]
pub async fn upload_backup(ctx: Ctx<'_>, path: String) -> std::result::Result<(), ApiError> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        err_api(backup::upload_backup_to_webdav(&core, Path::new(&path)))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
}

/// WebDAV 连接测试：返回远端已有备份数（配置错误/网络问题会报错）
#[tauri::command]
pub async fn test_webdav(ctx: Ctx<'_>) -> std::result::Result<u32, ApiError> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || err_api(backup::test_webdav(&core)))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
}

/// 列出 WebDAV 远端备份（新 → 旧）
#[tauri::command]
pub async fn list_webdav_backups(
    ctx: Ctx<'_>,
) -> std::result::Result<Vec<backup::WebdavBackupRow>, ApiError> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || err_api(backup::list_webdav_backups(&core)))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
}

/// 下载 WebDAV 远端备份到本地 backups/ 目录（幂等：已存在直接返回路径）
#[tauri::command]
pub async fn download_webdav_backup(
    ctx: Ctx<'_>,
    file_name: String,
) -> std::result::Result<String, ApiError> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        err_api(backup::download_webdav_backup(&core, &file_name))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
}
