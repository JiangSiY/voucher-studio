//! 仅回环地址的 HTTP 测试传输（v0.28 本地边界验证）。
//!
//! 只接受 `127.0.0.1` / `localhost` / `[::1]`，不使用 DNS、不跟随重定向、不启用 TLS。
//! 真实 WPS 端点仍由 `WpsConfig` 的 S0 门禁拒绝；本模块只用于本地伪服务。

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::time::Duration;

use serde_json::{json, Value};

use super::wps::{WpsError, WpsTransport};

/// 回环 HTTP 响应：(状态码, 响应头, 响应体)。
type RawResponse = (u16, Vec<(String, String)>, String);

/// 只允许回环地址的极简 HTTP/1.1 传输。
pub struct LoopbackHttpTransport {
    endpoint: SocketAddr,
    host_header: String,
    path: String,
    timeout: Duration,
}

impl LoopbackHttpTransport {
    /// 接受 `http://127.0.0.1:PORT/mcp`、`127.0.0.1:PORT/mcp` 或 `[::1]:PORT/mcp`。
    /// 拒绝非回环主机、HTTPS、查询串和片段；不解析 DNS。
    pub fn new(endpoint: &str) -> Result<Self, WpsError> {
        let raw = endpoint.trim();
        let raw = raw.strip_prefix("http://").unwrap_or(raw);
        if raw.starts_with("https://") {
            return Err(WpsError::Credentials(
                "回环测试传输不启用 TLS，拒绝 https 端点".into(),
            ));
        }
        let (authority, path) = match raw.find('/') {
            Some(index) => (&raw[..index], &raw[index..]),
            None => (raw, "/mcp"),
        };
        if path.contains('?') || path.contains('#') {
            return Err(WpsError::Credentials(
                "回环测试传输不接受查询串或片段".into(),
            ));
        }
        if path.is_empty() {
            return Err(WpsError::Credentials("回环测试传输缺少路径".into()));
        }
        let (host, port) = parse_authority(authority)?;
        let ip = match host.as_str() {
            "localhost" => IpAddr::V4(Ipv4Addr::LOCALHOST),
            "127.0.0.1" => IpAddr::V4(Ipv4Addr::LOCALHOST),
            "[::1]" | "::1" => IpAddr::V6(Ipv6Addr::LOCALHOST),
            _ => {
                return Err(WpsError::Credentials(
                    "测试传输只允许回环地址（127.0.0.1/localhost/[::1]）".into(),
                ))
            }
        };
        let endpoint = SocketAddr::new(ip, port);
        let host_header = match endpoint.ip() {
            IpAddr::V6(ip) => format!("[{ip}]:{port}"),
            IpAddr::V4(ip) => format!("{ip}:{port}"),
        };
        Ok(Self {
            endpoint,
            host_header,
            path: path.to_string(),
            timeout: Duration::from_secs(5),
        })
    }

    fn send(
        &self,
        method: &str,
        session_id: Option<&str>,
        auth: Option<&str>,
        body: &Value,
    ) -> Result<RawResponse, WpsError> {
        let auth = auth.ok_or_else(|| {
            WpsError::Credentials("回环 HTTP 测试传输要求显式 WPS token，缺失时拒绝请求".into())
        })?;
        if auth.contains('\r') || auth.contains('\n') {
            return Err(WpsError::Credentials(
                "WPS token 含非法控制字符，拒绝请求".into(),
            ));
        }
        let payload = body.to_string();
        let mut request = format!(
            "{method} {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: {}\r\n",
            self.path,
            self.host_header,
            auth,
            payload.len()
        );
        if let Some(session) = session_id {
            request.push_str(&format!("Mcp-Session-Id: {session}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(&payload);

        let mut stream = TcpStream::connect_timeout(&self.endpoint, self.timeout)
            .map_err(|err| WpsError::from_io(&err))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|err| WpsError::from_io(&err))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|err| WpsError::from_io(&err))?;
        stream
            .write_all(request.as_bytes())
            .map_err(|err| WpsError::from_io(&err))?;
        let mut response = Vec::new();
        stream
            .take(1024 * 1024)
            .read_to_end(&mut response)
            .map_err(|err| WpsError::from_io(&err))?;
        parse_http_response(&response)
    }
}

impl WpsTransport for LoopbackHttpTransport {
    fn initialize(&self, auth: Option<&str>) -> Result<String, WpsError> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "voucher-studio-loopback", "version": "0.28-test"}
            }
        });
        let (status, headers, response) = self.send("POST", None, auth, &body)?;
        check_status(status)?;
        let session = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("mcp-session-id"))
            .map(|(_, value)| value.clone())
            .or_else(|| {
                serde_json::from_str::<Value>(&response)
                    .ok()
                    .and_then(|value| {
                        value
                            .pointer("/result/sessionId")
                            .or_else(|| value.pointer("/result/session_id"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
            })
            .ok_or_else(|| WpsError::Protocol("初始化响应缺少 MCP session id".into()))?;
        if session.is_empty() {
            return Err(WpsError::Protocol("初始化返回空 MCP session id".into()));
        }
        Ok(session)
    }

    fn call(
        &self,
        session_id: &str,
        action: &str,
        args: &Value,
        auth: Option<&str>,
    ) -> Result<Value, WpsError> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": action, "arguments": args}
        });
        let (status, _, response) = self.send("POST", Some(session_id), auth, &body)?;
        if status >= 400 {
            if let Ok(value) = serde_json::from_str::<Value>(&response) {
                if value.get("error").is_some() {
                    return Err(WpsError::from_mcp_error(&value));
                }
            }
            return Err(WpsError::from_http_status(status));
        }
        let value: Value = serde_json::from_str(&response)
            .map_err(|_| WpsError::Protocol("回环 HTTP 响应不是 JSON".into()))?;
        if let Some(error) = value.get("error") {
            return Err(WpsError::from_mcp_error(error));
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| WpsError::Protocol("回环 HTTP 响应缺少 result".into()))
    }
}

fn parse_authority(authority: &str) -> Result<(String, u16), WpsError> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| WpsError::Credentials("IPv6 端点格式错误".into()))?;
        let port = tail
            .strip_prefix(':')
            .ok_or_else(|| WpsError::Credentials("IPv6 端点缺少端口".into()))?;
        (format!("[{host}]"), port)
    } else {
        let (host, port) = authority
            .rsplit_once(':')
            .ok_or_else(|| WpsError::Credentials("端点缺少端口".into()))?;
        (host.to_string(), port)
    };
    let port: u16 = port
        .parse()
        .map_err(|_| WpsError::Credentials("端点端口不是有效数字".into()))?;
    if port == 0 {
        return Err(WpsError::Credentials("端点端口不能为 0".into()));
    }
    Ok((host, port))
}

fn check_status(status: u16) -> Result<(), WpsError> {
    if (300..400).contains(&status) {
        return Err(WpsError::Remote(format!("HTTP {status}（不跟随重定向）")));
    }
    if status >= 400 {
        return Err(WpsError::from_http_status(status));
    }
    Ok(())
}

fn parse_http_response(raw: &[u8]) -> Result<RawResponse, WpsError> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| WpsError::Protocol("HTTP 响应缺少头部结束标记".into()))?;
    let head = std::str::from_utf8(&raw[..split])
        .map_err(|_| WpsError::Protocol("HTTP 响应头部不是 UTF-8".into()))?;
    let mut lines = head.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| WpsError::Protocol("HTTP 响应缺少状态行".into()))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| WpsError::Protocol("HTTP 状态行格式错误".into()))?
        .parse::<u16>()
        .map_err(|_| WpsError::Protocol("HTTP 状态码格式错误".into()))?;
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    let body = String::from_utf8_lossy(&raw[split + 4..]).to_string();
    Ok((status, headers, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    struct StaticProvider(&'static str);

    impl crate::services::wps::WpsCredentialProvider for StaticProvider {
        fn access_token(&self) -> Result<String, WpsError> {
            Ok(self.0.into())
        }
    }

    struct ExpiredProvider;

    impl crate::services::wps::WpsCredentialProvider for ExpiredProvider {
        fn access_token(&self) -> Result<String, WpsError> {
            Err(WpsError::AuthExpired)
        }
    }

    /// 伪服务脚本：(状态码, 响应头, 响应体)。
    type Scripted = (u16, Vec<(&'static str, &'static str)>, &'static str);
    /// (端点, 请求数, 收到请求原文, 服务线程)。
    type Server = (
        String,
        Arc<AtomicUsize>,
        Arc<std::sync::Mutex<Vec<String>>>,
        thread::JoinHandle<()>,
    );

    fn spawn_server(responses: Vec<Scripted>) -> Server {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let handle = thread::spawn(move || {
            for (status, headers, body) in responses {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut buffer = [0u8; 16 * 1024];
                let n = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..n]).to_string();
                recorded.lock().expect("requests").push(request);
                seen.fetch_add(1, Ordering::SeqCst);
                let mut response = format!(
                    "HTTP/1.1 {status} OK\r\nConnection: close\r\nContent-Length: {}\r\n",
                    body.len()
                );
                for (name, value) in headers {
                    response.push_str(&format!("{name}: {value}\r\n"));
                }
                response.push_str("\r\n");
                response.push_str(body);
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (
            format!("127.0.0.1:{}/mcp", addr.port()),
            count,
            requests,
            handle,
        )
    }

    #[test]
    fn loopback_sends_bearer_auth_and_not_token_in_body() {
        let (endpoint, count, requests, handle) = spawn_server(vec![
            (
                200,
                vec![("Mcp-Session-Id", "sess-1")],
                r#"{"result":{"ok":true}}"#,
            ),
            (200, vec![], r#"{"result":{"records":[{"id":"r1"}]}}"#),
        ]);
        let transport = LoopbackHttpTransport::new(&endpoint).expect("transport");
        let client = crate::services::wps::WpsClient::new(transport, "file-1")
            .with_credentials(Arc::new(StaticProvider("synthetic-token")));
        client.initialize().expect("initialize");
        let call = client
            .call(
                "tools/call",
                &json!({"name":"dbsheet.list_records","arguments":{}}),
            )
            .expect("call");
        assert_eq!(call["records"][0]["id"], "r1");
        assert_eq!(count.load(Ordering::SeqCst), 2);
        let seen = requests.lock().expect("requests");
        assert_eq!(seen.len(), 2);
        assert!(seen[0].contains("Authorization: Bearer synthetic-token"));
        assert!(seen[1].contains("Authorization: Bearer synthetic-token"));
        assert!(!seen.iter().any(|request| request.contains("{\"token\"")));
        assert!(!seen
            .iter()
            .any(|request| request.contains("synthetic-token\"")));
        drop(seen);
        handle.join().expect("server");
    }

    #[test]
    fn missing_or_expired_credentials_make_zero_http_requests() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let endpoint = format!("127.0.0.1:{}/mcp", listener.local_addr().unwrap().port());
        let client = crate::services::wps::WpsClient::new(
            LoopbackHttpTransport::new(&endpoint).expect("transport"),
            "file-1",
        );
        assert_eq!(
            client.initialize().expect_err("missing").code(),
            "WPS_E_CREDENTIALS"
        );
        let expired = crate::services::wps::WpsClient::new(
            LoopbackHttpTransport::new(&endpoint).expect("transport"),
            "file-1",
        )
        .with_credentials(Arc::new(ExpiredProvider));
        assert_eq!(
            expired.initialize().expect_err("expired").code(),
            "WPS_E_AUTH"
        );
        listener.set_nonblocking(true).expect("nonblocking");
        assert!(listener.accept().is_err(), "no HTTP request expected");
    }

    #[test]
    fn redirect_is_not_followed() {
        let (endpoint, count, _, handle) = spawn_server(vec![(
            302,
            vec![("Location", "http://example.com/exfiltrate")],
            "",
        )]);
        let client = crate::services::wps::WpsClient::new(
            LoopbackHttpTransport::new(&endpoint).expect("transport"),
            "file-1",
        )
        .with_credentials(Arc::new(StaticProvider("synthetic-token")));
        let err = client.initialize().expect_err("redirect rejected");
        assert_eq!(err.code(), "WPS_E_REMOTE");
        assert!(!err.to_string().contains("example.com"));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        handle.join().expect("server");
    }

    #[test]
    fn error_body_free_text_is_not_echoed() {
        let body =
            r#"{"error":{"code":500,"message":"synthetic-token-13800000000-91310000MA1K-128600"}}"#;
        let (endpoint, _, _, handle) = spawn_server(vec![
            (
                200,
                vec![("Mcp-Session-Id", "sess-1")],
                r#"{"result":{"ok":true}}"#,
            ),
            (500, vec![], body),
        ]);
        let client = crate::services::wps::WpsClient::new(
            LoopbackHttpTransport::new(&endpoint).expect("transport"),
            "file-1",
        )
        .with_credentials(Arc::new(StaticProvider("synthetic-token")));
        client.initialize().expect("initialize");
        let err = client
            .call("tools/call", &json!({"name":"x","arguments":{}}))
            .expect_err("remote error");
        let text = err.to_string();
        assert_eq!(err.code(), "WPS_E_REMOTE");
        assert!(!text.contains("synthetic-token"));
        assert!(!text.contains("13800000000"));
        assert!(!text.contains("91310000MA1K"));
        assert!(!text.contains("128600"));
        handle.join().expect("server");
    }
}
