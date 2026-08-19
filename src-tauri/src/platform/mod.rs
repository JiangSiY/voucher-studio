// 平台特定代码（打印链路等，Windows/macOS 分支入口）
// 打印链路落地结论待 G1 Spike A；当前提供「打开 PDF 由系统查看器打印」的兜底路径。

use std::path::Path;

use tauri::AppHandle;
use tauri::Emitter;
use tauri_plugin_opener::OpenerExt;

use std::sync::Arc;
use voucher_core::CoreContext;

/// 用系统默认程序打开文件/目录（PDF、备份目录等）
pub fn open_path(app: &AppHandle, path: &str) -> Result<(), String> {
    app.opener()
        .open_path(path.to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// 微信收件夹监听（配置启用时启动）：新图片自动入库并通知前端
pub fn spawn_wechat_if_enabled(app: AppHandle, ctx: &Arc<CoreContext>) {
    let cfg = ctx.config();
    if !cfg.wechat.enabled || cfg.wechat.watch_dir.trim().is_empty() {
        return;
    }
    let dir = std::path::PathBuf::from(cfg.wechat.watch_dir.trim());
    let ctx = ctx.clone();
    let (tx, rx) = std::sync::mpsc::channel::<std::path::PathBuf>();
    if let Err(e) = voucher_core::services::wechat::spawn_watch(dir, tx) {
        log::warn!("微信监听启动失败: {e}");
        let _ = app.emit("wechat:error", e.to_string());
        return;
    }
    std::thread::Builder::new()
        .name("wechat-ingest".into())
        .spawn(move || {
            while let Ok(path) = rx.recv() {
                match voucher_core::services::ingestion::import_images(&ctx, &[path], None, true) {
                    Ok(summary) => {
                        log::info!(
                            "微信图片入库: {} → {}",
                            summary.voucher_no,
                            summary.imported.len()
                        );
                        let _ = app.emit("wechat:imported", &summary);
                    }
                    Err(e) => {
                        log::warn!("微信图片入库失败: {e}");
                        let _ = app.emit("wechat:error", e.to_string());
                    }
                }
            }
        })
        .ok();
}

/// 定时备份调度：每 5 分钟检查一次到期（纯计算，见 voucher-core::services::scheduler）
pub fn spawn_backup_scheduler(app: AppHandle, ctx: Arc<CoreContext>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            let c = ctx.clone();
            let due = tauri::async_runtime::spawn_blocking(move || {
                voucher_core::services::scheduler::backup_due(&c)
            })
            .await;
            if !matches!(due, Ok(Ok(true))) {
                continue;
            }
            let c = ctx.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                voucher_core::services::backup::run_backup(&c)
            })
            .await;
            match result {
                Ok(Ok(info)) => {
                    log::info!("自动备份完成: {}", info.file_name);
                    let _ = app.emit("backup:done", &info);
                }
                Ok(Err(e)) => log::warn!("自动备份失败: {e}"),
                Err(e) => log::warn!("备份任务异常: {e}"),
            }
        }
    });
}

/// 应用启动时落位「待恢复备份」（restore 命令写入 .pending 文件，重启生效）
pub fn apply_pending_restore(paths: &voucher_core::utils::paths::AppPaths) {
    let pending_db = paths.base.join("voucher.db.pending");
    if pending_db.exists() {
        let _ = std::fs::remove_file(paths.db_file());
        let _ = std::fs::remove_file(paths.base.join("voucher.db-wal"));
        let _ = std::fs::remove_file(paths.base.join("voucher.db-shm"));
        if std::fs::rename(&pending_db, paths.db_file()).is_ok() {
            log::info!("已应用待恢复的数据库备份");
        }
    }
    let pending_cfg = paths.config_dir.join("app.json.pending");
    if pending_cfg.exists() {
        let _ = std::fs::remove_file(paths.config_file());
        let _ = std::fs::rename(&pending_cfg, paths.config_file());
    }
}

/// 恢复流程：备份内容校验后抽取为 .pending 文件，重启后由 apply_pending_restore 落位
pub fn stage_restore(
    ctx: &CoreContext,
    zip_path: &Path,
) -> Result<voucher_core::services::backup::RestoreSummary, String> {
    let staging = ctx.paths.temp.join("restore-staging");
    let _ = std::fs::remove_dir_all(&staging);
    voucher_core::services::backup::restore_backup(zip_path, &staging)
        .map_err(|e| e.to_string())?;

    let staged_db = staging.join("data").join("voucher.db");
    if staged_db.exists() {
        std::fs::copy(&staged_db, ctx.paths.base.join("voucher.db.pending"))
            .map_err(|e| format!("写入待恢复文件失败: {e}"))?;
    }
    let staged_cfg = staging.join("config").join("app.json");
    if staged_cfg.exists() {
        std::fs::copy(&staged_cfg, ctx.paths.config_dir.join("app.json.pending"))
            .map_err(|e| format!("写入待恢复配置失败: {e}"))?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    voucher_core::services::backup::inspect_backup(zip_path).map_err(|e| e.to_string())
}
