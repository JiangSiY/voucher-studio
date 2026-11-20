//! WPS 专用凭据状态（只读；不返回 token 明文）。

use super::*;

/// 返回系统钥匙串中 WPS 专用 service 的 `token_set` / `token_expires_at`。
#[tauri::command]
pub fn wps_credentials_status(
) -> std::result::Result<voucher_core::services::wps_credentials::WpsTokenStatus, ApiError> {
    voucher_api::wps_credentials::status().map_err(|e| ApiError::plain(e.code, e.message))
}
