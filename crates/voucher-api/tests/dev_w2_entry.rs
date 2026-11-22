//! 桌面开发入口的进程内验收：守卫 + pull + 正常/超时/缺引用 + 400006/429 与授权恢复。
//! 只使用临时目录与合成 mock transport，不读取 WPS 凭据、不发真实请求。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use voucher_api::dev_w2;
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
    let dir = std::env::temp_dir().join(format!("voucher-w2-entry-{stamp}"));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let ctx = CoreContext::init(Some(dir.clone())).expect("init core");
    (TempDir(dir), ctx)
}

#[test]
fn desktop_entry_runs_pull_write_and_stop_scenarios() {
    let (dir, ctx) = temp_ctx();
    std::env::set_var("VOUCHER_DATA_DIR", &dir.0);
    std::env::remove_var("VOUCHER_W2_MOCK");

    // 普通运行（未开启显式 Mock 开关）必须被守卫拒绝，且不触碰任何 WPS 数据。
    let denied = dev_w2::run(&ctx, Some("pull"), None).expect_err("must reject");
    assert_eq!(denied.code, "dev_only");

    std::env::set_var("VOUCHER_W2_MOCK", "1");
    let pull = dev_w2::run(&ctx, Some("pull"), None).expect("pull");
    assert_eq!(pull.mode, "pull");
    let pull_report = pull.pull.as_ref().expect("pull report");
    assert!(pull_report.complete);
    assert_eq!(pull_report.sheets.len(), 11);
    assert!(pull.batch.is_none());
    assert_eq!(pull.mock_requests.create, 0, "只读拉取不得发出 create 请求");

    // 400006：本批停止、需重新授权；未恢复前任何写批次都零请求。
    let auth = dev_w2::run(&ctx, Some("write"), Some("auth_expired")).expect("auth");
    let batch = auth.batch.expect("auth batch");
    assert!(batch.auth_required);
    assert!(batch.cooldown_until.is_none());
    assert_eq!(batch.outcomes[0].status, "unknown");
    assert_eq!(auth.mock_requests.create, 1);
    assert_eq!(
        auth.mock_requests.create_keys,
        vec!["AR-DEV-A1".to_string()]
    );

    let blocked = dev_w2::run(&ctx, Some("write"), Some("normal")).expect("blocked");
    assert!(blocked.pull.is_none(), "授权未恢复时不得拉取");
    assert!(blocked.batch.expect("blocked batch").auth_required);
    assert_eq!(blocked.mock_requests.total, 0);

    let recovered = dev_w2::run(&ctx, Some("auth_recover"), None).expect("recover");
    assert_eq!(recovered.mode, "auth_recover");
    assert_eq!(recovered.mock_requests.total, 0);

    let after = dev_w2::run(&ctx, Some("write"), Some("normal")).expect("after recover");
    assert_eq!(
        after.mock_requests.create_keys,
        vec!["AR-DEV-001".to_string(), "AR-DEV-002".to_string()],
        "恢复授权后不得自动重发待处理键"
    );
    let batch = after.batch.expect("after batch");
    assert_eq!(batch.outcomes.len(), 2);
    assert!(batch.outcomes.iter().all(|o| o.status == "succeeded"));
    assert!(batch.outcomes.iter().all(|o| o.saved_link));
    assert!(batch.outcomes.iter().all(|o| !o.posted));

    let timeout = dev_w2::run(&ctx, Some("write"), Some("timeout_second")).expect("timeout");
    let batch = timeout.batch.expect("batch");
    assert_eq!(
        batch.outcomes.iter().map(|o| o.status).collect::<Vec<_>>(),
        vec!["succeeded", "unknown", "not_attempted"]
    );
    assert!(batch.stopped);

    let missing = dev_w2::run(&ctx, Some("write"), Some("missing_reference")).expect("missing");
    let batch = missing.batch.expect("batch");
    assert_eq!(batch.outcomes[0].status, "rejected");
    assert!(batch.stopped);
    assert_eq!(
        missing.mock_requests.create, 0,
        "预检失败不得发出 create 请求"
    );

    // 429：本批停止并返回冷却截止；冷却期内再次点击零请求。
    let rate = dev_w2::run(&ctx, Some("write"), Some("rate_limited")).expect("rate");
    let batch = rate.batch.expect("rate batch");
    let until = batch.cooldown_until.clone().expect("cooldown");
    assert!(!batch.auth_required);
    assert!(batch.stopped);
    assert_eq!(rate.mock_requests.create, 1);

    let cooled = dev_w2::run(&ctx, Some("write"), Some("normal")).expect("cooled");
    assert!(cooled.pull.is_none());
    assert_eq!(
        cooled
            .batch
            .expect("cooled batch")
            .cooldown_until
            .as_deref(),
        Some(until.as_str()),
        "页面冷却时刻必须与批次返回值一致"
    );
    assert_eq!(
        cooled.mock_requests.total, 0,
        "冷却期内不得发出任何 Mock 请求"
    );

    std::env::remove_var("VOUCHER_W2_MOCK");
    std::env::remove_var("VOUCHER_DATA_DIR");
}
