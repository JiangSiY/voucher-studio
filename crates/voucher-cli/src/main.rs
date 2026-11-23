//! voucher-cli：给脚本/自动化用的最小命令行入口。
//!
//! 命令：
//! - `import-ledger`：把台账导出的三张清单 CSV 导进 voucher（新建/命中已有凭证）
//! - `list-pending`：列出「台账已导、缺附件」的凭证，供打印待归档清单
//! - `wps-p2-read`：P2 只读入口的最小 CLI（离线默认；回环 TLS 用于离线验收；
//!   `https` 真实端点模式须显式三重开关，见 `wps_p2_read` 的文档注释）
//!
//! 设计约定：
//! - 不开新依赖（本仓库运行环境无法下载 crate），参数手写解析；
//! - 与 GUI 共用同一个数据库（默认 `~/Library/Application Support/VoucherStudio`），
//!   同一时刻只允许一个写者（数据库文件锁会拦住并发）。
//! - 退出码：0 成功；1 业务错误；2 参数错误。

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use voucher_core::services::wps_p2::{
    HttpsP2Transport, P2Action, P2Credential, P2Endpoint, P2Error, P2Gate, P2Grant,
    P2ReadOnlyEntry, P2ReadOutcome, P2ReadRequest, P2Transport, SqliteP2State,
    UnavailableP2Transport,
};
use voucher_core::services::{ingestion, query};
use voucher_core::CoreContext;

/// CLI 只允许这三种本机主机名：任何其他主机在发网前直接拒绝。
const P2_CLI_LOOPBACK_HOSTS: [&str; 3] = ["p2-local.test", "localhost", "127.0.0.1"];
/// 合成凭据：离线与回环模式专用，绝不读取钥匙串。
const P2_CLI_SYNTHETIC_TOKEN: &str = "synthetic-p2-token";

fn usage() -> &'static str {
    "voucher-cli 0.1
用法：
  voucher-cli import-ledger --csv <清单.csv> --kind <shoukuan|fukuan|jiti> [--dry-run] [--data-dir <目录>]
  voucher-cli list-pending [--json] [--data-dir <目录>]
  voucher-cli restore --zip <备份.zip> --target <空目录>
  voucher-cli wps-p2-read --grant <授权.json> [--state <状态.db>] [--check]
                          [--transport offline|loopback-tls --trust-root <ca.der>]
                          [--transport https --allow-real-endpoint --token-file <t>]

说明：
  import-ledger  台账导出的收款/付款/计提清单 → voucher 凭证（幂等键=流水号/单据号）
  list-pending   列出已导入但还没配图的凭证
  restore        把全量备份还原到一个空目录（冷备接管用：i3 PC 上跑完指向该目录即可）
  wps-p2-read    P2 只读入口：--check 只校验授权单；执行时默认 offline（零发网），
                 loopback-tls 仅允许 p2-local.test/localhost/127.0.0.1 且须显式提供测试 CA；
                 https 真实端点模式另需 --allow-real-endpoint 与 --token-file（默认关闭，
                 缺任一项即拒绝）；offline/loopback-tls 用合成 token，不读钥匙串
  --data-dir     指定数据目录（默认 ~/Library/Application Support/VoucherStudio）
"
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn fail(code: u8, msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("错误：{msg}");
    ExitCode::from(code)
}

/// 把全量备份还原到目标目录（冷备接管）。
///
/// 为什么要单独一个命令：网页版没有「重启应用」按钮，恢复走的是
/// 「写 restore.pending → 重启落位」；冷备接管时没有旧进程可重启，
/// 直接把备份摊到空目录、再让 center-server 指向它最省事。
fn restore_into(zip: &std::path::Path, target: &std::path::Path) -> voucher_core::Result<()> {
    use voucher_core::services::backup;
    std::fs::create_dir_all(target)?;
    // 先建一个干净的 ctx（只为了拿到临时目录），恢复结果落在 target/temp 里
    let ctx = CoreContext::init(Some(target.to_path_buf()))?;
    let staging = ctx.paths.temp.join("restore-staging");
    let _ = std::fs::remove_dir_all(&staging);
    let summary = backup::restore_backup(&ctx, zip, &staging)?;

    // 关掉刚建的空库，再落位（否则拷过去的 db 会被占用中的连接写回）
    drop(ctx);

    let staged_db = staging.join("data").join("voucher.db");
    if staged_db.exists() {
        std::fs::copy(&staged_db, target.join("voucher.db"))?;
    }
    let staged_att = staging.join("attachments");
    if staged_att.is_dir() {
        let dest = target.join("attachments");
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&staged_att, &dest)?;
    }
    let staged_cfg = staging.join("config").join("app.json");
    if staged_cfg.exists() {
        let cfg_dir = target.join("config");
        std::fs::create_dir_all(&cfg_dir)?;
        std::fs::copy(&staged_cfg, cfg_dir.join("app.json"))?;
    }
    println!(
        "已还原：{}（附件 {} 个，校验={}）",
        target.display(),
        summary.attachment_count,
        summary.checksums_verified
    );
    Ok(())
}

/// P2 CLI 的最小凭据：固定合成 token；主体来自授权单或 `--subject`。
struct SyntheticCredential {
    subject: Option<String>,
}

impl P2Credential for SyntheticCredential {
    fn access_token(&self) -> std::result::Result<String, P2Error> {
        Ok(P2_CLI_SYNTHETIC_TOKEN.to_string())
    }

    fn subject(&self) -> Option<String> {
        self.subject.clone()
    }
}

/// 真实端点模式专用的文件凭据。
///
/// 只从显式 `--token-file` 读取一次：不读钥匙串、不读环境变量、不接受 argv 明文；
/// token 值不进入任何输出与错误信息，读取失败一律在发网前结束。
struct FileTokenCredential {
    token: String,
    subject: Option<String>,
}

impl P2Credential for FileTokenCredential {
    fn access_token(&self) -> std::result::Result<String, P2Error> {
        Ok(self.token.clone())
    }

    fn subject(&self) -> Option<String> {
        self.subject.clone()
    }
}

/// 读取凭据文件并做最小校验；失败信息只描述文件，不回显内容。
fn read_token_file(path: &str) -> std::result::Result<String, String> {
    let raw =
        std::fs::read_to_string(path).map_err(|e| format!("凭据文件不可读（{path}）：{e}"))?;
    if raw.trim().is_empty() {
        return Err(format!("凭据文件为空（{path}）"));
    }
    let mut lines = raw.lines().filter(|line| !line.trim().is_empty());
    let Some(token) = lines.next() else {
        return Err(format!("凭据文件为空（{path}）"));
    };
    if lines.next().is_some() {
        return Err("凭据文件只能包含一行 token".to_string());
    }
    Ok(token.trim().to_string())
}

fn p2_field<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    value.get(key).filter(|v| !v.is_null())
}

fn p2_string(value: &serde_json::Value, key: &str) -> Option<String> {
    p2_field(value, key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn p2_time(value: &serde_json::Value, key: &str) -> Option<DateTime<Utc>> {
    p2_string(value, key).and_then(|raw| {
        DateTime::parse_from_rfc3339(&raw)
            .ok()
            .map(|v| v.with_timezone(&Utc))
    })
}

fn p2_number_u32(value: &serde_json::Value, key: &str) -> Option<u32> {
    p2_field(value, key)
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
}

fn p2_number_i64(value: &serde_json::Value, key: &str) -> Option<i64> {
    p2_field(value, key).and_then(|v| v.as_i64())
}

/// 从授权单 JSON 还原 `P2Grant`；缺项一律报参数错误，不做默认填充。
fn p2_grant_from_value(
    value: &serde_json::Value,
    allow_real_endpoint: bool,
) -> std::result::Result<P2Grant, String> {
    let grant_id = p2_string(value, "grant_id").ok_or("授权单缺少 grant_id")?;
    let endpoint_raw = p2_string(value, "endpoint").ok_or("授权单缺少 endpoint")?;
    let endpoint = P2Endpoint::parse(&endpoint_raw).map_err(|e| e.to_string())?;
    if !P2_CLI_LOOPBACK_HOSTS.contains(&endpoint.host()) && !allow_real_endpoint {
        return Err(format!(
            "CLI 默认只允许本机回环端点（{}），拒绝 {}；真实端点须显式加 --allow-real-endpoint",
            P2_CLI_LOOPBACK_HOSTS.join("/"),
            endpoint.host()
        ));
    }
    let file_id = p2_string(value, "file_id").ok_or("授权单缺少 file_id")?;
    let approved_file_id = p2_string(value, "approved_file_id").unwrap_or_else(|| file_id.clone());
    let sheet_id = p2_number_i64(value, "sheet_id").ok_or("授权单缺少 sheet_id")?;
    let fields: Vec<String> = value
        .get("fields")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if fields.is_empty() {
        return Err("授权单 fields 不能为空（必须逐列批准）".into());
    }
    let window_start = p2_time(value, "window_start").ok_or("授权单缺少合法 window_start")?;
    let window_end = p2_time(value, "window_end").ok_or("授权单缺少合法 window_end")?;
    let page_size = p2_number_u32(value, "page_size").ok_or("授权单缺少 page_size")?;
    let max_total_requests =
        p2_number_u32(value, "max_total_requests").ok_or("授权单缺少 max_total_requests")?;
    let terminate_allowed = p2_field(value, "terminate_allowed")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut sheets = BTreeMap::new();
    sheets.insert(sheet_id, fields.into_iter().collect::<BTreeSet<_>>());
    Ok(P2Grant {
        grant_id,
        approved_by: p2_string(value, "approved_by"),
        approved_endpoint: endpoint.clone(),
        endpoint,
        approved_file_id,
        file_id,
        sheets,
        window_start,
        window_end,
        approved_subject: p2_string(value, "approved_subject"),
        page_size,
        max_total_requests,
        terminate_allowed,
    })
}

fn p2_json_error(code: &str, message: &str) -> serde_json::Value {
    serde_json::json!({ "ok": false, "error": { "code": code, "message": message } })
}

fn p2_execute<T: P2Transport>(
    transport: T,
    credential: Arc<dyn P2Credential>,
    gate: &P2Gate,
    state: Arc<SqliteP2State>,
    request: &P2ReadRequest,
) -> std::result::Result<P2ReadOutcome, P2Error> {
    let entry = P2ReadOnlyEntry::new(transport, Some(credential)).with_state(state);
    entry.read(gate, request)
}

/// `wps-p2-read`：P2 只读入口的最小 CLI。
///
/// 设计约束（与 A2 授权单一致）：
/// - 端点只接受本机回环主机名，CLI 自身无法指向真实 WPS；
/// - 默认 `offline` 传输（零发网），`loopback-tls` 需显式测试 CA；
/// - 凭据固定合成 token，不读钥匙串、不接受外部 token；
/// - `--check` 只做发网前预检，不写状态库、不发网。
fn wps_p2_read(args: &[String]) -> ExitCode {
    let Some(grant_path) = arg_value(args, "--grant") else {
        return fail(2, "wps-p2-read 需要 --grant <授权.json>");
    };
    let check_only = args.iter().any(|a| a == "--check");
    let transport_kind = arg_value(args, "--transport").unwrap_or_else(|| "offline".into());
    if !matches!(
        transport_kind.as_str(),
        "offline" | "loopback-tls" | "https"
    ) {
        return fail(2, format!("未知 --transport：{transport_kind}"));
    }
    let allow_real_endpoint = args.iter().any(|a| a == "--allow-real-endpoint");
    if transport_kind == "https" && !allow_real_endpoint {
        return fail(
            2,
            "https 真实端点模式默认关闭：须显式加 --allow-real-endpoint 才放行",
        );
    }
    if !check_only && transport_kind == "loopback-tls" && arg_value(args, "--trust-root").is_none()
    {
        return fail(2, "loopback-tls 需要 --trust-root <测试 CA DER>");
    }
    if !check_only && arg_value(args, "--state").is_none() {
        return fail(2, "执行模式需要 --state <状态.db>（单次授权跨重启防重）");
    }
    // 真实端点模式必须在发网前读到凭据；读不到就地结束，且不在任何输出里回显内容。
    let real_token = if transport_kind == "https" {
        let Some(path) = arg_value(args, "--token-file") else {
            return fail(2, "https 真实端点模式需要 --token-file <凭据文件>");
        };
        match read_token_file(&path) {
            Ok(token) => Some(token),
            Err(message) => return fail(2, message),
        }
    } else {
        None
    };

    let raw = match std::fs::read_to_string(&grant_path) {
        Ok(raw) => raw,
        Err(e) => return fail(1, format!("授权单不可读：{e}")),
    };
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(e) => return fail(2, format!("授权单不是合法 JSON：{e}")),
    };
    let grant = match p2_grant_from_value(&value, allow_real_endpoint) {
        Ok(grant) => grant,
        Err(message) => {
            println!("{}", p2_json_error("P2_E_GRANT", &message));
            return ExitCode::from(2);
        }
    };
    let subject = arg_value(args, "--subject").or_else(|| grant.approved_subject.clone());
    let request = P2ReadRequest {
        sheet_id: p2_number_i64(&value, "sheet_id").unwrap_or_default(),
        offset: 0,
        page_size: grant.page_size as usize,
    };

    if check_only {
        return match grant.preflight(
            Utc::now(),
            P2Action::ListRecords,
            subject.as_deref(),
            request.sheet_id,
        ) {
            Ok(()) => {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "check": "preflight",
                        "transport": transport_kind,
                        "real_endpoint": transport_kind == "https",
                        "credential_file": real_token.is_some(),
                        "grant_id": grant.grant_id,
                        "host": grant.endpoint.host(),
                        "sheet_id": request.sheet_id,
                        "page_size": grant.page_size,
                        "fields": grant.sheets.values().next().map(|set| set.len()),
                        "terminate_allowed": grant.terminate_allowed,
                    })
                );
                ExitCode::SUCCESS
            }
            Err(err) => {
                println!("{}", p2_json_error(err.code(), &err.to_string()));
                ExitCode::from(1)
            }
        };
    }

    let state_path = PathBuf::from(arg_value(args, "--state").unwrap_or_default());
    let state = match SqliteP2State::open(&state_path) {
        Ok(state) => Arc::new(state),
        Err(err) => {
            println!("{}", p2_json_error(err.code(), &err.to_string()));
            return ExitCode::from(1);
        }
    };
    let gate = P2Gate::open(grant.clone());
    let outcome = match transport_kind.as_str() {
        "offline" => p2_execute(
            UnavailableP2Transport,
            Arc::new(SyntheticCredential {
                subject: subject.clone(),
            }) as Arc<dyn P2Credential>,
            &gate,
            state,
            &request,
        ),
        "loopback-tls" => {
            let trust = match std::fs::read(arg_value(args, "--trust-root").unwrap_or_default()) {
                Ok(bytes) => bytes,
                Err(e) => return fail(2, format!("测试 CA 不可读：{e}")),
            };
            let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), grant.endpoint.port());
            let transport = match HttpsP2Transport::builder(grant.endpoint.clone())
                .resolve(grant.endpoint.host(), address)
                .trust_root_der(trust)
                .build()
            {
                Ok(transport) => transport,
                Err(err) => {
                    println!("{}", p2_json_error(err.code(), &err.to_string()));
                    return ExitCode::from(1);
                }
            };
            p2_execute(
                transport,
                Arc::new(SyntheticCredential {
                    subject: subject.clone(),
                }) as Arc<dyn P2Credential>,
                &gate,
                state,
                &request,
            )
        }
        _ => {
            let Some(token) = real_token else {
                return fail(2, "https 模式缺少凭据（内部状态异常）");
            };
            let mut builder = HttpsP2Transport::builder(grant.endpoint.clone());
            if let Some(path) = arg_value(args, "--trust-root") {
                match std::fs::read(&path) {
                    Ok(bytes) => builder = builder.trust_root_der(bytes),
                    Err(e) => return fail(2, format!("信任根不可读：{e}")),
                }
            }
            let transport = match builder.build() {
                Ok(transport) => transport,
                Err(err) => {
                    println!("{}", p2_json_error(err.code(), &err.to_string()));
                    return ExitCode::from(1);
                }
            };
            p2_execute(
                transport,
                Arc::new(FileTokenCredential {
                    token,
                    subject: subject.clone(),
                }) as Arc<dyn P2Credential>,
                &gate,
                state,
                &request,
            )
        }
    };

    match outcome {
        Ok(outcome) => {
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "transport": transport_kind,
                    "real_endpoint": transport_kind == "https",
                    "grant_id": grant.grant_id,
                    "sheet_id": outcome.sheet_id,
                    "record_count": outcome.records.len(),
                    "cursor": outcome.cursor,
                    "total": outcome.total,
                    "attempts_used": outcome.attempts_used,
                    "actions": {
                        "initialize": outcome.actions.initialize,
                        "read": outcome.actions.read,
                        "notify": outcome.actions.notify,
                        "terminate": outcome.actions.terminate,
                        "total": outcome.actions.total,
                    },
                    "termination": outcome.termination.message(),
                })
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("{}", p2_json_error(err.code(), &err.to_string()));
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        print!("{}", usage());
        return ExitCode::from(2);
    };

    let data_dir = arg_value(&args, "--data-dir").map(PathBuf::from);

    match cmd.as_str() {
        "-h" | "--help" | "help" => {
            print!("{}", usage());
            ExitCode::SUCCESS
        }
        "import-ledger" => {
            let (Some(csv), Some(kind_raw)) =
                (arg_value(&args, "--csv"), arg_value(&args, "--kind"))
            else {
                return fail(2, "import-ledger 需要 --csv 与 --kind");
            };
            let kind = match kind_raw.as_str() {
                "shoukuan" | "收款" => ingestion::LedgerKind::Shoukuan,
                "fukuan" | "付款" => ingestion::LedgerKind::Fukuan,
                "jiti" | "计提" => ingestion::LedgerKind::Jiti,
                other => return fail(2, format!("未知 kind：{other}")),
            };
            let csv = PathBuf::from(csv);
            if !csv.exists() {
                return fail(1, format!("CSV 不存在：{}", csv.display()));
            }
            let dry_run = args.iter().any(|a| a == "--dry-run");
            let ctx = match CoreContext::init(data_dir) {
                Ok(c) => c,
                Err(e) => return fail(1, e),
            };
            match ingestion::import_ledger_csv(&ctx, &csv, kind, dry_run) {
                Ok(sum) => {
                    println!("{}", serde_json::to_string_pretty(&sum).unwrap_or_default());
                    ExitCode::SUCCESS
                }
                Err(e) => fail(1, e),
            }
        }
        "restore" => {
            let (Some(zip), Some(target)) =
                (arg_value(&args, "--zip"), arg_value(&args, "--target"))
            else {
                return fail(2, "restore 需要 --zip 与 --target");
            };
            let zip = PathBuf::from(zip);
            if !zip.exists() {
                return fail(1, format!("备份不存在：{}", zip.display()));
            }
            let target = PathBuf::from(target);
            match restore_into(&zip, &target) {
                Ok(summary) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&summary).unwrap_or_default()
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => fail(1, e),
            }
        }
        "list-pending" => {
            let ctx = match CoreContext::init(data_dir) {
                Ok(c) => c,
                Err(e) => return fail(1, e),
            };
            match query::list_pending_ledger(&ctx) {
                Ok(items) => {
                    if args.iter().any(|a| a == "--json") {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&items).unwrap_or_default()
                        );
                    } else if items.is_empty() {
                        println!("没有待归档凭证。");
                    } else {
                        println!("待归档 {} 张：", items.len());
                        for it in items {
                            let yuan = it
                                .amount_cents
                                .map(|c| format!("{:.2}", c as f64 / 100.0))
                                .unwrap_or_else(|| "-".into());
                            println!(
                                "  {}  {}  {}  ¥{}  {}",
                                it.voucher_date,
                                it.voucher_no,
                                it.ledger_ref,
                                yuan,
                                it.counterparty
                            );
                        }
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(1, e),
            }
        }
        "wps-p2-read" => wps_p2_read(&args),
        other => fail(2, format!("未知命令：{other}\n\n{}", usage())),
    }
}
