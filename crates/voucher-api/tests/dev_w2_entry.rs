//! 桌面开发入口的进程内验收：守卫 + pull + 正常/超时/缺引用三场景。
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
    std::env::set_var("VOUCHER_W2_MOCK", "1");

    let pull = dev_w2::run(&ctx, Some("pull"), None).expect("pull");
    assert_eq!(pull.mode, "pull");
    assert!(pull.pull.complete);
    assert_eq!(pull.pull.sheets.len(), 11);
    assert!(pull.batch.is_none());

    let normal = dev_w2::run(&ctx, Some("write"), Some("normal")).expect("normal");
    let batch = normal.batch.expect("batch");
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

    std::env::remove_var("VOUCHER_W2_MOCK");
    std::env::remove_var("VOUCHER_DATA_DIR");
}
