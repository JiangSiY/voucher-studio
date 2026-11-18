//! 设置域命令（与 `src-tauri/src/commands/settings.rs` 对应）。
//!
//! 密钥仍走系统凭据库（macOS Keychain）：服务端在本机跑，钥匙串可用；
//! 这也是「网页版但密钥不出机器」的关键——浏览器永远拿不到 API Key。

use voucher_core::services::naming;
use voucher_core::services::query as qsvc;
use voucher_core::utils::config::{
    AppConfig, SECRET_BACKUP_PASSWORD, SECRET_OCR_API_KEY, SECRET_WEBDAV_PASSWORD,
};
use voucher_core::utils::crypto::SecretStatus;

use crate::{core, ApiError, ApiResult, CoreContext};

pub fn get_config(ctx: &CoreContext) -> AppConfig {
    ctx.config()
}

pub fn save_config(ctx: &CoreContext, mut config: AppConfig) -> ApiResult<AppConfig> {
    let validated =
        naming::NamingEngine::validate(&config.naming.template).map_err(ApiError::from_core)?;
    config.naming.template = validated;
    core(ctx.set_config(config))?;
    Ok(ctx.config())
}

pub fn validate_naming_template(template: String) -> ApiResult<String> {
    naming::NamingEngine::validate(&template).map_err(ApiError::from_core)
}

pub fn preview_voucher_no(ctx: &CoreContext, date: String) -> ApiResult<String> {
    core(qsvc::next_voucher_no(ctx, &date))
}

pub fn list_secrets(ctx: &CoreContext) -> Vec<SecretStatus> {
    ctx.secrets.inspect()
}

pub fn set_secret(ctx: &CoreContext, alias: String, value: String) -> ApiResult<()> {
    let known = [
        SECRET_OCR_API_KEY,
        SECRET_WEBDAV_PASSWORD,
        SECRET_BACKUP_PASSWORD,
    ];
    if !known.contains(&alias.as_str()) {
        return Err(ApiError::plain("invalid", format!("未知凭据别名: {alias}")));
    }
    core(ctx.secrets.set(&alias, &value))
}

pub fn delete_secret(ctx: &CoreContext, alias: String) -> ApiResult<bool> {
    core(ctx.secrets.delete(&alias))
}
