//! 开发态 W2 Mock 命令（桌面命令入口；不是生产同步）。
//!
//! 守卫是三重显式条件：debug 构建、`VOUCHER_W2_MOCK=1`、数据目录位于临时目录。
//! 命令本身不接受 file_id / 数据目录参数，因此界面无法把目标改成真实文件或生产库。

use std::path::{Path, PathBuf};

use serde::Serialize;

use voucher_core::services::wps::WpsStore;
use voucher_core::services::wps_w2::{W2BatchReport, W2PullReport, W2WriteStatus};
use voucher_core::services::wps_w2_dev::{
    dev_auth_recover, dev_pull, dev_write, DevW2MockCounts as CoreMockCounts, DevW2Scenario,
};
use voucher_core::CoreContext;

use crate::{ApiError, ApiResult};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevW2Result {
    pub mode: String,
    pub data_dir: String,
    /// `None` 表示本次被前置阻断（授权失效/冷却中）而未拉取，transport 零请求。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull: Option<DevW2Pull>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch: Option<DevW2Batch>,
    /// 本次调用的 Mock transport 请求计数（含只读拉取）。
    pub mock_requests: DevW2MockCounts,
}

/// 一次开发调用的 Mock transport 请求计数；`create_keys` 用于核对是否重发旧幂等键。
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevW2MockCounts {
    pub total: usize,
    pub list: usize,
    pub create: usize,
    pub create_keys: Vec<String>,
}

impl From<CoreMockCounts> for DevW2MockCounts {
    fn from(value: CoreMockCounts) -> Self {
        Self {
            total: value.total,
            list: value.list,
            create: value.create,
            create_keys: value.create_keys,
        }
    }
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
    /// 本批因 400006/授权失效停止；待处理项已保留，恢复后不得自动重发。
    pub auth_required: bool,
    /// 429 冷却截止时间（RFC3339）；冷却内不得发起新批次。
    pub cooldown_until: Option<String>,
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

/// 启动前校验：Mock 必须显式传入一个已存在、规范化后仍位于临时根内、且不等于
/// 临时根本身的独立目录。失败即返回错误，调用方不得回落到默认数据目录。
pub fn validate_existing_temp_data_dir(
    base: Option<&Path>,
    temp_root: &Path,
) -> std::result::Result<PathBuf, String> {
    let raw = base.ok_or_else(|| {
        "VOUCHER_W2_MOCK=1 时必须显式设置 VOUCHER_DATA_DIR（已存在的独立临时目录），不能回落默认数据目录"
            .to_string()
    })?;
    if raw.as_os_str().is_empty() {
        return Err("VOUCHER_DATA_DIR 为空；请用 mktemp 创建独立临时目录".into());
    }
    let raw = raw
        .canonicalize()
        .map_err(|e| format!("VOUCHER_DATA_DIR 不可解析（{}）：{e}", raw.display()))?;
    let root = temp_root
        .canonicalize()
        .map_err(|e| format!("系统临时根不可解析（{}）：{e}", temp_root.display()))?;
    if raw == root {
        return Err("VOUCHER_DATA_DIR 不能是系统临时根本身；请用 mktemp 建独立子目录".into());
    }
    if !raw.starts_with(&root) {
        return Err(format!(
            "VOUCHER_DATA_DIR 规范化后不在系统临时根内：{}（临时根 {}）",
            raw.display(),
            root.display()
        ));
    }
    if !raw.is_dir() {
        return Err(format!(
            "VOUCHER_DATA_DIR 不是已存在的目录：{}",
            raw.display()
        ));
    }
    Ok(raw)
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
    if !cfg!(debug_assertions) || !flag {
        return Err(ApiError::plain(
            "dev_only",
            "开发 Mock 仅限 debug 构建 + VOUCHER_W2_MOCK=1 + 临时数据目录",
        ));
    }
    validate_existing_temp_data_dir(Some(&ctx.paths.base), &std::env::temp_dir())
        .map_err(|e| ApiError::plain("dev_only", e))?;
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
        auth_required: report.auth_required,
        cooldown_until: report.cooldown_until.clone(),
        outcomes,
    })
}

/// `action` 为 `pull` / `write` / `auth_recover`；`scenario` 只在 `write` 时生效。
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
            let run = dev_pull(&store).map_err(ApiError::from_core)?;
            Ok(DevW2Result {
                mode: "pull".into(),
                data_dir: ctx.paths.base.to_string_lossy().to_string(),
                pull: run.pull.as_ref().map(pull_dto),
                batch: None,
                mock_requests: run.mock.into(),
            })
        }
        "write" => {
            let scenario = DevW2Scenario::parse(scenario).map_err(ApiError::from_core)?;
            let run = dev_write(&store, scenario).map_err(ApiError::from_core)?;
            let batch = match run.report.as_ref() {
                Some(report) => Some(batch_dto(&store, report)?),
                None => None,
            };
            Ok(DevW2Result {
                mode: format!("write:{}", scenario.as_str()),
                data_dir: ctx.paths.base.to_string_lossy().to_string(),
                pull: run.pull.as_ref().map(pull_dto),
                batch,
                mock_requests: run.mock.into(),
            })
        }
        "auth_recover" => {
            dev_auth_recover(&store).map_err(ApiError::from_core)?;
            Ok(DevW2Result {
                mode: "auth_recover".into(),
                data_dir: ctx.paths.base.to_string_lossy().to_string(),
                pull: None,
                batch: None,
                mock_requests: DevW2MockCounts::default(),
            })
        }
        other => Err(ApiError::plain(
            "invalid_args",
            format!("未知开发 Mock 动作: {other}（可用 pull/write/auth_recover）"),
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

    fn unique_temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "voucher-w2-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn validator_requires_explicit_existing_isolated_temp_dir() {
        let root = unique_temp("root");
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        assert!(validate_existing_temp_data_dir(Some(&data), &root).is_ok());

        assert!(validate_existing_temp_data_dir(None, &root).is_err());
        assert!(validate_existing_temp_data_dir(Some(Path::new("")), &root).is_err());
        assert!(validate_existing_temp_data_dir(Some(&root), &root).is_err());
        assert!(validate_existing_temp_data_dir(Some(&data.join("missing")), &root).is_err());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[cfg(unix)]
    #[test]
    fn validator_rejects_symlink_escaping_temp_root() {
        let base = unique_temp("symlink-base");
        let root = base.join("root");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let link = root.join("escape");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let err = validate_existing_temp_data_dir(Some(&link), &root).unwrap_err();
        assert!(err.contains("不在系统临时根内"), "{err}");
        let _ = std::fs::remove_dir_all(&base);
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
