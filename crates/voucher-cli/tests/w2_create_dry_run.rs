//! `w2-create-dry-run` 的离线验收：只证明「预检结论正确 + 一行 JSON + 零写入」。
//!
//! 四张同步表的 `COUNT(*)` 前后必须完全一致——这是「干跑没有偷偷写库」的硬证据；
//! 全部场景都不发任何远端请求（命令内传输层是 `OfflineDryRunTransport`，只报错）。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use chrono::Utc;
use serde_json::{json, Value};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_voucher-cli")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "voucher-w2dry-{}-{}-{tag}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    dir
}

fn write_input(dir: &Path, value: &Value) -> PathBuf {
    let path = dir.join("input.json");
    std::fs::write(&path, serde_json::to_string_pretty(value).unwrap()).expect("写输入");
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

/// 四张同步表的行数：wps_write_intents / wps_links / wps_idempotency_keys / wps_sync_runs。
fn sync_table_counts(path: &Path) -> [i64; 4] {
    let conn = voucher_core::db::connection::open(path).expect("打开库");
    [
        "wps_write_intents",
        "wps_links",
        "wps_idempotency_keys",
        "wps_sync_runs",
    ]
    .map(|table| {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_or_else(|err| panic!("统计 {table} 失败：{err}"))
    })
}

fn table_names(path: &Path) -> Vec<String> {
    let conn = voucher_core::db::connection::open(path).expect("打开库");
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("准备查询");
    stmt.query_map((), |row| row.get::<_, String>(0))
        .expect("查表名")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("取表名")
}

#[test]
fn valid_batch_is_accepted_and_writes_nothing() {
    let dir = temp_dir("ok");
    let db = dir.join("w2.db");
    // 库文件不存在：命令自己建库 + 跑既有迁移，这也是验收的一部分。
    assert!(!db.exists(), "起点必须是空目录");
    let input = write_input(
        &dir,
        &json!({
            "sheet_id": 3,
            "records": [
                {"fields": {"应收单号": "AR-1001"}, "idempotency_key": "idem-AR-1001", "business_date": "2026-11-20"},
                {"fields": {"应收单号": "AR-1002"}, "idempotency_key": "idem-AR-1002", "business_date": "2026-11-20"}
            ]
        }),
    );

    let out = run(&[
        "w2-create-dry-run",
        "--db",
        db.to_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let payload = stdout_json(&out);
    assert_eq!(payload["ok"], json!(true));
    assert_eq!(payload["command"], json!("w2-create-dry-run"));
    assert_eq!(payload["transport"], json!("offline"));
    assert_eq!(payload["accepted"], json!(true));
    assert_eq!(payload["sheet_id"], json!(3));
    let outcomes = payload["outcomes"].as_array().expect("outcomes 数组");
    assert_eq!(outcomes.len(), 2);
    for outcome in outcomes {
        assert_eq!(outcome["status"], json!("would_create"));
        assert_eq!(outcome["code"], Value::Null);
    }
    assert_eq!(outcomes[0]["idempotency_key"], json!("idem-AR-1001"));
    assert_eq!(outcomes[1]["idempotency_key"], json!("idem-AR-1002"));

    assert_eq!(sync_table_counts(&db), [0, 0, 0, 0], "干跑不得写任何同步表");
}

#[test]
fn dedupe_only_sheet_is_rejected_but_still_exits_zero() {
    let dir = temp_dir("reject");
    let db = dir.join("w2.db");
    // 先建库，才能取到「运行前」的基线行数；既有库必须显式放行，且只读试跑。
    {
        let conn = voucher_core::db::connection::open(&db).expect("建库");
        voucher_core::db::schema::init(&conn).expect("建表");
    }
    let before = sync_table_counts(&db);
    let input = write_input(
        &dir,
        &json!({
            "sheet_id": 5,
            "records": [
                {"fields": {"调整编号": "ADJ-0001"}, "idempotency_key": "idem-ADJ-0001"},
                {"fields": {"调整编号": "ADJ-0002"}, "idempotency_key": "idem-ADJ-0002"}
            ]
        }),
    );

    let out = run(&[
        "w2-create-dry-run",
        "--db",
        db.to_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
        "--allow-existing-db",
    ]);
    // 整批被拒是业务结果，不是程序错误。
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let payload = stdout_json(&out);
    assert_eq!(payload["accepted"], json!(false));
    let outcomes = payload["outcomes"].as_array().expect("outcomes 数组");
    assert_eq!(outcomes.len(), 2);
    for outcome in outcomes {
        assert_eq!(outcome["status"], json!("rejected"));
        assert_eq!(outcome["code"], json!("WPS_E_DEDUPE_ONLY"));
        assert!(
            outcome["message"].as_str().is_some_and(|m| !m.is_empty()),
            "拒绝必须带人话原因"
        );
    }
    assert_eq!(sync_table_counts(&db), before, "拒绝路径同样零写入");
}

#[test]
fn empty_records_is_a_usage_error_and_creates_no_db() {
    let dir = temp_dir("empty");
    let db = dir.join("w2.db");
    let input = write_input(&dir, &json!({"sheet_id": 3, "records": []}));

    let out = run(&[
        "w2-create-dry-run",
        "--db",
        db.to_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("records 缺失或为空"), "stderr：{stderr}");
    assert!(out.stdout.is_empty(), "参数错误不得输出业务 JSON");
    assert!(!db.exists(), "输入校验必须先于建库");
}

#[test]
fn malformed_json_is_a_usage_error() {
    let dir = temp_dir("badjson");
    let db = dir.join("w2.db");
    let input = dir.join("input.json");
    std::fs::write(&input, "{not-json").expect("写坏输入");

    let out = run(&[
        "w2-create-dry-run",
        "--db",
        db.to_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("输入不是合法 JSON"),
        "stderr：{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn missing_db_or_input_flag_is_a_usage_error() {
    let out = run(&["w2-create-dry-run"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("需要 --db 与 --input"),
        "stderr：{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn existing_non_empty_db_is_refused_without_flag_and_left_untouched() {
    let dir = temp_dir("existing");
    let db = dir.join("w2.db");
    {
        let conn = voucher_core::db::connection::open(&db).expect("建库");
        voucher_core::db::schema::init(&conn).expect("建表");
    }
    let before = std::fs::metadata(&db).expect("取元数据");
    assert!(before.len() > 0, "前提：既有库非空");
    let input = write_input(
        &dir,
        &json!({
            "sheet_id": 3,
            "records": [
                {"fields": {"应收单号": "AR-9001"}, "idempotency_key": "idem-AR-9001", "business_date": "2026-11-20"}
            ]
        }),
    );

    let out = run(&[
        "w2-create-dry-run",
        "--db",
        db.to_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("干跑只接受新建库"), "stderr：{stderr}");
    assert!(stderr.contains("--allow-existing-db"), "stderr：{stderr}");
    assert!(out.stdout.is_empty(), "拒绝不得输出业务 JSON");

    let after = std::fs::metadata(&db).expect("取元数据");
    assert_eq!(after.len(), before.len(), "既有库大小不得变化");
    assert_eq!(
        after.modified().expect("mtime"),
        before.modified().expect("mtime"),
        "命令不得打开既有库：mtime 变化说明碰过它"
    );
}

#[test]
fn allow_existing_db_never_creates_tables_when_schema_is_missing() {
    let dir = temp_dir("allow-missing");
    let db = dir.join("w2.db");
    {
        let conn = voucher_core::db::connection::open(&db).expect("建库");
        // 只有一张无关表：非空、但不是 v0.28 的 WPS 库结构。
        conn.execute_batch("CREATE TABLE marker(id INTEGER)")
            .expect("建占位表");
    }
    let before = table_names(&db);
    let input = write_input(
        &dir,
        &json!({
            "sheet_id": 3,
            "records": [
                {"fields": {"应收单号": "AR-9101"}, "idempotency_key": "idem-AR-9101", "business_date": "2026-11-20"}
            ]
        }),
    );

    let out = run(&[
        "w2-create-dry-run",
        "--db",
        db.to_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
        "--allow-existing-db",
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "stderr：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("干跑不会自动建表"),
        "stderr：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(table_names(&db), before, "既有库不得新增任何表");
}

#[test]
fn allow_existing_db_reads_a_valid_database_without_writing() {
    let dir = temp_dir("allow-valid");
    let db = dir.join("w2.db");
    {
        let conn = voucher_core::db::connection::open(&db).expect("建库");
        voucher_core::db::schema::init(&conn).expect("建表");
    }
    let before_tables = table_names(&db);
    let before_counts = sync_table_counts(&db);
    let input = write_input(
        &dir,
        &json!({
            "sheet_id": 3,
            "records": [
                {"fields": {"应收单号": "AR-9201"}, "idempotency_key": "idem-AR-9201", "business_date": "2026-11-20"}
            ]
        }),
    );

    let out = run(&[
        "w2-create-dry-run",
        "--db",
        db.to_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
        "--allow-existing-db",
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let payload = stdout_json(&out);
    assert_eq!(payload["accepted"], json!(true));
    assert_eq!(payload["outcomes"][0]["status"], json!("would_create"));
    assert_eq!(table_names(&db), before_tables, "不得新增表");
    assert_eq!(sync_table_counts(&db), before_counts, "四张同步表不得变化");
}
