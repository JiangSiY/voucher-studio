//! 设置域命令：配置 / 密钥（系统凭据库）/ 增强缓存管理。
//!
//! 越层约束（体检 #11）：编号预览经 `services::query` 门面取数，
//! 命令层不直连仓储层或裸数据库连接（CI grep 门禁固化）。

use super::*;
use voucher_core::services::naming;
use voucher_core::services::query as qsvc;
use voucher_core::utils::config::{
    AppConfig, SECRET_BACKUP_PASSWORD, SECRET_OCR_API_KEY, SECRET_WEBDAV_PASSWORD,
};
// ---------- 配置 ----------

#[tauri::command]
pub fn get_config(ctx: Ctx) -> AppConfig {
    ctx.config()
}

#[tauri::command]
pub fn save_config(ctx: Ctx, config: AppConfig) -> std::result::Result<AppConfig, ApiError> {
    // 命名模板先校验再落盘
    let validated =
        naming::NamingEngine::validate(&config.naming.template).map_err(ApiError::from_core)?;
    let mut config = config;
    config.naming.template = validated;
    err_api(ctx.set_config(config))?;
    Ok(ctx.config())
}

#[tauri::command]
pub fn validate_naming_template(template: String) -> std::result::Result<String, ApiError> {
    naming::NamingEngine::validate(&template).map_err(ApiError::from_core)
}

#[tauri::command]
pub fn preview_voucher_no(ctx: Ctx, date: String) -> std::result::Result<String, ApiError> {
    err_api(qsvc::next_voucher_no(&ctx, &date))
}

// ---------- 密钥管理（系统凭据库） ----------

#[tauri::command]
pub fn list_secrets(ctx: Ctx) -> Vec<voucher_core::utils::crypto::SecretStatus> {
    ctx.secrets.inspect()
}

#[tauri::command]
pub fn set_secret(ctx: Ctx, alias: String, value: String) -> std::result::Result<(), ApiError> {
    let known = [
        SECRET_OCR_API_KEY,
        SECRET_WEBDAV_PASSWORD,
        SECRET_BACKUP_PASSWORD,
    ];
    if !known.contains(&alias.as_str()) {
        return Err(ApiError::plain("invalid", format!("未知凭据别名: {alias}")));
    }
    err_api(ctx.secrets.set(&alias, &value))
}

#[tauri::command]
pub fn delete_secret(ctx: Ctx, alias: String) -> std::result::Result<bool, ApiError> {
    err_api(ctx.secrets.delete(&alias))
}
