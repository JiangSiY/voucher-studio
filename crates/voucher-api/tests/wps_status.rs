//! `wps_sync_status` 的进程内验收：空库空态、计数与队列、20 条上限、时间归一 `…Z`、
//! dispatch 契约，以及「只读」的静态判据。
//!
//! 全部使用临时目录 + 合成数据，不读取 WPS 凭据、不发任何网络请求。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use voucher_api::wps_status::{self, WpsSyncStatus};
use voucher_core::CoreContext;

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_ctx() -> (TempDir, CoreContext) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("voucher-wps-status-{stamp}"));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let ctx = CoreContext::init(Some(dir.clone())).expect("init core");
    (TempDir(dir), ctx)
}

fn exec(ctx: &CoreContext, sql: &str) {
    ctx.db
        .with_conn(|conn| {
            conn.execute(sql, ())?;
            Ok(())
        })
        .expect("seed");
}

fn insert_link(ctx: &CoreContext, key: &str, state: &str, updated_at: &str) {
    exec(
        ctx,
        &format!(
            "INSERT INTO wps_links(sheet_id, record_id, doc_type, idempotency_key, state, created_at, updated_at)
             VALUES(3, 'R-{key}', '03', 'R-{key}', '{state}', '{updated_at}', '{updated_at}')"
        ),
    );
}

fn insert_intent(ctx: &CoreContext, key: &str, state: &str, error: Option<&str>, updated_at: &str) {
    let error_sql = match error {
        Some(text) => format!("'{text}'"),
        None => "NULL".to_string(),
    };
    exec(
        ctx,
        &format!(
            "INSERT INTO wps_write_intents(run_id, batch_id, sequence, sheet_id, record_type,
                                           business_key, idempotency_key, payload_hash, state, error,
                                           created_at, updated_at)
             VALUES(1, 'b1', 0, 3, '03', 'K-{key}', 'K-{key}', 'hash', '{state}', {error_sql}, '{updated_at}', '{updated_at}')"
        ),
    );
}

#[test]
fn empty_database_returns_zeroed_status() {
    let (_dir, ctx) = temp_ctx();

    let status = wps_status::wps_sync_status(&ctx).expect("status");
    assert_eq!(status, WpsSyncStatus::empty());
    assert!(status.last_sync_at.is_none());
    assert!(status.last_scan_complete.is_none());
    assert_eq!(status.pending_count, 0);
    assert_eq!(status.unknown_count, 0);
    assert!(status.pending_items.is_empty());

    // 序列化契约：camelCase、空态是 null/0/[]，前端 wpsTime 直接可判空。
    let wire = serde_json::to_value(&status).expect("json");
    assert_eq!(wire["lastSyncAt"], json!(null));
    assert_eq!(wire["lastScanComplete"], json!(null));
    assert_eq!(wire["pendingCount"], json!(0));
    assert_eq!(wire["unknownCount"], json!(0));
    assert_eq!(wire["pendingItems"], json!([]));
}

#[test]
fn seeded_rows_are_counted_listed_and_timestamps_are_z() {
    let (_dir, ctx) = temp_ctx();

    exec(
        &ctx,
        "INSERT INTO wps_sync_runs(started_at, finished_at, status) VALUES('2026-11-24 04:00:00', '2026-11-24 04:00:10', 'ok')",
    );
    // 更晚但不是成功态的运行不得顶掉 last_sync_at。
    exec(
        &ctx,
        "INSERT INTO wps_sync_runs(started_at, finished_at, status) VALUES('2026-11-24 05:00:00', '2026-11-24 05:00:10', 'partial')",
    );
    exec(
        &ctx,
        "INSERT INTO wps_scans(sheet_id, started_at, finished_at, complete) VALUES(3, '2026-11-24 03:00:00', '2026-11-24 03:00:05', 1)",
    );
    // 未完成的扫描不算「发布」。
    exec(
        &ctx,
        "INSERT INTO wps_scans(sheet_id, started_at, finished_at, complete) VALUES(4, '2026-11-24 06:00:00', '2026-11-24 06:00:05', 0)",
    );

    insert_link(&ctx, "P1", "pending_backfill", "2026-11-24 04:10:00");
    insert_link(&ctx, "P2", "pending_backfill", "2026-11-24 04:20:00");
    insert_link(&ctx, "OK", "synced", "2026-11-24 04:30:00");
    insert_intent(
        &ctx,
        "U1",
        "unknown",
        Some("WPS_E_TIMEOUT: 请求超时，结果不明"),
        "2026-11-24 04:40:00",
    );

    let status = wps_status::wps_sync_status(&ctx).expect("status");
    let last_sync = status.last_sync_at.expect("last_sync_at");
    assert!(last_sync.ends_with('Z'), "时间必须以 Z 结尾：{last_sync}");
    assert_eq!(last_sync, "2026-11-24T04:00:10Z");

    let last_scan = status.last_scan_complete.expect("last_scan_complete");
    assert!(last_scan.ends_with('Z'), "时间必须以 Z 结尾：{last_scan}");
    assert_eq!(last_scan, "2026-11-24T03:00:05Z");

    assert_eq!(status.pending_count, 2);
    assert_eq!(status.unknown_count, 1);
    assert_eq!(status.pending_items.len(), 3, "2 条回填 + 1 条结果不明");
    // 按更新时间倒序：unknown(04:40) > P2(04:20) > P1(04:10)。
    assert_eq!(status.pending_items[0].idempotency_key, "K-U1");
    assert_eq!(status.pending_items[0].code, "WPS_E_TIMEOUT");
    assert_eq!(status.pending_items[0].state, "unknown");
    assert_eq!(status.pending_items[1].idempotency_key, "R-P2");
    assert_eq!(status.pending_items[1].code, "WPS_E_LINK_BACKFILL");
    assert_eq!(status.pending_items[1].state, "pending_backfill");
    assert_eq!(status.pending_items[2].idempotency_key, "R-P1");
    assert_eq!(status.pending_items[2].sheet_id, 3);

    // 不回显字段值/远端 payload：每条只有 4 个键。
    for item in &status.pending_items {
        let wire = serde_json::to_value(item).expect("item json");
        let obj = wire.as_object().expect("object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["code", "idempotencyKey", "sheetId", "state"]);
        assert!(!wire.to_string().contains("fields"), "不得带字段值：{wire}");
    }
}

#[test]
fn pending_items_are_capped_at_twenty_but_counts_are_not() {
    let (_dir, ctx) = temp_ctx();
    for i in 0..25 {
        let key = format!("P{i:02}");
        let updated = format!("2026-11-24 04:{:02}:00", i);
        insert_link(&ctx, &key, "pending_backfill", &updated);
    }

    let status = wps_status::wps_sync_status(&ctx).expect("status");
    assert_eq!(status.pending_count, 25, "计数不受列表上限影响");
    assert_eq!(status.pending_items.len(), 20);
    // 倒序截断：最新的 P24 在首位，P05 落在第 20 位之后被截掉。
    assert_eq!(status.pending_items[0].idempotency_key, "R-P24");
    assert_eq!(status.pending_items[19].idempotency_key, "R-P05");
}

#[test]
fn legacy_plus_zero_offset_is_normalized_to_z() {
    let (_dir, ctx) = temp_ctx();
    exec(
        &ctx,
        "INSERT INTO wps_sync_runs(started_at, finished_at, status) VALUES('2026-11-24T04:00:00+00:00', '2026-11-24T04:00:10+00:00', 'ok')",
    );

    let status = wps_status::wps_sync_status(&ctx).expect("status");
    let last_sync = status.last_sync_at.expect("last_sync_at");
    assert!(last_sync.ends_with('Z'), "不得输出 +00:00：{last_sync}");
    assert_eq!(last_sync, "2026-11-24T04:00:10Z");
}

#[test]
fn dispatch_exposes_the_camel_case_command() {
    let (_dir, ctx) = temp_ctx();
    let out = voucher_api::dispatch(&ctx, "wps_sync_status", json!({})).expect("dispatch");
    assert_eq!(out["pendingCount"], json!(0));
    assert_eq!(out["pendingItems"], json!([]));
    assert!(out.get("lastSyncAt").is_some());
}

/// 静态判据：本命令只读本地库——不得出现客户端/网络/凭据符号。
#[test]
fn command_source_has_no_network_or_credential_symbols() {
    let src = include_str!("../src/wps_status.rs");
    for needle in [
        ["Wps", "Client"].concat(),
        ["Wps", "Transport"].concat(),
        ["create", "_record("].concat(),
        ["list", "_records("].concat(),
        ["secret", "s"].concat(),
        ["Tcp", "Stream"].concat(),
    ] {
        assert!(!src.contains(&needle), "wps_status.rs 不得出现 {needle}");
    }
}
