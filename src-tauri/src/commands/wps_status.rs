//! WPS 同步状态（只读；不构造客户端、不读凭据、不发网络请求）。

use super::*;

/// 返回最近同步/扫描时刻与待处理队列（最多 20 条，只给错误码与状态，不回显字段值）。
///
/// 只读本地库，不加 `dev` 前置；命令体不接受 file_id / 数据目录参数。
#[tauri::command]
pub fn wps_sync_status(
    ctx: Ctx,
) -> std::result::Result<voucher_api::wps_status::WpsSyncStatus, ApiError> {
    voucher_api::wps_status::wps_sync_status(&ctx).map_err(|e| ApiError::plain(e.code, e.message))
}
