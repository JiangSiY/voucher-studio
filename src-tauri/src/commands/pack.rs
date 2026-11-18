//! 汇总册命令（v0.26 M3）：图片册 / 发票册 / PDF 合并。

use super::{ApiError, Ctx};
use voucher_api::pack as pack_api;

/// 桌面端命令返回的错误类型（与网页端 `ApiError` 字段一致）
type ApiResult<T> = std::result::Result<T, ApiError>;

/// 生成汇总册；返回落盘路径与入册张数
#[tauri::command]
pub async fn pack_build(
    ctx: Ctx<'_>,
    voucher_ids: Vec<i64>,
    kind: String,
) -> ApiResult<voucher_core::services::pack::PackResult> {
    let core = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        pack_api::pack_build(&core, voucher_ids, kind).map_err(|e| ApiError {
            code: e.code,
            message: e.message,
            fields: e.fields,
        })
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
}
