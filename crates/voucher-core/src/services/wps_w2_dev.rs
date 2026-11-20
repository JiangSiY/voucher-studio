//! 开发态 W2 Mock 入口（只在显式隔离开关下由桌面命令调用）。
//!
//! 这不是生产同步：transport 是进程内合成数据，不读 WPS 凭据、不发 HTTP、不碰真实 file_id。
//! 调用方必须同时满足 `VOUCHER_W2_MOCK=1`、debug 构建与临时 `VOUCHER_DATA_DIR`（见 voucher-api
//! `dev_w2` 守卫）；本模块自身只负责构造可重复的合成批次。

use std::sync::Mutex;

use serde_json::{json, Value};

use std::sync::Arc;

use crate::services::wps::{WpsClient, WpsCredentialProvider, WpsError, WpsStore, WpsTransport};
use crate::services::wps_w2::{
    run_mock_batch, W2BatchReport, W2Config, W2CreateRequest, W2PullReport, W2Sync,
};
use crate::{CoreError, Result};

/// 可复跑的三种合成场景：正常两笔、第一笔成功第二笔超时、缺引用整批拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevW2Scenario {
    Normal,
    TimeoutSecond,
    MissingReference,
}

impl DevW2Scenario {
    pub fn parse(raw: Option<&str>) -> Result<Self> {
        match raw.unwrap_or("normal").trim().to_ascii_lowercase().as_str() {
            "normal" => Ok(Self::Normal),
            "timeout_second" | "timeout" => Ok(Self::TimeoutSecond),
            "missing_reference" | "missing_ref" => Ok(Self::MissingReference),
            other => Err(CoreError::Invalid(format!(
                "未知 W2 开发场景: {other}（可用 normal/timeout_second/missing_reference）"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::TimeoutSecond => "timeout_second",
            Self::MissingReference => "missing_reference",
        }
    }
}

/// 开发 Mock 专用合成凭据：只用于本地伪传输，不读取真实钥匙串。
pub struct DevW2Credentials;

impl WpsCredentialProvider for DevW2Credentials {
    fn access_token(&self) -> std::result::Result<String, WpsError> {
        Ok("synthetic-w2-dev-token".into())
    }
}

/// 一次合成批次的结果；`pull` 用于展示每张表的索引健康度。
pub struct DevW2Run {
    pub pull: W2PullReport,
    pub report: W2BatchReport,
}

/// 合成 transport：11 张表只返回预置基础资料；create 按场景成功/超时。
pub struct DevW2Transport {
    scenario: DevW2Scenario,
    create_calls: Mutex<usize>,
}

impl DevW2Transport {
    pub fn new(scenario: DevW2Scenario) -> Self {
        Self {
            scenario,
            create_calls: Mutex::new(0),
        }
    }

    fn reference_page(sheet_id: i64) -> Value {
        let records = match sheet_id {
            1 => vec![json!({"id": "dev-01-客户A", "fields": {"名称": "客户A"}})],
            8 => vec![json!({"id": "dev-08-1", "fields": {"表号": "METER-1"}})],
            10 => vec![json!({"id": "dev-10-1", "fields": {"SKU": "SKU-1"}})],
            _ => Vec::new(),
        };
        json!({
            "records": records,
            "has_more": false,
            "total": records.len(),
        })
    }
}

impl WpsTransport for DevW2Transport {
    fn initialize(&self, _auth: Option<&str>) -> std::result::Result<String, WpsError> {
        Ok("dev-w2-session".into())
    }

    fn call(
        &self,
        _session_id: &str,
        action: &str,
        args: &Value,
        _auth: Option<&str>,
    ) -> std::result::Result<Value, WpsError> {
        match action {
            "dbsheet.list_records" => {
                let sheet_id = args
                    .get("sheet_id")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| WpsError::Protocol("开发 mock 缺少 sheet_id".into()))?;
                Ok(Self::reference_page(sheet_id))
            }
            "dbsheet.create_records" => {
                let mut calls = self
                    .create_calls
                    .lock()
                    .map_err(|_| WpsError::Protocol("开发 mock 计数锁中毒".into()))?;
                let index = *calls;
                *calls += 1;
                if self.scenario == DevW2Scenario::TimeoutSecond && index == 1 {
                    return Err(WpsError::Timeout);
                }
                Ok(json!({
                    "records": [{"id": format!("dev-03-{}", index + 1)}],
                    "total": 1
                }))
            }
            other => Err(WpsError::Protocol(format!(
                "开发 mock 不支持的 action: {other}"
            ))),
        }
    }
}

/// 两笔正常创建；第三笔为超时场景预留，第四种为缺引用。
pub fn dev_requests(scenario: DevW2Scenario) -> Vec<W2CreateRequest> {
    let refs = vec![(1, "客户A".to_string()), (8, "METER-1".to_string())];
    let ref_ids = vec![(1, "dev-01-客户A".to_string()), (8, "dev-08-1".to_string())];
    let mut requests = match scenario {
        DevW2Scenario::Normal => vec![
            request(
                "AR-DEV-001",
                "开发模拟应收-1",
                refs.clone(),
                ref_ids.clone(),
            ),
            request("AR-DEV-002", "开发模拟应收-2", refs, ref_ids),
        ],
        DevW2Scenario::TimeoutSecond => vec![
            request(
                "AR-DEV-T1",
                "开发模拟应收-超时1",
                refs.clone(),
                ref_ids.clone(),
            ),
            request(
                "AR-DEV-T2",
                "开发模拟应收-超时2",
                refs.clone(),
                ref_ids.clone(),
            ),
            request("AR-DEV-T3", "开发模拟应收-超时3", refs, ref_ids),
        ],
        DevW2Scenario::MissingReference => vec![request(
            "AR-DEV-MISS",
            "开发模拟应收-缺引用",
            vec![(1, "不存在的客户".to_string()), (8, "METER-1".to_string())],
            vec![(1, "dev-01-客户A".to_string()), (8, "dev-08-1".to_string())],
        )],
    };
    if scenario == DevW2Scenario::MissingReference {
        requests[0].required_refs.push((10, "SKU-1".into()));
        requests[0]
            .required_ref_ids
            .push((10, "dev-10-missing".into()));
    }
    requests
}

fn request(
    doc_no: &str,
    summary: &str,
    refs: Vec<(i64, String)>,
    ref_ids: Vec<(i64, String)>,
) -> W2CreateRequest {
    let mut request = W2CreateRequest::new(
        3,
        "03",
        json!({
            "应收单号": doc_no,
            "客户": ["dev-01-客户A"],
            "摘要": summary,
            "金额": 100.00,
        }),
        doc_no,
    );
    request.business_date = Some("2026-11-20".into());
    request.required_refs = refs;
    request.required_ref_ids = ref_ids;
    request
}

/// 只拉取：结果与 `write` 前同样走一遍 11 表全历史链，不初始化写会话。
pub fn dev_pull(store: &WpsStore<'_>) -> Result<W2PullReport> {
    let client = WpsClient::new(DevW2Transport::new(DevW2Scenario::Normal), "mock://w2-dev")
        .with_credentials(Arc::new(DevW2Credentials));
    let sync = W2Sync::new(&client, store);
    sync.pull_all_history()
}

/// 显式测试写：固定 Mock 配置，先全历史拉取，再整批预检/逐条创建。
pub fn dev_write(store: &WpsStore<'_>, scenario: DevW2Scenario) -> Result<DevW2Run> {
    let client = WpsClient::new(DevW2Transport::new(scenario), "mock://w2-dev")
        .with_credentials(Arc::new(DevW2Credentials));
    let sync = W2Sync::new(&client, store);
    let pull = sync.pull_all_history()?;
    if pull.writes_blocked {
        return Err(CoreError::Invalid(
            "WPS_E_PULL_BLOCKED: 开发 mock 全历史拉取不完整，已阻断写入".into(),
        ));
    }
    let report = sync.execute(&W2Config::mock_write(), &dev_requests(scenario))?;
    Ok(DevW2Run { pull, report })
}

/// 保留 `run_mock_batch` 的入口测试：应用层只允许显式 Mock 配置。
pub fn dev_run_mock_batch(store: &WpsStore<'_>, scenario: DevW2Scenario) -> Result<W2BatchReport> {
    run_mock_batch(
        store,
        DevW2Transport::new(scenario),
        &dev_requests(scenario),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{schema, Database};
    use crate::services::wps_w2::W2WriteStatus;
    use rusqlite::Connection;

    fn store() -> (Database, ()) {
        let conn = Connection::open_in_memory().expect("open memory db");
        schema::init(&conn).expect("init schema");
        (Database::new(conn), ())
    }

    #[test]
    fn dev_normal_write_succeeds_and_links() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_write(&store, DevW2Scenario::Normal).expect("run");
        assert!(run.pull.complete);
        assert_eq!(run.report.outcomes.len(), 2);
        assert!(run
            .report
            .outcomes
            .iter()
            .all(|o| o.status == W2WriteStatus::Succeeded));
        assert!(run.report.outcomes.iter().all(|o| o.link_id.is_some()));
        assert!(!run.report.stopped);
    }

    #[test]
    fn dev_timeout_stops_and_marks_third_not_attempted() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_write(&store, DevW2Scenario::TimeoutSecond).expect("run");
        let statuses: Vec<_> = run.report.outcomes.iter().map(|o| o.status).collect();
        assert_eq!(
            statuses,
            vec![
                W2WriteStatus::Succeeded,
                W2WriteStatus::Unknown,
                W2WriteStatus::NotAttempted
            ]
        );
        assert!(run.report.stopped);
        assert!(run.report.outcomes[1]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("TIMEOUT")));
    }

    #[test]
    fn dev_missing_reference_rejects_before_any_write() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_write(&store, DevW2Scenario::MissingReference).expect("run");
        assert_eq!(run.report.outcomes.len(), 1);
        assert_eq!(run.report.outcomes[0].status, W2WriteStatus::Rejected);
        assert!(run.report.stopped);
        assert!(run.report.outcomes[0]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("WPS_E_REF_STALE") || e.contains("WPS_E_REF_MISSING")));
    }

    #[test]
    fn dev_second_run_rejects_existing_idempotency_key() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let first = dev_write(&store, DevW2Scenario::Normal).expect("first");
        assert_eq!(first.report.outcomes.len(), 2);
        let second = dev_write(&store, DevW2Scenario::Normal).expect("second");
        assert!(second
            .report
            .outcomes
            .iter()
            .all(|o| o.status == W2WriteStatus::Rejected));
        assert!(second.report.outcomes.iter().all(|o| o
            .error
            .as_deref()
            .is_some_and(|e| e.contains("WPS_E_IDEMPOTENCY_STATE"))));
    }

    #[test]
    fn dev_timeout_persists_intent_before_each_attempt_and_stops_third() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_write(&store, DevW2Scenario::TimeoutSecond).expect("run");
        assert_eq!(run.report.outcomes[1].status, W2WriteStatus::Unknown);
        let intents: Vec<(i64, String, String)> = db
            .with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT sequence, state, COALESCE(remote_record_id, '') FROM wps_write_intents ORDER BY sequence",
                )?;
                let rows = stmt
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .expect("read intents");
        assert_eq!(intents.len(), 2, "第三笔不应落意图");
        assert_eq!(intents[0].1, "linked");
        assert_eq!(intents[1].1, "unknown");
        assert_eq!(intents[0].2, "dev-03-1");
        assert!(intents[1].2.is_empty());
    }

    #[test]
    fn dev_normal_writes_unclaimed_link_and_never_posts() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_write(&store, DevW2Scenario::Normal).expect("run");
        let link_id = run.report.outcomes[0].link_id.expect("link");
        assert_eq!(
            store.link_state(link_id).expect("state").as_deref(),
            Some("unclaimed")
        );
        assert!(!run.report.outcomes.iter().any(|o| o.link_id.is_none()));
    }

    #[test]
    fn dev_scenario_parse_rejects_unknown() {
        assert_eq!(
            DevW2Scenario::parse(Some("normal")).expect("normal"),
            DevW2Scenario::Normal
        );
        assert_eq!(
            DevW2Scenario::parse(Some("timeout")).expect("timeout"),
            DevW2Scenario::TimeoutSecond
        );
        assert!(DevW2Scenario::parse(Some("real_wps")).is_err());
    }

    #[test]
    fn dev_pull_only_does_not_need_write_session() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let pull = dev_pull(&store).expect("pull");
        assert!(pull.complete);
        assert_eq!(pull.sheets.len(), 11);
    }
}
