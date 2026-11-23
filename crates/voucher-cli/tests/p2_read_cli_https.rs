//! P2 只读 CLI 的本机 HTTPS 成功验收：独立 CLI 进程 + 隔离状态库 + 合成授权 +
//! 本机 rustls 测试服务，核对服务端动作计数与「同授权重启不可重放」。
//!
//! 不连接任何真实端点；证书、密钥与 CA 均为仓库内测试夹具。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use serde_json::{json, Value};

struct Server {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl Server {
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn methods(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|line| line.to_string())
            .collect()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../voucher-core/tests/fixtures/p2-tls")
        .join(name)
}

fn spawn_server() -> Server {
    use std::sync::Once;
    static PROVIDER: Once = Once::new();
    PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });

    let cert = std::fs::read(fixture("srv.der")).expect("读服务端证书");
    let key = std::fs::read(fixture("srv.key.der")).expect("读服务端私钥");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().expect("addr").port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));

    let config = Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(cert)],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(key),
                ),
            )
            .expect("server config"),
    );

    let requests_for_thread = Arc::clone(&requests);
    let stop_for_thread = Arc::clone(&stop);
    std::thread::spawn(move || loop {
        if stop_for_thread.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((tcp, _)) => {
                let _ = tcp.set_nonblocking(false);
                let config = Arc::clone(&config);
                let requests = Arc::clone(&requests_for_thread);
                std::thread::spawn(move || handle(tcp, config, requests));
            }
            Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(_) => break,
        }
    });

    Server {
        port,
        requests,
        stop,
    }
}

fn handle(
    tcp: std::net::TcpStream,
    config: Arc<rustls::ServerConfig>,
    requests: Arc<Mutex<Vec<String>>>,
) {
    let Ok(connection) = rustls::ServerConnection::new(config) else {
        return;
    };
    let mut stream = rustls::StreamOwned::new(connection, tcp);
    let mut raw = Vec::new();
    let mut buffer = [0u8; 2048];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                raw.extend_from_slice(&buffer[..read]);
                if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    if raw.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(&raw).to_string();
    let first_line = text.lines().next().unwrap_or_default().to_string();
    requests.lock().unwrap().push(first_line.clone());

    let (headers, body) = if text.contains("\"method\":\"initialize\"") {
        (
            vec![("Mcp-Session-Id".to_string(), "sess-cli-1".to_string())],
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\"}}"
                .to_string(),
        )
    } else if text.contains("\"method\":\"notifications/initialized\"")
        || first_line.starts_with("DELETE ")
    {
        (Vec::new(), String::new())
    } else {
        let payload = "{\"records\":[{\"A\":\"一\"}],\"total\":1}";
        let literal = serde_json::to_string(payload).expect("payload literal");
        (
            Vec::new(),
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":{literal}}}],\"isError\":false}}}}"
            ),
        )
    };
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    response.push_str(&body);
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "voucher-p2cli-https-{}-{}-{tag}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    dir
}

fn grant_path(dir: &Path, port: u16) -> PathBuf {
    let now = Utc::now();
    let value = json!({
        "grant_id": format!("A2-CLI-HTTPS-{}-01", std::process::id()),
        "approved_by": "安全/运维（示例）",
        "endpoint": format!("https://p2-local.test:{port}/mcp"),
        "file_id": "dbsheet_replica_001",
        "approved_file_id": "dbsheet_replica_001",
        "sheet_id": 3,
        "fields": ["A"],
        "window_start": (now - Duration::minutes(5)).to_rfc3339(),
        "window_end": (now + Duration::minutes(10)).to_rfc3339(),
        "approved_subject": "tenant-a/reader",
        "page_size": 50,
        "max_total_requests": 8,
        "terminate_allowed": true
    });
    let path = dir.join("grant.json");
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).expect("写授权单");
    path
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_voucher-cli"))
        .args(args)
        .output()
        .expect("运行 voucher-cli")
}

fn json_of(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|_| panic!("stdout 不是 JSON：{}", String::from_utf8_lossy(&out.stdout)))
}

#[test]
fn cli_runs_one_legal_read_over_local_https_then_blocks_replay() {
    let server = spawn_server();
    let dir = temp_dir("https-ok");
    let grant = grant_path(&dir, server.port);
    let state = dir.join("state.db");
    let ca = fixture("ca.der");

    let first = run(&[
        "wps-p2-read",
        "--grant",
        grant.to_str().unwrap(),
        "--state",
        state.to_str().unwrap(),
        "--transport",
        "loopback-tls",
        "--trust-root",
        ca.to_str().unwrap(),
    ]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "本机 HTTPS 合法读取应成功：{}",
        String::from_utf8_lossy(&first.stdout)
    );
    let value = json_of(&first);
    assert_eq!(value["ok"], true);
    assert_eq!(value["transport"], "loopback-tls");
    assert_eq!(value["sheet_id"], 3);
    assert_eq!(value["record_count"], 1);
    assert_eq!(value["attempts_used"], 1);
    assert_eq!(value["actions"]["initialize"], 1);
    assert_eq!(value["actions"]["read"], 1);
    assert_eq!(value["actions"]["notify"], 1);
    assert_eq!(value["actions"]["terminate"], 1);
    assert_eq!(value["actions"]["total"], 4);
    assert_eq!(value["termination"], "远端终止已确认");

    let methods = server.methods();
    assert_eq!(methods.len(), 4, "服务端应收到 4 次请求：{methods:?}");
    assert!(methods[0].starts_with("POST "), "初始化应为 POST");
    assert!(methods[1].starts_with("POST "), "通知应为 POST");
    assert!(methods[2].starts_with("POST "), "读取应为 POST");
    assert!(methods[3].starts_with("DELETE "), "终止应为 DELETE");

    // 同一授权 + 同一状态库重启 CLI：必须被状态库挡住且不新增请求。
    let before = server.count();
    let second = run(&[
        "wps-p2-read",
        "--grant",
        grant.to_str().unwrap(),
        "--state",
        state.to_str().unwrap(),
        "--transport",
        "loopback-tls",
        "--trust-root",
        ca.to_str().unwrap(),
    ]);
    assert_eq!(second.status.code(), Some(1));
    let value = json_of(&second);
    assert_eq!(value["error"]["code"], "P2_E_ALREADY_USED");
    assert_eq!(server.count(), before, "重放不得产生任何新请求");
}

#[test]
fn cli_check_does_not_consume_grant() {
    let server = spawn_server();
    let dir = temp_dir("check-only");
    let grant = grant_path(&dir, server.port);
    let state = dir.join("state.db");

    let out = run(&[
        "wps-p2-read",
        "--grant",
        grant.to_str().unwrap(),
        "--state",
        state.to_str().unwrap(),
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(json_of(&out)["check"], "preflight");
    assert!(!state.exists(), "预检不得登记/创建状态库");
    assert_eq!(server.count(), 0, "预检不得发网");

    // 预检后仍可用同一授权真正执行一次。
    let ca = fixture("ca.der");
    let exec = run(&[
        "wps-p2-read",
        "--grant",
        grant.to_str().unwrap(),
        "--state",
        state.to_str().unwrap(),
        "--transport",
        "loopback-tls",
        "--trust-root",
        ca.to_str().unwrap(),
    ]);
    assert_eq!(exec.status.code(), Some(0));
    assert_eq!(server.count(), 4);
}
