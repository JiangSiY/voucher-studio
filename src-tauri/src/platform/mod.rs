// 运行时支撑（打开路径、备份调度、待恢复落位）——跨平台实现，无平台分支。
// 打印链路结论见 G1 Spike A：当前提供「打开 PDF 由系统查看器打印」的兜底路径。

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

/// 应用启动时落位「待恢复备份」（restore 命令写入 .pending 文件，重启生效）。
///
/// **数据安全铁律（2026-09-23 审计修复）**：落位前必须校验、失败必须回滚，
/// 绝不能「先删活库再改名」——旧实现正是如此：`.pending` 半截（`fs::copy` 会先截断目标）
/// 或 rename 失败时，活库已经没了，启动会新建空库，用户看到「界面还在、凭证全没了」
/// 且没有任何报错。同级附件分支本来就有回滚，数据库/配置分支没有，属实现疏漏。
pub fn apply_pending_restore(paths: &voucher_core::utils::paths::AppPaths) {
    apply_pending_db(paths);
    apply_pending_config(paths);
    apply_pending_attachments(paths);
}

/// 数据库落位：校验通过才换入；换入失败回滚并保留活库。
fn apply_pending_db(paths: &voucher_core::utils::paths::AppPaths) {
    let pending = paths.base.join("voucher.db.pending");
    if !pending.is_file() {
        return;
    }
    if let Err(e) = voucher_core::services::backup::validate_db_file(&pending) {
        log::error!(
            "待恢复数据库未通过校验，已放弃落位并保留当前数据：{e}（{}）",
            pending.display()
        );
        return;
    }
    let live = paths.db_file();
    let wal = paths.base.join("voucher.db-wal");
    let shm = paths.base.join("voucher.db-shm");
    let backup = paths.base.join("voucher.db.old-restore");

    // 活库**改名保留**而不是删除：rename 失败时还能改回来
    let _ = std::fs::remove_file(&backup);
    if live.exists() && std::fs::rename(&live, &backup).is_err() {
        log::error!(
            "待恢复数据库落位失败：无法移开当前数据库 {}，已保持原状",
            live.display()
        );
        return;
    }
    // 旧库的 WAL/SHM 一并挪走（新主库文件不该与旧 WAL 并存）
    let wal_old = paths.base.join("voucher.db-wal.old-restore");
    let shm_old = paths.base.join("voucher.db-shm.old-restore");
    let _ = std::fs::rename(&wal, &wal_old);
    let _ = std::fs::rename(&shm, &shm_old);

    if std::fs::rename(&pending, &live).is_ok() {
        log::info!("已应用待恢复的数据库备份");
        let _ = std::fs::remove_file(&backup);
        let _ = std::fs::remove_file(&wal_old);
        let _ = std::fs::remove_file(&shm_old);
    } else {
        // 回滚：把原库改回来，待恢复文件留在原地供排查
        log::error!("待恢复数据库落位失败，正在回滚到原数据库");
        let _ = std::fs::rename(&backup, &live);
        let _ = std::fs::rename(&wal_old, &wal);
        let _ = std::fs::rename(&shm_old, &shm);
    }
}

/// 配置落位：同样先校验（能被 JSON 解析）再换入，失败保留原配置。
fn apply_pending_config(paths: &voucher_core::utils::paths::AppPaths) {
    let pending = paths.config_dir.join("app.json.pending");
    if !pending.is_file() {
        return;
    }
    let parsed = std::fs::read_to_string(&pending)
        .map_err(|e| e.to_string())
        .and_then(|s| {
            serde_json::from_str::<serde_json::Value>(&s)
                .map(|_| ())
                .map_err(|e| e.to_string())
        });
    if let Err(e) = parsed {
        log::error!(
            "待恢复配置未通过校验，已放弃落位并保留当前配置：{e}（{}）",
            pending.display()
        );
        return;
    }
    let live = paths.config_file();
    let backup = paths.config_dir.join("app.json.old-restore");
    let _ = std::fs::remove_file(&backup);
    if live.exists() && std::fs::rename(&live, &backup).is_err() {
        log::error!("待恢复配置落位失败：无法移开当前配置，已保持原状");
        return;
    }
    if std::fs::rename(&pending, &live).is_ok() {
        log::info!("已应用待恢复的配置");
        let _ = std::fs::remove_file(&backup);
    } else {
        log::error!("待恢复配置落位失败，正在回滚");
        let _ = std::fs::rename(&backup, &live);
    }
}

/// 附件（v2 备份）：整目录替换，旧目录先改名兜底
fn apply_pending_attachments(paths: &voucher_core::utils::paths::AppPaths) {
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
    // 陈旧 pending 必须**无条件**清理：上次恢复若被中断，残留的 attachments.pending
    // 会在下次启动被误落位（旧实现只在「本次含附件」的分支里清理）。
    clear_stale_pending(&ctx.paths);
    let pending_att = ctx.paths.attachments.with_extension("pending");

    let summary = voucher_core::services::backup::restore_backup(ctx, zip_path, &staging)
        .map_err(|e| e.to_string())?;
    if !summary.attachments_included {
        log::warn!(
            "该备份不含附件实体（快照只含原图/无附件）：恢复后界面将回退原图，增强版可在「扫描矫正」一键重建"
        );
    }

    // 待恢复文件一律「临时名 → rename」落盘：`fs::copy` 会先截断目标，
    // 中途断电/写满会留下半截文件，而落位端（apply_pending_db）虽有 integrity_check 兜底，
    // 也不该让半截文件出现在最终名字上（用户看到 `.pending` 会以为恢复已就绪）。
    let staged_db = staging.join("data").join("voucher.db");
    if staged_db.exists() {
        let pending_db = ctx.paths.base.join("voucher.db.pending");
        let tmp = ctx.paths.base.join("voucher.db.pending.tmp");
        std::fs::copy(&staged_db, &tmp).map_err(|e| format!("写入待恢复文件失败: {e}"))?;
        voucher_core::utils::jsonio::commit_file(&tmp, &pending_db)
            .map_err(|e| format!("落位待恢复文件失败: {e}"))?;
    }
    let staged_cfg = staging.join("config").join("app.json");
    if staged_cfg.exists() {
        let pending_cfg = ctx.paths.config_dir.join("app.json.pending");
        let tmp = ctx.paths.config_dir.join("app.json.pending.tmp");
        std::fs::copy(&staged_cfg, &tmp).map_err(|e| format!("写入待恢复配置失败: {e}"))?;
        std::fs::rename(&tmp, &pending_cfg).map_err(|e| format!("落位待恢复配置失败: {e}"))?;
    }
    // 附件（v2 备份）：staging → attachments.pending（目录级替换，重启落位）
    let staged_att = staging.join("attachments");
    if staged_att.is_dir() {
        std::fs::rename(&staged_att, &pending_att)
            .map_err(|e| format!("写入待恢复附件失败: {e}"))?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    Ok(summary)
}

/// 清理上一次被中断的恢复残留：`attachments.pending` + 数据库/配置的 `.pending`（及写入用的 `.tmp`）。
/// 独立成函数便于单测——这里的行为曾在 2026-09-17 被修正过：旧实现把清理写在
/// 「本次备份含附件」的分支里，导致**无附件备份的恢复不清残渣**，
/// 残留目录会在下一次启动被 `apply_pending_restore` 误落位。
/// v2026-09-23 扩展：数据库/配置的 `.pending` 此前无人清理，残留会在下次启动被误落位。
fn clear_stale_pending(paths: &voucher_core::utils::paths::AppPaths) {
    let pending_att = paths.attachments.with_extension("pending");
    let _ = std::fs::remove_dir_all(&pending_att);
    for p in [
        paths.base.join("voucher.db.pending"),
        paths.base.join("voucher.db.pending.tmp"),
        paths.config_dir.join("app.json.pending"),
        paths.config_dir.join("app.json.pending.tmp"),
    ] {
        let _ = std::fs::remove_file(&p);
    }
}

/// 应用 macOS 窗口材质（v0.18）：
///
/// **优先 Liquid Glass**（macOS 26+ 的 `NSGlassEffectView`，即 macOS 27 当前的原生玻璃），
/// 失败（系统过旧 / 系统拒绝）则**回退**到 `tauri.conf.json` 里配置的 `windowEffects`
/// （`NSVisualEffectView`，Spike M 已验证可用）。
///
/// `.interactive(true)` 需要 **macOS 27+**：玻璃会随点击产生轻微回弹（crate 在更旧系统上
/// 静默忽略该选项）。透明窗口与材质的前置条件见 `tauri.conf.json`（`transparent` +
/// `macOSPrivateApi`）与设计正本 §1.4。
pub fn apply_macos_material(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_liquid_glass, LiquidGlassOptions, NSGlassEffectViewStyle};
        let opts = LiquidGlassOptions::new(NSGlassEffectViewStyle::Clear)
            .radius(12.0)
            .opaque(false)
            .interactive(true);
        match apply_liquid_glass(window, opts) {
            Ok(()) => log::info!("窗口材质：已应用 macOS Liquid Glass（NSGlassEffectView）"),
            Err(e) => log::warn!(
                "窗口材质：Liquid Glass 不可用（需 macOS 26+），沿用 windowEffects 的 VisualEffect 材质：{e}"
            ),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
    }
}

#[cfg(test)]
mod tests {
    use voucher_core::utils::paths::AppPaths;

    /// 陈旧 attachments.pending 必须被清理，且**不得**碰真实 attachments 目录
    #[test]
    fn clear_stale_pending_removes_only_pending_dir() {
        let base = std::env::temp_dir().join(format!("vs-pending-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let paths = AppPaths::resolve(Some(base.clone())).expect("解析路径");
        paths.ensure_all().expect("建目录");

        // 造「真实附件目录 + 陈旧 pending」现场
        let att = paths.attachments.join("2026").join("08");
        std::fs::create_dir_all(&att).expect("建附件子目录");
        std::fs::write(att.join("keep.jpg"), b"original").expect("写原图");
        let stale = paths.attachments.with_extension("pending");
        std::fs::create_dir_all(stale.join("2026").join("08")).expect("建 pending");
        std::fs::write(stale.join("2026").join("08").join("stale.jpg"), b"stale")
            .expect("写 pending 内容");

        super::clear_stale_pending(&paths);

        assert!(!stale.exists(), "陈旧 attachments.pending 应被清理");
        assert!(att.join("keep.jpg").is_file(), "真实附件目录不得被触碰");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 2026-09-23 缺陷锁（审计 B1）：半截/损坏的 `voucher.db.pending` 不得落位，
    /// 活库必须原样保留。旧实现是「先删活库再改名」——半截库会直接进来（整库丢失）
    /// 且失败无任何报错，用户看到的是「界面还在、凭证全没了」。
    #[test]
    fn corrupt_pending_db_is_rejected_and_live_db_survives() {
        let base = std::env::temp_dir().join(format!("vs-restore-reject-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let paths = AppPaths::resolve(Some(base.clone())).expect("解析路径");
        paths.ensure_all().expect("建目录");

        // 活库：内容随意（落位逻辑不读它），只要存在即可
        std::fs::write(paths.db_file(), b"live-db-content").expect("写活库");
        // 半截 / 根本不是库的 pending
        std::fs::write(paths.base.join("voucher.db.pending"), b"not-a-sqlite-file")
            .expect("写坏 pending");

        super::apply_pending_restore(&paths);

        assert_eq!(
            std::fs::read(paths.db_file()).expect("活库应仍在"),
            b"live-db-content",
            "校验不通过时不得覆盖活库"
        );
        assert!(
            paths.base.join("voucher.db.pending").is_file(),
            "未落位的待恢复文件应留在原地供排查"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 正向：合法 SQLite 的 `voucher.db.pending` 能正常落位，且不留旧库残渣。
    #[test]
    fn valid_pending_db_is_applied() {
        let base = std::env::temp_dir().join(format!("vs-restore-apply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let paths = AppPaths::resolve(Some(base.clone())).expect("解析路径");
        paths.ensure_all().expect("建目录");

        // 用 core 造一个真实 SQLite 库，充当「备份里的库」
        let src_dir = base.join("src-db");
        std::fs::create_dir_all(&src_dir).expect("建源目录");
        {
            let ctx = voucher_core::CoreContext::init(Some(src_dir.clone())).expect("建库");
            drop(ctx);
        }
        std::fs::copy(
            src_dir.join("voucher.db"),
            paths.base.join("voucher.db.pending"),
        )
        .expect("放置 pending");
        std::fs::write(paths.db_file(), b"old-live").expect("写旧活库");

        super::apply_pending_restore(&paths);

        assert!(
            !paths.base.join("voucher.db.pending").exists(),
            "落位后 pending 应被消费"
        );
        assert!(
            !paths.base.join("voucher.db.old-restore").exists(),
            "落位成功后不留旧库备份"
        );
        assert_ne!(
            std::fs::read(paths.db_file()).expect("读落位后的库"),
            b"old-live",
            "活库应已被替换为恢复的库"
        );
        assert!(
            voucher_core::services::backup::validate_db_file(&paths.db_file()).is_ok(),
            "落位后的库必须是可用的 SQLite 库"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
