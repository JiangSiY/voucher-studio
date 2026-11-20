//! WPS 凭据状态接口：只暴露 `token_set` / `token_expires_at`，不提供读回明文的命令。

use voucher_core::services::wps_credentials::{WpsCredentialStore, WpsTokenStatus};

use crate::{ApiError, ApiResult};

pub fn status() -> ApiResult<WpsTokenStatus> {
    WpsCredentialStore::new()
        .status()
        .map_err(ApiError::from_core)
}
