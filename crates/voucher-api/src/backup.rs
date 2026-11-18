//! 备份 / 恢复 / WebDAV 命令（与 `src-tauri/src/commands/backup.rs` 对应）。
//!
//! 服务端与桌面版唯一的语义差异在「恢复」：桌面版把待恢复整包交给壳层、重启后生效；
//! 服务端没有重启按钮，所以写一个 `restore.pending` 标记文件——服务启动时（M1 尾）
//! 检查它并应用，界面上的「恢复」因此返回「已排队，重启服务后生效」。

use std::path::Path;

use voucher_core::services::backup::{
    self, BackupInfo, DrillReport, RestoreSummary, WebdavBackupRow,
};

use crate::{core, ApiError, ApiResult, CoreContext};

pub fn run_backup(ctx: &CoreContext) -> ApiResult<BackupInfo> {
    core(backup::run_backup(ctx))
}

pub fn list_backups(ctx: &CoreContext) -> ApiResult<Vec<BackupInfo>> {
    core(backup::list_backups(ctx))
}

pub fn inspect_backup(ctx: &CoreContext, path: String) -> ApiResult<RestoreSummary> {
    core(backup::inspect_backup(ctx, Path::new(&path)))
}

pub fn export_full_backup(ctx: &CoreContext, path: String) -> ApiResult<String> {
    core(backup::export_full_backup(ctx, Path::new(&path)))
        .map(|p| p.to_string_lossy().into_owned())
}

pub fn drill_backup(ctx: &CoreContext, path: Option<String>) -> ApiResult<DrillReport> {
    core(backup::drill_backup(ctx, path.as_deref().map(Path::new)))
}

/// 恢复：先做完整校验（sha256 全包），通过后写 `restore.pending` 排队，服务重启时应用。
///
/// 为什么不在运行时直接替换：SQLite 连接与附件文件都在用，热替换会撕裂一次请求。
/// 把「校验」与「应用」拆开，用户当场能看到「包是好的」，真正的切换发生在重启。
pub fn restore_backup(ctx: &CoreContext, path: String) -> ApiResult<RestoreSummary> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(ApiError::plain(
            "not_found",
            format!("备份文件不存在：{path}"),
        ));
    }
    let summary = core(backup::inspect_backup(ctx, p))?;
    let marker = ctx.paths.base.join("restore.pending");
    std::fs::write(&marker, path)
        .map_err(|e| ApiError::internal(format!("写入待恢复标记失败：{e}")))?;
    Ok(summary)
}

pub fn upload_backup(ctx: &CoreContext, path: String) -> ApiResult<()> {
    core(backup::upload_backup_to_webdav(ctx, Path::new(&path)))
}

pub fn test_webdav(ctx: &CoreContext) -> ApiResult<u32> {
    core(backup::test_webdav(ctx))
}

pub fn list_webdav_backups(ctx: &CoreContext) -> ApiResult<Vec<WebdavBackupRow>> {
    core(backup::list_webdav_backups(ctx))
}

pub fn download_webdav_backup(ctx: &CoreContext, file_name: String) -> ApiResult<String> {
    core(backup::download_webdav_backup(ctx, &file_name))
}
