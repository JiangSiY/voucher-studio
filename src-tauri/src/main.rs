#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Voucher Studio 桌面应用入口（Tauri 壳）

mod commands;
mod platform;

use std::sync::Arc;

use tauri::{Emitter, Manager};
use voucher_core::utils::file_lock::FileLockGuard;
use voucher_core::utils::paths::AppPaths;
use voucher_core::CoreContext;

/// 持有数据目录文件锁（进程存活期间不释放；字段仅在 Drop 时起作用）
#[allow(dead_code)]
struct DataLock(FileLockGuard);

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            // 1. 落位待恢复备份（restore 命令重启生效机制）
            let paths = AppPaths::resolve(None)?;
            paths.ensure_all()?;
            platform::apply_pending_restore(&paths);

            // 2. 数据目录文件锁：同一目录同一时刻只允许一个实例写入
            let guard = FileLockGuard::try_acquire(&paths.lock_file()).map_err(|e| {
                Box::<dyn std::error::Error>::from(format!(
                    "{e}。若确认没有其他实例在运行，可删除 {} 后重试。",
                    paths.lock_file().display()
                ))
            })?;
            app.manage(DataLock(guard));

            // 3. 核心上下文（打开数据库、建表、加载配置）
            let ctx = Arc::new(CoreContext::init(None)?);
            app.manage(ctx.clone());

            // 3.5 本地 OCR 模型：首次启动从安装源复制到数据目录（幂等）
            for src in voucher_core::services::ocr_local::default_model_sources() {
                match voucher_core::LocalOcr::ensure_installed(&src, &ctx.paths.models) {
                    Ok(true) => {
                        log::info!("本地 OCR 模型已从 {} 安装", src.display());
                        break;
                    }
                    Ok(false) => {
                        if voucher_core::LocalOcr::is_installed(&ctx.paths.models) {
                            break;
                        }
                    }
                    Err(e) => log::warn!("本地 OCR 模型安装失败（{}）: {e}", src.display()),
                }
            }

            // 4. 微信监听 + 定时备份调度（配置启用时生效；监听支持保存配置后热重载）
            platform::sync_wechat_watch(app.handle().clone(), &ctx);
            platform::spawn_backup_scheduler(app.handle().clone(), ctx.clone());

            // 5. 崩溃恢复：标记会话运行中 + 健康检查（只读），报告缓存供前端读取
            let report = voucher_core::services::recovery::check_recovery(&ctx)?;
            if report.abnormal_exit {
                if let Some(last) = &report.last_session {
                    log::warn!(
                        "检测到上次会话异常退出（开始于 {}，PID {}），已执行数据健康检查",
                        last.started_at,
                        last.pid
                    );
                }
                if !report.db_ok {
                    log::error!("数据库完整性检查未通过：{:?}", report.integrity_errors);
                }
                if report.attachment_missing_count > 0 {
                    log::error!(
                        "{} 张附件物理文件缺失（样本：{:?}）",
                        report.attachment_missing_count,
                        report.attachment_missing
                    );
                }
            }
            app.manage(report);

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                // 正常关闭窗口 → 清除会话 marker；崩溃/强杀不会走到这里，下次启动即触发恢复检查
                tauri::WindowEvent::CloseRequested { .. } => {
                    if let Some(ctx) = window.try_state::<Arc<CoreContext>>() {
                        let _ = voucher_core::services::recovery::mark_clean_exit(&ctx);
                    }
                }
                // 拖放导入（Rust 侧直接处理，绕开 WebView JS 事件桥在透明窗口下不稳定的已知问题）。
                // 语义：任何页面拖入图片 = 新建凭证导入（一图一凭证）。
                tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Enter { .. }) => {
                    let _ = window.app_handle().emit("import:drag-enter", ());
                }
                tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Leave) => {
                    let _ = window.app_handle().emit("import:drag-leave", ());
                }
                tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                    let images: Vec<std::path::PathBuf> = paths
                        .iter()
                        .filter(|p| voucher_core::services::imaging::is_supported(p))
                        .cloned()
                        .collect();
                    if images.is_empty() {
                        return;
                    }
                    let Some(ctx) = window.try_state::<Arc<CoreContext>>() else {
                        return;
                    };
                    let ctx: Arc<CoreContext> = ctx.inner().clone();
                    let app = window.app_handle().clone();
                    log::info!("拖放导入：{} 张图片", images.len());
                    let _ = app.emit("import:started", images.len());
                    tauri::async_runtime::spawn_blocking(move || {
                        let started = std::time::Instant::now();
                        match voucher_core::services::ingestion::import_images(
                            &ctx, &images, None, true,
                        ) {
                            Ok(summary) => {
                                log::info!(
                                    "拖放导入完成：新建 {} 张，跳过重复 {}，失败 {}，耗时 {:?}",
                                    summary.vouchers.len(),
                                    summary.skipped_duplicates.len(),
                                    summary.failed.len(),
                                    started.elapsed()
                                );
                                let _ = app.emit("import:done", &summary);
                            }
                            Err(e) => {
                                log::error!("拖放导入失败：{e}");
                                let _ = app.emit("import:error", e.to_string());
                            }
                        }
                    });
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::open_path,
            commands::get_recovery_report,
            commands::restart_app,
            commands::get_config,
            commands::save_config,
            commands::get_wechat_status,
            commands::validate_naming_template,
            commands::preview_voucher_no,
            commands::list_secrets,
            commands::set_secret,
            commands::delete_secret,
            commands::list_vouchers,
            commands::get_voucher,
            commands::update_voucher,
            commands::delete_vouchers,
            commands::voucher_stats,
            commands::monthly_summary,
            commands::search_vouchers,
            commands::pick_and_import,
            commands::pick_and_replace_image,
            commands::pick_directory,
            commands::attachment_abs_path,
            commands::run_ocr,
            commands::run_ocr_pending,
            commands::local_ocr_status,
            commands::export_pdf,
            commands::export_pdf_daily,
            commands::list_export_batches,
            commands::run_backup,
            commands::list_backups,
            commands::inspect_backup,
            commands::restore_backup,
            commands::drill_backup,
            commands::upload_backup,
            commands::enhance_status,
            commands::clear_enhance_cache,
            commands::enhance_status_v2,
            commands::clear_enhance_cache_v2,
            commands::delete_enhanced_one,
            commands::recent_enhanced,
            commands::record_viewer_hit,
        ])
        .run(tauri::generate_context!())
        .expect("Voucher Studio 启动失败");
}
