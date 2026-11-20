//! 开发态 W2 Mock 命令（桌面命令入口；不是生产同步）。
//!
//! 守卫是三重显式条件：debug 构建、`VOUCHER_W2_MOCK=1`、数据目录位于临时目录。
//! 命令本身不接受 file_id / 数据目录参数，因此界面无法把目标改成真实文件或生产库。

use std::path::Path;

use serde::Serialize;

use voucher_core::services::wps::WpsStore;
use voucher_core::services::wps_w2::{W2BatchReport, W2PullReport, W2WriteStatus};
use voucher_core::services::wps_w2_dev::{dev_pull, dev_write, DevW2Scenario};
use voucher_core::CoreContext;

use crate::{ApiError, ApiResult};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevW2Result {
    pub mode: String,
    pub data_dir: String,
    pub pull: DevW2Pull,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch: Option<DevW2Batch>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevW2Pull {
    pub complete: bool,
    pub writes_blocked: bool,
    pub sheets: Vec<DevW2PullSheet>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevW2PullSheet {
    pub sheet_id: i64,
    pub record_type: String,
    pub complete: bool,
    pub skipped: usize,
    pub raw_records: usize,
    pub indexed: usize,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevW2Batch {
    pub run_id: Option<i64>,
    pub stopped: bool,
    pub outcomes: Vec<DevW2Outcome>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevW2Outcome {
    pub sequence: usize,
    pub idempotency_key: String,
    pub status: &'static str,
    pub remote_record_id: Option<String>,
    pub link_id: Option<i64>,
    pub link_state: Option<String>,
    pub saved_link: bool,
    pub posted: bool,
    pub error: Option<String>,
}

/// 纯守卫，便于单测；`data_dir` 必须是临时目录下的隔离库。
pub fn dev_allowed(debug: bool, flag: bool, data_dir: &Path, temp_root: &Path) -> bool {
    debug && flag && data_dir.starts_with(temp_root)
}

fn ensure_dev(ctx: &CoreContext) -> ApiResult<()> {
    let flag = std::env::var("VOUCHER_W2_MOCK")
        .map(|v| v.trim() == "1")
        .unwrap_or(false);
    let data_dir_set = std::env::var("VOUCHER_DATA_DIR")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    if !data_dir_set {
        return Err(ApiError::plain(
            "dev_only",
            "开发 Mock 需要显式 VOUCHER_DATA_DIR（临时库），当前未设置",
        ));
    }
    if !dev_allowed(
        cfg!(debug_assertions),
        flag,
        &ctx.paths.base,
        &std::env::temp_dir(),
    ) {
        return Err(ApiError::plain(
            "dev_only",
            "开发 Mock 仅限 debug 构建 + VOUCHER_W2_MOCK=1 + 临时数据目录",
        ));
    }
    Ok(())
}

fn status_text(status: W2WriteStatus) -> &'static str {
    match status {
        W2WriteStatus::Succeeded => "succeeded",
        W2WriteStatus::Unknown => "unknown",
        W2WriteStatus::Rejected => "rejected",
        W2WriteStatus::PendingBackfill => "pending_backfill",
        W2WriteStatus::NotAttempted => "not_attempted",
    }
}

fn pull_dto(pull: &W2PullReport) -> DevW2Pull {
    DevW2Pull {
        complete: pull.complete,
        writes_blocked: pull.writes_blocked,
        sheets: pull
            .sheets
            .iter()
            .map(|sheet| DevW2PullSheet {
                sheet_id: sheet.sheet_id,
                record_type: sheet.report.record_type.clone(),
                complete: sheet.report.complete,
                skipped: sheet.report.skipped,
                raw_records: sheet.report.raw_records,
                indexed: sheet.report.indexed,
                error: sheet
                    .report
                    .error_code
                    .clone()
                    .or_else(|| sheet.report.error.clone()),
            })
            .collect(),
    }
}

fn batch_dto(store: &WpsStore<'_>, report: &W2BatchReport) -> ApiResult<DevW2Batch> {
    let outcomes = report
        .outcomes
        .iter()
        .enumerate()
        .map(|(index, outcome)| {
            let link_state = match outcome.link_id {
                Some(link_id) => store.link_state(link_id).map_err(ApiError::from_core)?,
                None => None,
            };
            Ok(DevW2Outcome {
                sequence: index + 1,
                idempotency_key: outcome.idempotency_key.clone(),
                status: status_text(outcome.status),
                remote_record_id: outcome.remote_record_id.clone(),
                link_id: outcome.link_id,
                link_state: link_state.clone(),
                saved_link: outcome.link_id.is_some(),
                posted: link_state.as_deref() == Some("posted"),
                error: outcome.error.clone(),
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(DevW2Batch {
        run_id: report.run_id,
        stopped: report.stopped,
        outcomes,
    })
}

/// `action` 只有 `pull` / `write`；`scenario` 只在 `write` 时生效。
pub fn run(
    ctx: &CoreContext,
    action: Option<&str>,
    scenario: Option<&str>,
) -> ApiResult<DevW2Result> {
    ensure_dev(ctx)?;
    let action = action.unwrap_or("pull").trim();
    let store = WpsStore::from_db(&ctx.db);
    match action {
        "pull" => {
            let pull = dev_pull(&store).map_err(ApiError::from_core)?;
            Ok(DevW2Result {
                mode: "pull".into(),
                data_dir: ctx.paths.base.to_string_lossy().to_string(),
                pull: pull_dto(&pull),
                batch: None,
            })
        }
        "write" => {
            let scenario = DevW2Scenario::parse(scenario).map_err(ApiError::from_core)?;
            let run = dev_write(&store, scenario).map_err(ApiError::from_core)?;
            Ok(DevW2Result {
                mode: format!("write:{}", scenario.as_str()),
                data_dir: ctx.paths.base.to_string_lossy().to_string(),
                pull: pull_dto(&run.pull),
                batch: Some(batch_dto(&store, &run.report)?),
            })
        }
        other => Err(ApiError::plain(
            "invalid_args",
            format!("未知开发 Mock 动作: {other}（可用 pull/write）"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn guard_rejects_non_debug_flag_or_non_temp_dir() {
        let temp = std::env::temp_dir().join("voucher-w2-mock-test");
        assert!(dev_allowed(true, true, &temp, &std::env::temp_dir()));
        assert!(!dev_allowed(false, true, &temp, &std::env::temp_dir()));
        assert!(!dev_allowed(true, false, &temp, &std::env::temp_dir()));
        assert!(!dev_allowed(
            true,
            true,
            &PathBuf::from("/Users/apple/Library/Application Support/VoucherStudio"),
            &std::env::temp_dir()
        ));
    }

    #[test]
    fn status_text_is_stable() {
        assert_eq!(status_text(W2WriteStatus::Succeeded), "succeeded");
        assert_eq!(
            status_text(W2WriteStatus::PendingBackfill),
            "pending_backfill"
        );
        assert_eq!(status_text(W2WriteStatus::NotAttempted), "not_attempted");
    }
}
