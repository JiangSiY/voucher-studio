//! Center 服务端入口。
//!
//! 部署形态（2026-11 确定）：Mac 常驻 + Tailscale 内网，前端是浏览器 / PWA。
//! 启动方式：
//! ```bash
//! CENTER_PASSWORD='至少8位' target/release/center-server
//! tailscale serve --bg --https=443 http://127.0.0.1:8787
//! ```
//! 环境变量：
//! - `CENTER_ADDR`           监听地址，默认 `127.0.0.1:8787`
//! - `CENTER_DATA_DIR`       数据目录，默认 `~/Library/Application Support/VoucherStudio`
//! - `CENTER_WEB_DIR`        前端产物目录，默认取可执行文件旁的 `web/`，再退回 `dist/`
//! - `CENTER_PASSWORD`       首次启动时创建访问密码（库内已有用户则忽略）
//! - `CENTER_COOKIE_SECURE`  设为 1 时给会话 Cookie 加 Secure（走 https 时必须开）
//! - `CENTER_LOG`            日志级别，默认 `info`

mod auth;
mod files;
mod http;
mod rpc;
mod state;
mod upload;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use voucher_core::CoreContext;

use state::AppState;

struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        eprintln!("[{ts}] {:<5} {}", record.level(), record.args());
    }

    fn flush(&self) {}
}

fn init_logging() {
    let level = std::env::var("CENTER_LOG").unwrap_or_else(|_| "info".into());
    let level = match level.to_ascii_lowercase().as_str() {
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "debug" => log::LevelFilter::Debug,
        "trace" => log::LevelFilter::Trace,
        _ => log::LevelFilter::Info,
    };
    let logger = Box::leak(Box::new(StderrLogger));
    // 初始化失败（例如测试里重复初始化）不应让服务起不来
    let _ = log::set_logger(logger).map(|()| log::set_max_level(level));
}

fn resolve_web_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CENTER_WEB_DIR") {
        let p = PathBuf::from(dir);
        if p.join("index.html").is_file() {
            return p;
        }
        log::warn!("CENTER_WEB_DIR={} 下没有 index.html", p.display());
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let candidates = [
        exe_dir.join("web"),
        exe_dir.join("../web"),
        PathBuf::from("dist"),
        PathBuf::from("../dist"),
        PathBuf::from("../../dist"),
    ];
    for c in candidates {
        if c.join("index.html").is_file() {
            return c;
        }
    }
    PathBuf::from("dist")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let data_dir = std::env::var("CENTER_DATA_DIR").ok().map(PathBuf::from);
    // 服务端是无 GUI 的常驻进程：绝不能走 Keychain（读一次就卡死，见 crypto.rs 注释）。
    // 在 init 之前把凭据后端指到数据目录里的 0600 文件。
    if std::env::var("CENTER_SECRETS_FILE").is_err() {
        let paths = voucher_core::utils::paths::AppPaths::resolve(data_dir.clone())?;
        std::env::set_var("CENTER_SECRETS_FILE", paths.config_dir.join("secrets.json"));
    }
    let ctx = CoreContext::init(data_dir)?;
    log::info!("数据目录：{}", ctx.paths.base.display());

    if let Ok(pw) = std::env::var("CENTER_PASSWORD") {
        auth::bootstrap_password(&ctx, &pw)?;
    }

    let web_dir = resolve_web_dir();
    let cookie_secure = matches!(
        std::env::var("CENTER_COOKIE_SECURE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    );
    let state = Arc::new(AppState {
        ctx: Arc::new(ctx),
        web_dir,
        cookie_secure,
    });
    log::info!("前端目录：{}", state.web_dir.display());
    if !state.web_dir.join("index.html").is_file() {
        log::warn!("前端未构建：先执行 npm run build，或设置 CENTER_WEB_DIR");
    }

    let addr: SocketAddr = std::env::var("CENTER_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8787".into())
        .parse()?;

    let server = tokio::spawn(http::serve(state, addr));
    tokio::select! {
        r = server => {
            if let Err(e) = r {
                log::error!("服务线程异常：{e}");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            log::info!("收到 Ctrl-C，Center 服务端退出");
        }
    }
    Ok(())
}
