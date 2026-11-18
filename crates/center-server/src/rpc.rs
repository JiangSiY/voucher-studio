//! RPC：`POST /api/cmd/{cmd}` → `voucher_api::dispatch`。
//!
//! 与桌面版的区别只有一处：这里没有 Tauri 的 `State`，参数从 HTTP body 来，
//! 业务函数本身完全相同（同一个 `voucher-api` crate），所以两边行为天然一致。

use std::sync::Arc;

use serde_json::{json, Value};

use crate::state::{ApiErr, AppState};

/// 同步执行一条命令。调用方负责放到 `spawn_blocking` 里（内部会拿数据库锁）。
pub fn run(st: &Arc<AppState>, cmd: &str, args: Value) -> Result<Value, ApiErr> {
    if cmd.is_empty() || cmd.len() > 64 {
        return Err(ApiErr::bad_request("invalid", "命令名不合法"));
    }
    voucher_api::dispatch(&st.ctx, cmd, args).map_err(ApiErr::from)
}

/// `GET /api/capabilities`：让前端知道这台机器有哪些能力（本地 OCR / PDF 字体等）
pub fn capabilities(st: &Arc<AppState>) -> Value {
    json!({
        "server": true,
        "localOcr": cfg!(target_os = "macos"),
        "platform": std::env::consts::OS,
        "version": env!("CARGO_PKG_VERSION"),
        "dataDir": st.ctx.paths.base.to_string_lossy(),
    })
}
