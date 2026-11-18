//! HTTP 层：路由 + 请求解析 + 响应组装（hyper 1.x，手写路由）。
//!
//! 路由表：
//! ```text
//! GET  /api/health            健康检查（不需要登录）
//! GET  /api/auth/status       是否需要初始化 / 是否已登录
//! POST /api/auth/setup        首次设置访问密码
//! POST /api/auth/login        登录
//! POST /api/auth/logout       退出登录
//! GET  /api/capabilities      能力探测（本地 OCR / 平台 / 版本）
//! POST /api/cmd/{cmd}         业务命令（等价桌面版 invoke）
//! POST /api/upload            multipart 上传导入
//! GET  /api/asset?path=       图片/PDF 资源（附件预览）
//! GET  /api/download?path=    下载（导出批次、备份包）
//! GET  /*                     前端静态文件（SPA fallback）
//! ```

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, CONTENT_TYPE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::auth;
use crate::files;
use crate::rpc;
use crate::state::{ApiErr, AppState};
use crate::upload;

type Resp = Response<Full<Bytes>>;

pub async fn serve(state: Arc<AppState>, addr: SocketAddr) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    log::info!("Center 服务端已监听 http://{addr}");
    loop {
        let (stream, peer) = listener.accept().await?;
        let st = state.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let svc = service_fn(move |req: Request<Incoming>| {
                let st = st.clone();
                async move { Ok::<_, Infallible>(route(req, st).await) }
            });
            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                log::debug!("{peer} 连接结束：{e}");
            }
        });
    }
}

async fn route(req: Request<Incoming>, st: Arc<AppState>) -> Resp {
    match handle(req, st).await {
        Ok(r) => r,
        Err(e) => {
            if e.status >= 500 {
                log::error!("{} {} → {}", e.code, e.message, e.status);
            }
            json_resp(e.status, e.body(), vec![])
        }
    }
}

async fn handle(req: Request<Incoming>, st: Arc<AppState>) -> Result<Resp, ApiErr> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let headers = req.headers().clone();

    if path.starts_with("/api/") {
        return api(method, &path, &query, headers, req, st).await;
    }

    if method != Method::GET && method != Method::HEAD {
        return Err(ApiErr::new(405, "method_not_allowed", "只支持 GET"));
    }
    match files::read_spa(&st, &path).await {
        Some((bytes, mime, immutable)) => {
            let cache = if immutable {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            Ok(bytes_resp(
                200,
                mime,
                bytes,
                vec![("cache-control".into(), cache.into())],
            ))
        }
        None => Err(ApiErr::not_found(format!(
            "找不到 {path}（前端未构建请先 npm run build）"
        ))),
    }
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        if k != key {
            return None;
        }
        Some(percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

async fn read_body(req: Request<Incoming>, limit: usize) -> Result<Vec<u8>, ApiErr> {
    let body = Limited::new(req.into_body(), limit);
    let collected = body
        .collect()
        .await
        .map_err(|e| ApiErr::new(413, "too_large", format!("请求体过大或读取失败：{e}")))?;
    Ok(collected.to_bytes().to_vec())
}

fn require_user(st: &Arc<AppState>, headers: &hyper::HeaderMap) -> Result<i64, ApiErr> {
    auth::current_user(&st.ctx, headers)
        .ok_or_else(|| ApiErr::unauthorized("未登录，请先输入访问密码"))
}

async fn api(
    method: Method,
    path: &str,
    query: &str,
    headers: hyper::HeaderMap,
    req: Request<Incoming>,
    st: Arc<AppState>,
) -> Result<Resp, ApiErr> {
    match (method.as_str(), path) {
        ("GET", "/api/health") => Ok(json_resp(
            200,
            json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }),
            vec![],
        )),
        ("GET", "/api/auth/status") => {
            let st2 = st.clone();
            let h2 = headers.clone();
            let body = tokio::task::spawn_blocking(move || auth::status(&st2.ctx, &h2))
                .await
                .map_err(|e| ApiErr::internal(format!("状态查询线程异常：{e}")))?;
            Ok(json_resp(200, body, vec![]))
        }
        ("POST", "/api/auth/setup") | ("POST", "/api/auth/login") => {
            let raw = read_body(req, 64 * 1024).await?;
            let parsed: auth::PasswordBody = serde_json::from_slice(&raw)
                .map_err(|_| ApiErr::bad_request("invalid", "请求体应为 {\"password\":\"...\"}"))?;
            let is_setup = path.ends_with("setup");
            let st2 = st.clone();
            let h2 = headers.clone();
            let reply = tokio::task::spawn_blocking(move || {
                if is_setup {
                    auth::setup(&st2, &h2, &parsed)
                } else {
                    auth::login(&st2, &h2, &parsed)
                }
            })
            .await
            .map_err(|e| ApiErr::internal(format!("认证线程异常：{e}")))??;
            let extra = reply.headers.into_iter().collect::<Vec<_>>();
            Ok(json_resp(200, reply.body, extra))
        }
        ("POST", "/api/auth/logout") => {
            let st2 = st.clone();
            let h2 = headers.clone();
            let reply = tokio::task::spawn_blocking(move || auth::logout(&st2, &h2))
                .await
                .map_err(|e| ApiErr::internal(format!("退出线程异常：{e}")))?;
            Ok(json_resp(200, reply.body, reply.headers))
        }
        ("GET", "/api/capabilities") => {
            require_user(&st, &headers)?;
            Ok(json_resp(200, rpc::capabilities(&st), vec![]))
        }
        ("POST", cmd_path) if cmd_path.starts_with("/api/cmd/") => {
            require_user(&st, &headers)?;
            let cmd = cmd_path.trim_start_matches("/api/cmd/").to_string();
            let raw = read_body(req, 32 * 1024 * 1024).await?;
            let args: Value = if raw.is_empty() {
                json!({})
            } else {
                serde_json::from_slice(&raw)
                    .map_err(|_| ApiErr::bad_request("invalid", "命令参数不是合法 JSON"))?
            };
            let st2 = st.clone();
            let out = tokio::task::spawn_blocking(move || rpc::run(&st2, &cmd, args))
                .await
                .map_err(|e| ApiErr::internal(format!("命令线程异常：{e}")))??;
            Ok(json_resp(200, out, vec![]))
        }
        ("POST", "/api/upload") => {
            require_user(&st, &headers)?;
            let ct = headers
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let raw = read_body(req, upload::MAX_BODY).await?;
            let parts = upload::parse_multipart(&ct, &raw)
                .map_err(|e| ApiErr::bad_request("invalid", e))?;
            let st2 = st.clone();
            let out = tokio::task::spawn_blocking(move || -> Result<Value, ApiErr> {
                let staged = upload::stage_files(&st2, &parts)?;
                let result = upload::run_staged(&st2, &staged);
                // stage 模式的文件要继续被后续命令引用（台账 CSV 预演→写入），不能清
                if staged.mode != "stage" {
                    upload::cleanup(&staged.paths);
                }
                result
            })
            .await
            .map_err(|e| ApiErr::internal(format!("导入线程异常：{e}")))??;
            Ok(json_resp(200, out, vec![]))
        }
        ("GET", "/api/asset") => {
            require_user(&st, &headers)?;
            let raw = query_param(query, "path")
                .ok_or_else(|| ApiErr::bad_request("invalid", "缺少 path 参数"))?;
            let p = files::resolve_in_data_dir(&st, &raw)?;
            let bytes = tokio::fs::read(&p)
                .await
                .map_err(|e| ApiErr::not_found(format!("读取失败：{e}")))?;
            let mime = files::mime_of(&p);
            Ok(bytes_resp(
                200,
                mime,
                bytes,
                vec![("cache-control".into(), "private, max-age=3600".into())],
            ))
        }
        ("GET", "/api/download") => {
            require_user(&st, &headers)?;
            let raw = query_param(query, "path")
                .ok_or_else(|| ApiErr::bad_request("invalid", "缺少 path 参数"))?;
            let p = files::resolve_in_data_dir(&st, &raw)?;
            let bytes = tokio::fs::read(&p)
                .await
                .map_err(|e| ApiErr::not_found(format!("读取失败：{e}")))?;
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "download".into());
            let mime = files::mime_of(&p);
            // 文件名含中文时必须用 filename*，否则 Chrome 会乱码
            let encoded = percent_encode(&name);
            Ok(bytes_resp(
                200,
                mime,
                bytes,
                vec![(
                    "content-disposition".into(),
                    format!(
                        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
                        encoded, encoded
                    ),
                )],
            ))
        }
        _ => Err(ApiErr::not_found(format!("未知接口：{path}"))),
    }
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn json_resp(status: u16, body: Value, extra: Vec<(String, String)>) -> Resp {
    let bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    bytes_resp(status, "application/json; charset=utf-8", bytes, extra)
}

fn bytes_resp(status: u16, mime: &str, bytes: Vec<u8>, extra: Vec<(String, String)>) -> Resp {
    let mut builder = Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));
    builder = builder.header(
        CONTENT_TYPE,
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    for (k, v) in extra {
        if let (Ok(name), Ok(val)) = (
            hyper::header::HeaderName::try_from(k.as_str()),
            HeaderValue::from_str(&v),
        ) {
            builder = builder.header(name, val);
        }
    }
    builder
        .body(Full::new(Bytes::from(bytes)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::from_static(b"{}"))))
}
