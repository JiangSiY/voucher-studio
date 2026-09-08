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
                Ok(Err(e)) => {
                    log::warn!("自动备份失败: {e}");
                    let _ = app.emit("backup:failed", e.to_string());
                }
                Err(e) => {
                    log::warn!("备份任务异常: {e}");
                    let _ = app.emit("backup:failed", e.to_string());
                }
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
    // 附件（v2 备份）：整目录替换，旧目录先改名兜底
    let pending_att = paths.attachments.with_extension("pending");
    if pending_att.is_dir() {
        let old = paths.attachments.with_extension("old-restore");
        let _ = std::fs::remove_dir_all(&old);
        if paths.attachments.is_dir() {
            let _ = std::fs::rename(&paths.attachments, &old);
        }
        if std::fs::rename(&pending_att, &paths.attachments).is_ok() {
            log::info!("已应用待恢复的附件目录");
            let _ = std::fs::remove_dir_all(&old);
        } else if old.is_dir() {
            // 回滚：附件替换失败时保留原目录
            let _ = std::fs::rename(&old, &paths.attachments);
        }
    }
}

/// 恢复流程：备份内容校验后抽取为 .pending 文件/目录，重启后由 apply_pending_restore 落位
pub fn stage_restore(
    ctx: &CoreContext,
    zip_path: &Path,
) -> Result<voucher_core::services::backup::RestoreSummary, String> {
    let staging = ctx.paths.temp.join("restore-staging");
    let _ = std::fs::remove_dir_all(&staging);
    voucher_core::services::backup::restore_backup(ctx, zip_path, &staging)
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
    // 附件（v2 备份）：staging → attachments.pending（目录级替换，重启落位）
    let staged_att = staging.join("attachments");
    if staged_att.is_dir() {
        let pending_att = ctx.paths.attachments.with_extension("pending");
        let _ = std::fs::remove_dir_all(&pending_att);
        std::fs::rename(&staged_att, &pending_att)
            .map_err(|e| format!("写入待恢复附件失败: {e}"))?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    voucher_core::services::backup::inspect_backup(ctx, zip_path).map_err(|e| e.to_string())
}
