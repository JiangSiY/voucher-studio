//! W2 写入预检层（干跑，v0.28）。
//!
//! 本模块只回答「如果现在提交这批创建请求，预检会不会放行」，**不发起任何远端
//! 请求、不落任何本地同步表**。所有判定都委托给 [`W2Sync::preflight`]，避免在
//! 这里复制第二套规则或第二个写入口（C5 阻断维持）。
//!
//! 允许的用法只有 [`WpsCreateFlow::dry_run`]；本模块不提供 `create_*` 之类的
//! 落库路径，真正的写入仍只由 `W2Sync::execute` 承担。

use crate::services::wps::{WpsClient, WpsStore, WpsTransport};
use crate::services::wps_w2::{W2Config, W2CreateRequest, W2Sync};
use crate::Result;

/// 单条创建请求在干跑阶段的预判结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WpsCreateStatus {
    /// 预检放行；真正提交时仍会重新过一遍预检。
    WouldCreate,
    /// 预检拒绝，整批不进入写入。
    Rejected,
}

/// 单条请求的干跑结论；`code`/`message` 只在拒绝时出现。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WpsCreateOutcome {
    pub idempotency_key: String,
    pub status: WpsCreateStatus,
    pub code: Option<&'static str>,
    pub message: Option<String>,
}

/// 整批干跑结果；`accepted=false` 表示整批被预检拒绝。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WpsCreateDryRun {
    pub outcomes: Vec<WpsCreateOutcome>,
    pub accepted: bool,
}

/// 写入预检的干跑入口；持有真实写入所需的同一对 client/store，但只读。
pub struct WpsCreateFlow<'a, T: WpsTransport> {
    client: &'a WpsClient<T>,
    store: &'a WpsStore<'a>,
}

impl<'a, T: WpsTransport> WpsCreateFlow<'a, T> {
    pub fn new(client: &'a WpsClient<T>, store: &'a WpsStore<'a>) -> Self {
        Self { client, store }
    }

    /// 整批干跑：复用 [`W2Sync::preflight`]，逐条给出结论。
    ///
    /// 预检拒绝时整批标记为 `Rejected`（与 `execute` 的全批拒绝语义一致）；
    /// 放行时逐条标记 `WouldCreate`。两条路径都不触碰远端与本地同步表。
    pub fn dry_run(
        &self,
        config: &W2Config,
        requests: &[W2CreateRequest],
    ) -> Result<WpsCreateDryRun> {
        let preflight = W2Sync::new(self.client, self.store).preflight(config, requests);
        match preflight {
            Ok(_) => Ok(WpsCreateDryRun {
                outcomes: requests
                    .iter()
                    .map(|request| WpsCreateOutcome {
                        idempotency_key: request.idempotency_key.clone(),
                        status: WpsCreateStatus::WouldCreate,
                        code: None,
                        message: None,
                    })
                    .collect(),
                accepted: true,
            }),
            Err(reject) => Ok(WpsCreateDryRun {
                outcomes: requests
                    .iter()
                    .map(|request| WpsCreateOutcome {
                        idempotency_key: request.idempotency_key.clone(),
                        status: WpsCreateStatus::Rejected,
                        code: Some(reject.code),
                        message: Some(reject.message.clone()),
                    })
                    .collect(),
                accepted: false,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{schema, Database};
    use crate::services::wps::MockTransport;
    use crate::services::wps_w2::{W2Config, W2CreateRequest};
    use rusqlite::Connection;
    use serde_json::json;

    /// 干跑前后必须零增量的四张本地同步表。
    const SYNC_TABLES: [&str; 4] = [
        "wps_write_intents",
        "wps_links",
        "wps_idempotency_keys",
        "wps_sync_runs",
    ];

    fn db_with_schema() -> Database {
        let conn = Connection::open_in_memory().expect("open memory db");
        schema::init(&conn).expect("init schema");
        Database::new(conn)
    }

    fn table_counts(store: &WpsStore<'_>) -> [i64; 4] {
        let mut counts = [0_i64; 4];
        store
            .database()
            .with_conn(|conn| {
                for (index, table) in SYNC_TABLES.iter().enumerate() {
                    counts[index] =
                        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                            row.get(0)
                        })?;
                }
                Ok(())
            })
            .expect("count sync tables");
        counts
    }

    fn request(key: &str, date: &str) -> W2CreateRequest {
        let mut request =
            W2CreateRequest::new(3, "03", json!({"应收单号": key}), format!("idem-{key}"));
        request.business_date = Some(date.to_string());
        request
    }

    fn assert_no_side_effects(
        client: &WpsClient<MockTransport>,
        store: &WpsStore<'_>,
        before: [i64; 4],
    ) {
        assert!(
            client.transport().calls().is_empty(),
            "干跑不得产生远端请求，实际 {:?}",
            client.transport().calls()
        );
        assert_eq!(table_counts(store), before, "干跑不得写本地同步表");
    }

    #[test]
    fn dry_run_accepts_valid_batch_without_side_effects() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
        let flow = WpsCreateFlow::new(&client, &store);
        let before = table_counts(&store);

        let dry_run = flow
            .dry_run(&W2Config::mock_write(), &[request("AR-OK", "2026-11-01")])
            .expect("dry run");

        assert!(dry_run.accepted);
        assert_eq!(dry_run.outcomes.len(), 1);
        assert_eq!(dry_run.outcomes[0].status, WpsCreateStatus::WouldCreate);
        assert_eq!(dry_run.outcomes[0].idempotency_key, "idem-AR-OK");
        assert_eq!(dry_run.outcomes[0].code, None);
        assert_eq!(dry_run.outcomes[0].message, None);
        assert_no_side_effects(&client, &store, before);
    }

    #[test]
    fn dry_run_rejects_empty_business_key_without_side_effects() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
        let flow = WpsCreateFlow::new(&client, &store);
        let before = table_counts(&store);
        let mut empty = W2CreateRequest::new(
            3,
            "03",
            json!({"客户": "客户-1"}),
            "idem-empty-business-key",
        );
        empty.business_date = Some("2026-11-01".into());

        let dry_run = flow
            .dry_run(&W2Config::mock_write(), &[empty])
            .expect("dry run");

        assert!(!dry_run.accepted);
        assert_eq!(dry_run.outcomes[0].status, WpsCreateStatus::Rejected);
        assert_eq!(dry_run.outcomes[0].code, Some("WPS_E_BUSINESS_KEY_MISSING"));
        assert_no_side_effects(&client, &store, before);
    }

    #[test]
    fn dry_run_rejects_duplicate_business_keys_in_one_batch() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
        let flow = WpsCreateFlow::new(&client, &store);
        let before = table_counts(&store);
        let mut second = request("AR-DUP", "2026-11-01");
        second.idempotency_key = "idem-AR-DUP-2".into();

        let dry_run = flow
            .dry_run(
                &W2Config::mock_write(),
                &[request("AR-DUP", "2026-11-01"), second],
            )
            .expect("dry run");

        assert!(!dry_run.accepted);
        assert_eq!(dry_run.outcomes.len(), 2);
        assert_eq!(dry_run.outcomes[0].code, Some("WPS_E_BATCH_DUPLICATE"));
        assert_eq!(dry_run.outcomes[1].code, Some("WPS_E_BATCH_DUPLICATE"));
        assert_no_side_effects(&client, &store, before);
    }

    #[test]
    fn dry_run_rejects_dedupe_only_sheets_05_and_07() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
        let flow = WpsCreateFlow::new(&client, &store);
        let before = table_counts(&store);

        for (sheet_id, record_type, fields) in [
            (5, "05", json!({"调整编号": "ADJ-1"})),
            (7, "07", json!({"发票号": "INV-1"})),
        ] {
            let mut request =
                W2CreateRequest::new(sheet_id, record_type, fields, format!("idem-{record_type}"));
            request.business_date = Some("2026-11-01".into());
            let dry_run = flow
                .dry_run(&W2Config::mock_write(), &[request])
                .expect("dry run");
            assert!(!dry_run.accepted, "表 {record_type} 不得开放写入");
            assert_eq!(
                dry_run.outcomes[0].code,
                Some("WPS_E_DEDUPE_ONLY"),
                "表 {record_type} 必须走既有查重专用码"
            );
            assert_no_side_effects(&client, &store, before);
        }
    }

    #[test]
    fn dry_run_rejects_reference_only_sheet_without_side_effects() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
        let flow = WpsCreateFlow::new(&client, &store);
        let before = table_counts(&store);
        let mut base = W2CreateRequest::new(1, "01", json!({"名称": "客户-1"}), "idem-REF-1");
        base.business_date = Some("2026-11-01".into());

        let dry_run = flow
            .dry_run(&W2Config::mock_write(), &[base])
            .expect("dry run");

        assert!(!dry_run.accepted);
        assert_eq!(dry_run.outcomes[0].code, Some("WPS_E_REFERENCE_ONLY"));
        assert_no_side_effects(&client, &store, before);
    }

    /// 静态判据：本模块不得出现任何写入语句或写入口符号。
    ///
    /// 关键字按片段拼接，避免检查代码自身把关键字写进被检查的源文本。
    #[test]
    fn module_source_contains_no_write_path() {
        let source = include_str!("wps_create.rs");
        for (label, needle) in [
            ("片段1", ["IN", "SERT"].concat()),
            ("片段2", ["UP", "DATE"].concat()),
            ("片段3", ["DE", "LETE"].concat()),
            ("片段4", ["start", "_run"].concat()),
            ("片段5", ["set", "_state"].concat()),
            ("片段6", ["create", "_record"].concat()),
        ] {
            assert!(
                !source.contains(&needle),
                "wps_create.rs 不得包含写入符号（{label}）"
            );
        }
    }
}
