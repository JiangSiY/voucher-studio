//! P2 只读 CLI 的离线验收：不触发任何真实网络，只验证「本机回环 + 合成凭据 +
//! 单次授权状态库」三道边界。
//!
//! 证据口径：退出码 / JSON 错误码 / 状态库是否登记；全部场景都不连接真实端点。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use chrono::{Duration, Utc};
use serde_json::{json, Value};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_voucher-cli")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "voucher-p2cli-{}-{}-{tag}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    dir
}

/// 授权单：默认落在「当前时间 ±5/10 分钟」窗口内，端点固定本机回环。
fn grant(dir: &Path, endpoint: &str, file_id: &str) -> PathBuf {
    let now = Utc::now();
    let value = json!({
        "grant_id": format!("A2-CLI-{}-01", std::process::id()),
        "approved_by": "安全/运维（示例）",
        "endpoint": endpoint,
        "file_id": file_id,
        "approved_file_id": "dbsheet_replica_001",
        "sheet_id": 3,
        "fields": ["A", "B"],
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
    Command::new(binary())
        .args(args)
        .output()
        .expect("运行 voucher-cli")
}

fn stdout_json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|_| panic!("stdout 不是 JSON：{}", String::from_utf8_lossy(&out.stdout)))
}

fn code_of(out: &Output) -> String {
    stdout_json(out)
        .get("error")
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string()
}

#[test]
fn check_accepts_valid_grant_and_writes_no_state() {
    let dir = temp_dir("check-ok");
    let grant = grant(
        &dir,
        "https://p2-local.test:8443/mcp",
        "dbsheet_replica_001",
    );
    let state = dir.join("state.db");

    let out = run(&["wps-p2-read", "--grant", grant.to_str().unwrap(), "--check"]);
    assert!(out.status.success(), "预检应通过：{:?}", out);
    let value = stdout_json(&out);
    assert_eq!(value["ok"], true);
    assert_eq!(value["check"], "preflight");
    assert_eq!(value["sheet_id"], 3);
    assert!(!state.exists(), "预检不得创建状态库");
}

#[test]
fn check_rejects_non_loopback_endpoint_before_any_network() {
    let dir = temp_dir("check-host");
    let grant = grant(&dir, "https://example.com/mcp", "dbsheet_replica_001");

    let out = run(&["wps-p2-read", "--grant", grant.to_str().unwrap(), "--check"]);
    assert_eq!(out.status.code(), Some(2), "非回环端点应作为参数错误拒绝");
    let value = stdout_json(&out);
    assert_eq!(value["error"]["code"], "P2_E_GRANT");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("只允许本机回环端点"));
}

#[test]
fn execute_offline_fails_closed_and_blocks_replay() {
    let dir = temp_dir("offline");
    let grant = grant(
        &dir,
        "https://p2-local.test:8443/mcp",
        "dbsheet_replica_001",
    );
    let state = dir.join("state.db");

    let first = run(&[
        "wps-p2-read",
        "--grant",
        grant.to_str().unwrap(),
        "--state",
        state.to_str().unwrap(),
    ]);
    assert_eq!(first.status.code(), Some(1));
    assert_eq!(
        code_of(&first),
        "P2_E_PROTOCOL_PENDING",
        "offline 传输必须本地失败"
    );
    assert!(state.exists(), "执行模式应在发网前登记状态库");

    let second = run(&[
        "wps-p2-read",
        "--grant",
        grant.to_str().unwrap(),
        "--state",
        state.to_str().unwrap(),
    ]);
    assert_eq!(second.status.code(), Some(1));
    assert_eq!(
        code_of(&second),
        "P2_E_ALREADY_USED",
        "同一授权不得再次启动"
    );
}

#[test]
fn loopback_tls_to_closed_port_fails_locally_and_still_consumes_grant() {
    let dir = temp_dir("tls-closed");
    let grant = grant(&dir, "https://p2-local.test:1/mcp", "dbsheet_replica_001");
    let state = dir.join("state.db");
    let ca = dir.join("ca.der");
    // 空 DER 会被拒（P2_E_TLS），因此这里只验证「需要显式 CA」的参数边界。
    std::fs::write(&ca, [0u8; 8]).unwrap();

    let missing = run(&[
        "wps-p2-read",
        "--grant",
        grant.to_str().unwrap(),
        "--state",
        state.to_str().unwrap(),
        "--transport",
        "loopback-tls",
    ]);
    assert_eq!(missing.status.code(), Some(2), "缺 --trust-root 应拒绝");
    assert!(!state.exists(), "参数错误不得登记状态库");

    let out = run(&[
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
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(code_of(&out), "P2_E_TLS", "非法 CA 应在建连前失败");
    assert!(state.exists(), "进入执行后仍应先登记再发网");
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn token_file(dir: &Path, content: &str) -> PathBuf {
    let path = dir.join("token.txt");
    std::fs::write(&path, content).expect("写凭据文件");
    path
}

#[test]
fn real_endpoint_requires_explicit_allow_flag() {
    let dir = temp_dir("real-no-flag");
    // 非回环主机：默认应被授权单解析直接拒绝，且不进入任何网络分支。
    let grant = grant(
        &dir,
        "https://wps.example.invalid/mcp",
        "dbsheet_replica_001",
    );
    let out = run(&[
        "wps-p2-read",
        "--grant",
        grant.to_str().unwrap(),
        "--transport",
        "https",
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(2));
    let text = stderr_of(&out);
    assert!(text.contains("--allow-real-endpoint"), "{text}");
}

#[test]
fn real_endpoint_requires_token_file() {
    let dir = temp_dir("real-no-token");
    let grant = grant(&dir, "https://p2-local.test/mcp", "dbsheet_replica_001");
    let out = run(&[
        "wps-p2-read",
        "--grant",
        grant.to_str().unwrap(),
        "--transport",
        "https",
        "--allow-real-endpoint",
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(2));
    let text = stderr_of(&out);
    assert!(text.contains("--token-file"), "{text}");
}

#[test]
fn real_endpoint_rejects_empty_token_before_any_request() {
    let dir = temp_dir("real-empty-token");
    let grant = grant(&dir, "https://p2-local.test/mcp", "dbsheet_replica_001");
    let token = token_file(&dir, "   \n");
    let state = dir.join("state.db");
    let out = run(&[
        "wps-p2-read",
        "--grant",
        grant.to_str().unwrap(),
        "--transport",
        "https",
        "--allow-real-endpoint",
        "--token-file",
        token.to_str().unwrap(),
        "--state",
        state.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    let text = stderr_of(&out);
    assert!(text.contains("凭据文件为空"), "{text}");
    assert!(!state.exists(), "预检前失败不应创建状态库");
}

#[test]
fn real_endpoint_check_reports_readiness_without_state_or_network() {
    let dir = temp_dir("real-check");
    let grant = grant(&dir, "https://p2-local.test/mcp", "dbsheet_replica_001");
    let token = token_file(&dir, "synthetic-real-mode-token\n");
    let state = dir.join("state.db");
    let out = run(&[
        "wps-p2-read",
        "--grant",
        grant.to_str().unwrap(),
        "--transport",
        "https",
        "--allow-real-endpoint",
        "--token-file",
        token.to_str().unwrap(),
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(0));
    let value = stdout_json(&out);
    assert_eq!(value["real_endpoint"], json!(true));
    assert_eq!(value["credential_file"], json!(true));
    assert_eq!(value["check"], json!("preflight"));
    assert!(!state.exists());
    // token 值不得出现在输出里。
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.contains("synthetic-real-mode-token"), "{text}");
}
