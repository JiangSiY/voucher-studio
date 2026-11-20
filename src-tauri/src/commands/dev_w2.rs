//! 开发态 W2 Mock 命令（只在显式隔离开关下可用；不接真实 WPS）。

use super::*;

fn conv<T>(r: std::result::Result<T, voucher_api::ApiError>) -> std::result::Result<T, ApiError> {
    r.map_err(|e| ApiError::plain(e.code, e.message))
}

/// 从界面发起一次本地合成拉取/模拟创建。
///
/// 守卫在 `voucher_api::dev_w2::run`：debug 构建 + `VOUCHER_W2_MOCK=1` + 临时
/// `VOUCHER_DATA_DIR`；普通运行、生产目录或网页端会收到 `dev_only`。
#[tauri::command]
pub fn dev_w2_mock_sync(
    ctx: Ctx,
    action: Option<String>,
    scenario: Option<String>,
) -> std::result::Result<voucher_api::dev_w2::DevW2Result, ApiError> {
    conv(voucher_api::dev_w2::run(
        &ctx,
        action.as_deref(),
        scenario.as_deref(),
    ))
}
