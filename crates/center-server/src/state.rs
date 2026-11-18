//! 服务端共享状态与统一错误体。

use std::path::PathBuf;
use std::sync::Arc;

use voucher_core::CoreContext;

/// 全局状态：业务上下文 + 前端静态目录 + 是否给 Cookie 加 Secure
#[derive(Clone)]
pub struct AppState {
    pub ctx: Arc<CoreContext>,
    pub web_dir: PathBuf,
    /// 经 tailscale serve（HTTPS）访问时应为 true；本机 http 调试为 false
    pub cookie_secure: bool,
}

/// HTTP 错误：与桌面版 `ApiError` 同形 `{ "error": { code, message, fields? } }`，
/// 前端 `client.ts` 的错误处理不用改；`fields` 只在 `required_missing` 时出现。
#[derive(Debug)]
pub struct ApiErr {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub fields: Option<Vec<String>>,
}

impl ApiErr {
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.to_string(),
            message: message.into(),
            fields: None,
        }
    }

    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(401, "unauthorized", msg)
    }

    pub fn bad_request(code: &str, msg: impl Into<String>) -> Self {
        Self::new(400, code, msg)
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(500, "internal", msg)
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(404, "not_found", msg)
    }

    pub fn body(&self) -> serde_json::Value {
        let mut err = serde_json::json!({ "code": self.code, "message": self.message });
        if let Some(f) = &self.fields {
            err["fields"] = serde_json::json!(f);
        }
        serde_json::json!({ "error": err })
    }
}

impl From<voucher_api::ApiError> for ApiErr {
    fn from(e: voucher_api::ApiError) -> Self {
        let status = match e.code {
            "unauthorized" => 401,
            "not_found" => 404,
            _ => 400,
        };
        Self {
            status,
            code: e.code.to_string(),
            message: e.message,
            fields: e.fields,
        }
    }
}

/// 当前时间（本地时区 ISO8601，与库内时间同口径）
pub fn now_iso() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string()
}
