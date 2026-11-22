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
    run_mock_batch, run_strict_mock_batch, W2BatchReport, W2Config, W2CreateRequest, W2PullReport,
    W2Sync,
};
use crate::{CoreError, Result};

/// 开发 Mock 的合成授权状态键（存在 `wps_state`，随临时库持久化）。
pub const DEV_AUTH_EXPIRED_KEY: &str = "dev_w2_auth_expired";

/// 可复跑的合成场景：正常两笔、第二笔超时、缺引用整批拒绝、
/// 授权失效（400006）、限流（429），以及走严格写入入口被 C5 阻断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevW2Scenario {
    Normal,
    TimeoutSecond,
    MissingReference,
    AuthExpired,
    RateLimited,
    /// 严格入口：11 表扫描完成但无快照证据（C5），写入在预检前被阻断、零 create。
    C5Blocked,
}

impl DevW2Scenario {
    pub fn parse(raw: Option<&str>) -> Result<Self> {
        match raw.unwrap_or("normal").trim().to_ascii_lowercase().as_str() {
            "normal" => Ok(Self::Normal),
            "timeout_second" | "timeout" => Ok(Self::TimeoutSecond),
            "missing_reference" | "missing_ref" => Ok(Self::MissingReference),
            "auth_expired" | "auth" => Ok(Self::AuthExpired),
            "rate_limited" | "rate_limit" | "rate" => Ok(Self::RateLimited),
            "c5_blocked" | "c5" => Ok(Self::C5Blocked),
            other => Err(CoreError::Invalid(format!(
                "未知 W2 开发场景: {other}（可用 normal/timeout_second/missing_reference/auth_expired/rate_limited/c5_blocked）"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::TimeoutSecond => "timeout_second",
            Self::MissingReference => "missing_reference",
            Self::AuthExpired => "auth_expired",
            Self::RateLimited => "rate_limited",
            Self::C5Blocked => "c5_blocked",
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

/// 一次开发 Mock 调用的 transport 计数（含只读拉取），用于走查核对「零请求」。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DevW2MockCounts {
    pub total: usize,
    pub list: usize,
    pub create: usize,
    pub create_keys: Vec<String>,
}

/// 一次合成调用的结果；`pull` 为 `None` 表示前置阻断（授权失效/冷却中）而未拉取。
pub struct DevW2Run {
    pub pull: Option<W2PullReport>,
    pub report: Option<W2BatchReport>,
    pub mock: DevW2MockCounts,
}

/// 合成 transport：11 张表只返回预置基础资料；create 按场景成功/超时。
pub struct DevW2Transport {
    scenario: DevW2Scenario,
    create_calls: Mutex<usize>,
    counts: Arc<Mutex<DevW2MockCounts>>,
}

impl DevW2Transport {
    pub fn new(scenario: DevW2Scenario) -> Self {
        Self::with_counts(scenario, Arc::new(Mutex::new(DevW2MockCounts::default())))
    }

    fn with_counts(scenario: DevW2Scenario, counts: Arc<Mutex<DevW2MockCounts>>) -> Self {
        Self {
            scenario,
            create_calls: Mutex::new(0),
            counts,
        }
    }

    fn record(&self, list: bool, key: Option<&str>) -> std::result::Result<(), WpsError> {
        let mut counts = self
            .counts
            .lock()
            .map_err(|_| WpsError::Protocol("开发 mock 计数锁中毒".into()))?;
        counts.total += 1;
        if list {
            counts.list += 1;
        } else {
            counts.create += 1;
            if let Some(key) = key {
                counts.create_keys.push(key.to_string());
            }
        }
        Ok(())
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
                self.record(true, None)?;
                let sheet_id = args
                    .get("sheet_id")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| WpsError::Protocol("开发 mock 缺少 sheet_id".into()))?;
                Ok(Self::reference_page(sheet_id))
            }
            "dbsheet.create_records" => {
                let key = args
                    .get("idempotency_key")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                self.record(false, key.as_deref())?;
                let mut calls = self
                    .create_calls
                    .lock()
                    .map_err(|_| WpsError::Protocol("开发 mock 计数锁中毒".into()))?;
                let index = *calls;
                *calls += 1;
                if self.scenario == DevW2Scenario::TimeoutSecond && index == 1 {
                    return Err(WpsError::Timeout);
                }
                if self.scenario == DevW2Scenario::AuthExpired && index == 0 {
                    return Err(WpsError::AuthExpired);
                }
                if self.scenario == DevW2Scenario::RateLimited && index == 0 {
                    return Err(WpsError::RateLimited);
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
        DevW2Scenario::AuthExpired => vec![
            request(
                "AR-DEV-A1",
                "开发模拟应收-授权失效1",
                refs.clone(),
                ref_ids.clone(),
            ),
            request("AR-DEV-A2", "开发模拟应收-授权失效2", refs, ref_ids),
        ],
        DevW2Scenario::RateLimited => vec![
            request(
                "AR-DEV-R1",
                "开发模拟应收-限流1",
                refs.clone(),
                ref_ids.clone(),
            ),
            request("AR-DEV-R2", "开发模拟应收-限流2", refs, ref_ids),
        ],
        DevW2Scenario::C5Blocked => {
            vec![request("AR-DEV-C51", "开发模拟应收-C5阻断", refs, ref_ids)]
        }
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
pub fn dev_pull(store: &WpsStore<'_>) -> Result<DevW2Run> {
    let counts = Arc::new(Mutex::new(DevW2MockCounts::default()));
    let transport = DevW2Transport::with_counts(DevW2Scenario::Normal, Arc::clone(&counts));
    let client =
        WpsClient::new(transport, "mock://w2-dev").with_credentials(Arc::new(DevW2Credentials));
    let sync = W2Sync::new(&client, store);
    let pull = sync.pull_all_history()?;
    let mock = counts
        .lock()
        .map_err(|_| CoreError::Invalid("开发 mock 计数锁中毒".into()))?
        .clone();
    Ok(DevW2Run {
        pull: Some(pull),
        report: None,
        mock,
    })
}

/// 合成授权是否处于失效态（存 `wps_state`，随同一临时库跨进程保留）。
pub fn dev_auth_expired(store: &WpsStore<'_>) -> Result<bool> {
    Ok(store
        .get_state(DEV_AUTH_EXPIRED_KEY)?
        .as_deref()
        .is_some_and(|v| v == "1"))
}

/// 本地合成授权恢复：只清开发态标记，不触发任何网络或凭据读取。
pub fn dev_auth_recover(store: &WpsStore<'_>) -> Result<()> {
    store.set_state(DEV_AUTH_EXPIRED_KEY, "0")?;
    Ok(())
}

/// 前置阻断批次：全部整批拒绝、零 transport 请求，但仍保留 `auth_required`/`cooldown_until`。
fn blocked_report(
    requests: &[W2CreateRequest],
    auth_required: bool,
    cooldown_until: Option<String>,
    message: &str,
) -> W2BatchReport {
    W2BatchReport {
        run_id: None,
        outcomes: requests
            .iter()
            .map(|request| crate::services::wps_w2::W2Outcome {
                idempotency_key: request.idempotency_key.clone(),
                status: crate::services::wps_w2::W2WriteStatus::Rejected,
                remote_record_id: None,
                link_id: None,
                error: Some(message.to_string()),
            })
            .collect(),
        stopped: true,
        auth_required,
        cooldown_until,
    }
}

/// 显式测试写：固定 Mock 配置，先全历史拉取，再整批预检/逐条创建。
///
/// 两种前置阻断都在拉取之前短路，保证「恢复授权前」与「冷却期内」再次点击时
/// transport 请求数为 0；待处理意图留在库里，不会自动重发。
pub fn dev_write(store: &WpsStore<'_>, scenario: DevW2Scenario) -> Result<DevW2Run> {
    let requests = dev_requests(scenario);
    let counts = Arc::new(Mutex::new(DevW2MockCounts::default()));

    // C5 场景走严格写入入口：拉取阶段即被快照门阻断，不进入预检/写入。
    if scenario == DevW2Scenario::C5Blocked {
        let strict = run_strict_mock_batch(
            store,
            DevW2Transport::with_counts(scenario, Arc::clone(&counts)),
            &requests,
        )?;
        let mock = counts
            .lock()
            .map_err(|_| CoreError::Invalid("开发 mock 计数锁中毒".into()))?
            .clone();
        return Ok(DevW2Run {
            pull: Some(strict.pull),
            report: strict.report,
            mock,
        });
    }

    let transport = DevW2Transport::with_counts(scenario, Arc::clone(&counts));
    let client =
        WpsClient::new(transport, "mock://w2-dev").with_credentials(Arc::new(DevW2Credentials));
    let sync = W2Sync::new(&client, store);

    if dev_auth_expired(store)? {
        return Ok(DevW2Run {
            pull: None,
            report: Some(blocked_report(
                &requests,
                true,
                None,
                "WPS_E_AUTH: 授权失效，已停止批量写入；该条保留待处理",
            )),
            mock: DevW2MockCounts::default(),
        });
    }
    if let Some(until) = sync.active_rate_limit()? {
        let cooldown = until.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        return Ok(DevW2Run {
            pull: None,
            report: Some(blocked_report(
                &requests,
                false,
                Some(cooldown.clone()),
                &format!("WPS_E_RATE: 限流，已停止批量写入并冷却至 {cooldown}"),
            )),
            mock: DevW2MockCounts::default(),
        });
    }

    let pull = sync.pull_all_history()?;
    if pull.writes_blocked {
        return Err(CoreError::Invalid(
            "WPS_E_PULL_BLOCKED: 开发 mock 全历史拉取不完整，已阻断写入".into(),
        ));
    }
    let report = sync.execute(&W2Config::mock_write(), &requests)?;
    if scenario == DevW2Scenario::AuthExpired && report.auth_required {
        store.set_state(DEV_AUTH_EXPIRED_KEY, "1")?;
    }
    let mock = counts
        .lock()
        .map_err(|_| CoreError::Invalid("开发 mock 计数锁中毒".into()))?
        .clone();
    Ok(DevW2Run {
        pull: Some(pull),
        report: Some(report),
        mock,
    })
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

    fn pull_of(run: &DevW2Run) -> &W2PullReport {
        run.pull.as_ref().expect("pull")
    }

    fn report_of(run: &DevW2Run) -> &W2BatchReport {
        run.report.as_ref().expect("report")
    }

    fn key_status(db: &Database, key: &str) -> String {
        db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT local_status FROM wps_idempotency_keys WHERE idempotency_key = ?1",
            )?;
            Ok(stmt.query_row([key], |row| row.get(0))?)
        })
        .expect("idempotency status")
    }

    fn intent_state(db: &Database, key: &str) -> String {
        db.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT state FROM wps_write_intents WHERE idempotency_key = ?1")?;
            Ok(stmt.query_row([key], |row| row.get(0))?)
        })
        .expect("intent state")
    }

    #[test]
    fn dev_normal_write_succeeds_and_links() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_write(&store, DevW2Scenario::Normal).expect("run");
        assert!(pull_of(&run).complete);
        let report = report_of(&run);
        assert_eq!(report.outcomes.len(), 2);
        assert!(report
            .outcomes
            .iter()
            .all(|o| o.status == W2WriteStatus::Succeeded));
        assert!(report.outcomes.iter().all(|o| o.link_id.is_some()));
        assert!(!report.stopped);
        assert_eq!(run.mock.create, 2);
        assert_eq!(run.mock.list, 11);
    }

    #[test]
    fn dev_timeout_stops_and_marks_third_not_attempted() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_write(&store, DevW2Scenario::TimeoutSecond).expect("run");
        let report = report_of(&run);
        let statuses: Vec<_> = report.outcomes.iter().map(|o| o.status).collect();
        assert_eq!(
            statuses,
            vec![
                W2WriteStatus::Succeeded,
                W2WriteStatus::Unknown,
                W2WriteStatus::NotAttempted
            ]
        );
        assert!(report.stopped);
        assert!(report.outcomes[1]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("TIMEOUT")));
    }

    #[test]
    fn dev_missing_reference_rejects_before_any_write() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_write(&store, DevW2Scenario::MissingReference).expect("run");
        let report = report_of(&run);
        assert_eq!(report.outcomes.len(), 1);
        assert_eq!(report.outcomes[0].status, W2WriteStatus::Rejected);
        assert!(report.stopped);
        assert!(report.outcomes[0]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("WPS_E_REF_STALE") || e.contains("WPS_E_REF_MISSING")));
        assert_eq!(run.mock.create, 0, "预检失败不得发出 create 请求");
    }

    #[test]
    fn dev_second_run_rejects_existing_idempotency_key() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let first = dev_write(&store, DevW2Scenario::Normal).expect("first");
        assert_eq!(report_of(&first).outcomes.len(), 2);
        let second = dev_write(&store, DevW2Scenario::Normal).expect("second");
        let report = report_of(&second);
        assert!(report
            .outcomes
            .iter()
            .all(|o| o.status == W2WriteStatus::Rejected));
        assert!(report.outcomes.iter().all(|o| o
            .error
            .as_deref()
            .is_some_and(|e| e.contains("WPS_E_IDEMPOTENCY_STATE"))));
    }

    #[test]
    fn dev_timeout_persists_intent_before_each_attempt_and_stops_third() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        dev_write(&store, DevW2Scenario::TimeoutSecond).expect("run");
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
        let link_id = report_of(&run).outcomes[0].link_id.expect("link");
        assert_eq!(
            store.link_state(link_id).expect("state").as_deref(),
            Some("unclaimed")
        );
        assert!(!report_of(&run).outcomes.iter().any(|o| o.link_id.is_none()));
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
        assert_eq!(
            DevW2Scenario::parse(Some("auth")).expect("auth"),
            DevW2Scenario::AuthExpired
        );
        assert_eq!(
            DevW2Scenario::parse(Some("rate_limit")).expect("rate"),
            DevW2Scenario::RateLimited
        );
        assert!(DevW2Scenario::parse(Some("real_wps")).is_err());
    }

    #[test]
    fn dev_pull_only_does_not_need_write_session() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_pull(&store).expect("pull");
        assert!(pull_of(&run).complete);
        assert_eq!(pull_of(&run).sheets.len(), 11);
        assert!(run.report.is_none());
        assert_eq!(run.mock.create, 0);
        assert_eq!(run.mock.list, 11);
    }

    #[test]
    fn dev_auth_expired_stops_first_create_and_keeps_intent_pending() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_write(&store, DevW2Scenario::AuthExpired).expect("run");
        let report = report_of(&run);
        assert!(report.auth_required, "400006 必须回填 auth_required");
        assert!(report.cooldown_until.is_none());
        assert!(report.stopped);
        assert_eq!(
            report.outcomes[0].status,
            W2WriteStatus::Unknown,
            "授权失效的那一条是结果不明，不是成功"
        );
        assert!(report.outcomes[0]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("WPS_E_AUTH")));
        assert_eq!(report.outcomes[1].status, W2WriteStatus::NotAttempted);
        assert_eq!(run.mock.create, 1);
        assert_eq!(run.mock.create_keys, vec!["AR-DEV-A1".to_string()]);
        assert_eq!(intent_state(&db, "AR-DEV-A1"), "unknown");
        assert_eq!(
            key_status(&db, "AR-DEV-A1"),
            "pending",
            "待处理键不得标记为成功"
        );
        assert!(dev_auth_expired(&store).expect("state"));
    }

    #[test]
    fn dev_auth_blocks_next_write_with_zero_mock_requests() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        dev_write(&store, DevW2Scenario::AuthExpired).expect("auth");
        let blocked = dev_write(&store, DevW2Scenario::Normal).expect("blocked");
        assert!(blocked.pull.is_none(), "前置阻断不应再拉取");
        let report = report_of(&blocked);
        assert!(report.auth_required);
        assert!(report.stopped);
        assert!(report
            .outcomes
            .iter()
            .all(|o| o.status == W2WriteStatus::Rejected));
        assert_eq!(blocked.mock.total, 0, "授权未恢复时不得发出任何请求");
        assert!(blocked.mock.create_keys.is_empty());
        assert_eq!(intent_state(&db, "AR-DEV-A1"), "unknown");
        assert_eq!(key_status(&db, "AR-DEV-A1"), "pending");
    }

    #[test]
    fn dev_auth_recover_allows_new_batch_without_retrying_pending() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        dev_write(&store, DevW2Scenario::AuthExpired).expect("auth");
        dev_auth_recover(&store).expect("recover");
        assert!(!dev_auth_expired(&store).expect("state"));
        let run = dev_write(&store, DevW2Scenario::Normal).expect("normal");
        let report = report_of(&run);
        assert!(report
            .outcomes
            .iter()
            .all(|o| o.status == W2WriteStatus::Succeeded));
        assert!(
            !run.mock.create_keys.iter().any(|k| k == "AR-DEV-A1"),
            "恢复后新批次不得自动重发待处理键：{:?}",
            run.mock.create_keys
        );
        assert_eq!(run.mock.create_keys, vec!["AR-DEV-001", "AR-DEV-002"]);
        assert_eq!(intent_state(&db, "AR-DEV-A1"), "unknown");
        assert_eq!(key_status(&db, "AR-DEV-A1"), "pending");
    }

    #[test]
    fn dev_rate_limited_sets_cooldown_matching_report() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_write(&store, DevW2Scenario::RateLimited).expect("run");
        let report = report_of(&run);
        assert!(!report.auth_required);
        let until = report.cooldown_until.clone().expect("cooldown");
        assert!(until.len() > 19, "冷却截止应为 RFC3339：{until}");
        assert!(report.outcomes[0]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("WPS_E_RATE") && e.contains(&until)));
        assert!(report.stopped);
        assert_eq!(run.mock.create, 1);
        assert_eq!(intent_state(&db, "AR-DEV-R1"), "unknown");
        assert_eq!(key_status(&db, "AR-DEV-R1"), "pending");
    }

    #[test]
    fn dev_rate_limited_second_click_issues_zero_mock_requests() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let first = dev_write(&store, DevW2Scenario::RateLimited).expect("first");
        let until = report_of(&first).cooldown_until.clone().expect("cooldown");
        let second = dev_write(&store, DevW2Scenario::Normal).expect("second");
        assert!(second.pull.is_none());
        let report = report_of(&second);
        assert_eq!(report.cooldown_until.as_deref(), Some(until.as_str()));
        assert!(report
            .outcomes
            .iter()
            .all(|o| o.status == W2WriteStatus::Rejected));
        assert_eq!(second.mock.total, 0, "冷却期内不得发出任何请求");
        assert_eq!(second.mock.create, 0);
    }

    #[test]
    fn dev_c5_blocked_strict_entry_returns_c5_reason_with_zero_create() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_write(&store, DevW2Scenario::C5Blocked).expect("run");
        let pull = pull_of(&run);
        // 扫描确实发起且 11 表全部完成；阻断原因是快照不可验证（C5），不是分页缺失。
        assert!(pull.complete, "C5 场景应扫描完整");
        assert!(!pull.snapshot_verified);
        assert!(pull.writes_blocked);
        let reason = pull.blocked_reason.as_deref().expect("blocked reason");
        assert!(
            reason.contains("WPS_E_PULL_SNAPSHOT_UNVERIFIED"),
            "{reason}"
        );
        assert!(run.report.is_none(), "阻断发生在预检之前，不应有批次结果");
        assert_eq!(run.mock.list, 11);
        assert_eq!(run.mock.create, 0, "C5 阻断后不得发出 create 请求");
        assert!(run.mock.create_keys.is_empty());
        // 阻断不落 run 行、不落意图，库保持干净。
        let runs: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row("SELECT COUNT(*) FROM wps_sync_runs", [], |r| r.get(0))?)
            })
            .expect("count runs");
        assert_eq!(runs, 0);
        let intents: i64 = db
            .with_conn(|conn| {
                Ok(conn.query_row("SELECT COUNT(*) FROM wps_write_intents", [], |r| r.get(0))?)
            })
            .expect("count intents");
        assert_eq!(intents, 0);
    }

    #[test]
    fn dev_c5_control_normal_write_records_create_requests() {
        let (db, _) = store();
        let store = WpsStore::from_db(&db);
        let run = dev_write(&store, DevW2Scenario::Normal).expect("run");
        assert!(report_of(&run).outcomes.len() == 2);
        assert_eq!(run.mock.list, 11);
        assert_eq!(run.mock.create, 2, "对照场景证明计数能捕获 create");
        assert_eq!(
            run.mock.create_keys,
            vec!["AR-DEV-001".to_string(), "AR-DEV-002".to_string()]
        );
    }
}
