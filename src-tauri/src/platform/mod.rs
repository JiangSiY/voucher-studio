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

// ---------- 微信收件夹监听 ----------

/// 微信监听运行状态（前端设置页展示）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WechatStatus {
    /// 配置是否启用
    pub enabled: bool,
    /// 监听线程是否在运行
    pub running: bool,
    pub watch_dir: String,
    /// 最近一次启动失败/入库失败的原因
    pub error: Option<String>,
}

struct WechatWatch {
    stop: voucher_core::services::wechat::StopHandle,
}

static WECHAT_WATCH: std::sync::Mutex<Option<WechatWatch>> = std::sync::Mutex::new(None);
static WECHAT_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// 查询微信监听状态（命令层透传给前端）
pub fn wechat_status(ctx: &CoreContext) -> WechatStatus {
    let cfg = ctx.config();
    let running = WECHAT_WATCH.lock().map(|w| w.is_some()).unwrap_or(false);
    let error = WECHAT_ERROR.lock().ok().and_then(|e| e.clone());
    WechatStatus {
        enabled: cfg.wechat.enabled,
        running,
        watch_dir: cfg.wechat.watch_dir.clone(),
        error,
    }
}

/// 让监听状态与配置同步：启动时与每次保存配置后调用（热重载，无需重启应用）。
/// 1. 先停止现有监听；2. 配置启用且目录有效 → 重新启动。
pub fn sync_wechat_watch(app: AppHandle, ctx: &Arc<CoreContext>) {
    // 停止现有监听（一个 TICK 内旧 watcher 释放；期间旧事件由旧消费线程处理，
    // 重复入库由附件内容哈希去重兜底）
    if let Ok(mut guard) = WECHAT_WATCH.lock() {
        if let Some(w) = guard.take() {
            w.stop.stop();
        }
    }

    let cfg = ctx.config();
    if !cfg.wechat.enabled || cfg.wechat.watch_dir.trim().is_empty() {
        if let Ok(mut e) = WECHAT_ERROR.lock() {
            *e = None;
        }
        return;
    }
    let dir = std::path::PathBuf::from(cfg.wechat.watch_dir.trim());
    let ctx = ctx.clone();
    let (tx, rx) = std::sync::mpsc::channel::<std::path::PathBuf>();
    match voucher_core::services::wechat::spawn_watch(dir, tx) {
        Ok(stop) => {
            if let Ok(mut guard) = WECHAT_WATCH.lock() {
                *guard = Some(WechatWatch { stop });
            }
            if let Ok(mut e) = WECHAT_ERROR.lock() {
                *e = None;
            }
            log::info!("微信监听已启动: {}", cfg.wechat.watch_dir);
        }
        Err(e) => {
            log::warn!("微信监听启动失败: {e}");
            let msg = e.to_string();
            if let Ok(mut e) = WECHAT_ERROR.lock() {
                *e = Some(msg.clone());
            }
            let _ = app.emit("wechat:error", msg);
            return;
        }
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
    // 附件（v2 备份）：staging → attachments.pending（目录级替换，重启落位）
    let staged_att = staging.join("attachments");
    if staged_att.is_dir() {
        let pending_att = ctx.paths.attachments.with_extension("pending");
        let _ = std::fs::remove_dir_all(&pending_att);
        std::fs::rename(&staged_att, &pending_att)
            .map_err(|e| format!("写入待恢复附件失败: {e}"))?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    voucher_core::services::backup::inspect_backup(zip_path).map_err(|e| e.to_string())
}
