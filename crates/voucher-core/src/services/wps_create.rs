//! W2 写入层（干跑预检 + 受守卫约束的逐条写入，v0.28）。
//!
//! 两条路径共用同一对 client/store：
//! - [`WpsCreateFlow::dry_run`]：只调用既有 [`W2Sync::preflight`]，回答「整批会不会
//!   放行」；不发远端请求、不写本地同步表。
//! - [`WpsCreateFlow::create_doc_records`] / [`WpsCreateFlow::create_meter_records`]：
//!   三段式写入——守卫取证（只读）→ 逐条 `create_record` → 回填 `wps_links`。
//!
//! 守卫是只读的，09 门与单号门**并列**：各自入口只调用自己的守卫，互不嵌套、互不
//! 调用。写入段只经 [`WpsWriteSession::create_records`] 逐条发起，不使用 WPS 原生
//! 批量创建；部分失败不回滚已成功记录，回填失败标记 `PendingBackfill` 且不吞错。
//! 所有调用都跑在 `transport=mock`；本模块不接 Tauri/CLI，也不触真实端点。

use std::collections::HashSet;

use serde_json::Value;

use crate::services::wps::{
    KeyedRecord, LinkWriter, WpsClient, WpsStore, WpsTransport, WpsWriteSession, WriteStatus,
};
use crate::services::wps_09::Wps09CreateGuard;
use crate::services::wps_doc_guard::{WpsDocNoCreateGuard, WpsDocNoSpec};
use crate::services::wps_index::business_key_for;
use crate::services::wps_w2::{
    W2Config, W2CreateRequest, W2Outcome, W2Sync, W2WriteStatus, SHEET_DEDUPE_ONLY,
};
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

/// 单条待写入记录：字段值 + 该条独立的幂等键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WpsCreateInput {
    pub fields: Value,
    pub idempotency_key: String,
}

impl WpsCreateInput {
    pub fn new(fields: Value, idempotency_key: impl Into<String>) -> Self {
        Self {
            fields,
            idempotency_key: idempotency_key.into(),
        }
    }
}

/// W2 写入层的唯一入口：持有同一对 client/store。
///
/// `dry_run` 只读（不碰远端与本地表）；`create_*` 先过守卫，再逐条写入并回填关联。
pub struct WpsCreateFlow<'a, T: WpsTransport> {
    client: &'a WpsClient<T>,
    store: &'a WpsStore<'a>,
    /// 回填缝：生产恒为同一个 `store`；测试可换成故障桩（见 `LinkWriter`）。
    links: &'a dyn LinkWriter,
}

impl<'a, T: WpsTransport> WpsCreateFlow<'a, T> {
    pub fn new(client: &'a WpsClient<T>, store: &'a WpsStore<'a>) -> Self {
        Self {
            client,
            store,
            links: store,
        }
    }

    /// 测试专用：把回填换成注入实现（生产构造器不暴露该缝）。
    #[cfg(test)]
    pub(crate) fn with_link_writer(
        client: &'a WpsClient<T>,
        store: &'a WpsStore<'a>,
        links: &'a dyn LinkWriter,
    ) -> Self {
        Self {
            client,
            store,
            links,
        }
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

    /// 单号表的三段式写入（02/03/04/06/08/10/11，05/07 维持只查重）。
    ///
    /// - 阶段 1（只读）：业务键单一真源 [`business_key_for`]；同批重复键本地拒绝；
    ///   每条再调 [`WpsDocNoCreateGuard::ensure_create_allowed`]。任一条被拒，整批
    ///   不进入写入段——远端 0 次写请求、`wps_links` 与同步表零增量。
    /// - 阶段 2：只经 [`WpsWriteSession::create_records`] 逐条创建，每条独立幂等键。
    /// - 阶段 3：只对 `Succeeded` 回填 `wps_links`；回填失败不吞错，该条标记
    ///   [`W2WriteStatus::PendingBackfill`] 并带 `WPS_E_LINK_BACKFILL`。
    pub fn create_doc_records(
        &self,
        spec: &WpsDocNoSpec,
        inputs: &[WpsCreateInput],
        run_id: i64,
    ) -> Vec<W2Outcome> {
        if inputs.is_empty() {
            return Vec::new();
        }
        // 05/07 首批只做全表查重、不生成业务行；与 W2 预检共用同一常量。
        if SHEET_DEDUPE_ONLY.contains(&spec.sheet_id) {
            return rejected_inputs(
                inputs,
                0,
                "WPS_E_DEDUPE_ONLY",
                format!("表 {} 首批仅全表查重，不生成业务行", spec.record_type),
            );
        }

        // 守卫按远端索引判重，看不到同批尚未写入的键，所以同批重复在本地拦截。
        let mut seen = HashSet::new();
        for (index, input) in inputs.iter().enumerate() {
            let (business_key, _) = business_key_for(spec.sheet_id, &input.fields);
            let business_key = business_key.trim().to_string();
            if business_key.is_empty() {
                continue;
            }
            if !seen.insert(business_key.clone()) {
                return rejected_inputs(
                    inputs,
                    index,
                    "WPS_E_DOC_DUPLICATE",
                    format!("同一批内业务键重复：{business_key}"),
                );
            }
        }

        for (index, input) in inputs.iter().enumerate() {
            let (business_key, _) = business_key_for(spec.sheet_id, &input.fields);
            if let Err(err) = WpsDocNoCreateGuard::ensure_create_allowed(
                self.client,
                self.store,
                spec,
                &business_key,
            ) {
                return rejected_inputs(inputs, index, err.code(), err.to_string());
            }
        }

        self.write_records(spec.sheet_id, spec.record_type, inputs, run_id)
    }

    /// 09 抄表的三段式写入；守卫与单号门并列，互不嵌套、互不调用。
    ///
    /// 09 的业务键是「电表记录 id + 期间」复合键，故单独入口，判定委托
    /// [`Wps09CreateGuard::ensure_create_allowed`]。
    pub fn create_meter_records(
        &self,
        meter_record_id: &str,
        period: &str,
        inputs: &[WpsCreateInput],
        run_id: i64,
    ) -> Vec<W2Outcome> {
        if inputs.is_empty() {
            return Vec::new();
        }
        for (index, _) in inputs.iter().enumerate() {
            if let Err(err) = Wps09CreateGuard::ensure_create_allowed(
                self.client,
                self.store,
                meter_record_id,
                period,
            ) {
                return rejected_inputs(inputs, index, err.code(), err.to_string());
            }
        }
        self.write_records(9, "09", inputs, run_id)
    }

    /// 阶段 2/3：逐条创建并回填关联；部分失败不回滚已成功记录。
    fn write_records(
        &self,
        sheet_id: i64,
        record_type: &str,
        inputs: &[WpsCreateInput],
        run_id: i64,
    ) -> Vec<W2Outcome> {
        let keyed: Vec<KeyedRecord<'_>> = inputs
            .iter()
            .map(|input| KeyedRecord::new(&input.fields, &input.idempotency_key))
            .collect();
        let written = WpsWriteSession::new(self.client, self.store, run_id).create_records(
            sheet_id,
            record_type,
            &keyed,
        );

        written
            .into_iter()
            .map(|outcome| {
                let mut mapped = W2Outcome {
                    idempotency_key: outcome.idempotency_key.clone(),
                    status: map_status(outcome.status),
                    remote_record_id: outcome.remote_record_id.clone(),
                    link_id: None,
                    error: outcome
                        .error
                        .as_ref()
                        .map(|err| format!("{}: {err}", err.code())),
                };
                if mapped.status != W2WriteStatus::Succeeded {
                    return mapped;
                }
                let Some(remote_id) = mapped.remote_record_id.clone() else {
                    return mapped;
                };
                let fields = inputs
                    .iter()
                    .find(|input| input.idempotency_key == mapped.idempotency_key)
                    .map(|input| input.fields.clone())
                    .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
                match self.links.write_link_row(
                    sheet_id,
                    &remote_id,
                    record_type,
                    &mapped.idempotency_key,
                    &fields,
                    outcome.remote_json.as_deref().unwrap_or("{}"),
                ) {
                    Ok(link_id) => mapped.link_id = Some(link_id),
                    Err(err) => {
                        mapped.status = W2WriteStatus::PendingBackfill;
                        mapped.error = Some(format!("WPS_E_LINK_BACKFILL: {err}"));
                    }
                }
                mapped
            })
            .collect()
    }
}

fn map_status(status: WriteStatus) -> W2WriteStatus {
    match status {
        WriteStatus::Succeeded => W2WriteStatus::Succeeded,
        WriteStatus::Unknown => W2WriteStatus::Unknown,
        WriteStatus::Rejected => W2WriteStatus::Rejected,
        WriteStatus::NotAttempted => W2WriteStatus::NotAttempted,
    }
}

/// 阶段 1 拒绝：命中项 `Rejected`，其余 `NotAttempted`，整批零请求零写入。
fn rejected_inputs(
    inputs: &[WpsCreateInput],
    index: usize,
    code: &str,
    message: String,
) -> Vec<W2Outcome> {
    inputs
        .iter()
        .enumerate()
        .map(|(position, input)| W2Outcome {
            idempotency_key: input.idempotency_key.clone(),
            status: if position == index {
                W2WriteStatus::Rejected
            } else {
                W2WriteStatus::NotAttempted
            },
            remote_record_id: None,
            link_id: None,
            error: (position == index).then(|| format!("{code}: {message}")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{schema, Database};
    use crate::services::wps::{MockTransport, WpsError};
    use crate::services::wps_doc_guard::doc_spec_for_sheet;
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

    /// 读取 wps_links：(record_id, state, voucher_id, doc_type)。
    fn link_rows(store: &WpsStore<'_>) -> Vec<(String, String, Option<String>, String)> {
        let mut rows = Vec::new();
        store
            .database()
            .with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT record_id, state, voucher_id, doc_type FROM wps_links ORDER BY id",
                )?;
                let iter = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?;
                for row in iter {
                    rows.push(row?);
                }
                Ok(())
            })
            .expect("read links");
        rows
    }

    fn create_calls(client: &WpsClient<MockTransport>) -> Vec<crate::services::wps::MockCall> {
        client
            .transport()
            .calls()
            .into_iter()
            .filter(|call| call.action == "dbsheet.create_records")
            .collect()
    }

    /// 回填桩：命中 `failing_key` 的链接写入返回错误，其余委托真实 store。
    ///
    /// 用「按幂等键失败」而不是「一律失败」，才能在同一个批次里同时验证
    /// 「失败条标 PendingBackfill」与「成功条的回填不受影响」。
    struct FailLinkFor<'a> {
        store: &'a WpsStore<'a>,
        failing_key: &'a str,
    }

    impl LinkWriter for FailLinkFor<'_> {
        fn write_link_row(
            &self,
            sheet_id: i64,
            record_id: &str,
            doc_type: &str,
            idempotency_key: &str,
            fields: &Value,
            remote_json: &str,
        ) -> crate::Result<i64> {
            if idempotency_key == self.failing_key {
                return Err(crate::CoreError::Invalid(
                    "桩注入：链接写入失败（模拟回填异常）".into(),
                ));
            }
            self.store.insert_link(
                sheet_id,
                record_id,
                doc_type,
                idempotency_key,
                fields,
                remote_json,
            )
        }
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

    #[test]
    fn create_doc_records_creates_each_record_and_backfills_links() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let transport = MockTransport::new(
            "s",
            vec![
                Ok(json!({"records": []})),
                Ok(json!({"records": []})),
                Ok(json!({"records": [{"id": "R-1"}]})),
                Ok(json!({"records": [{"id": "R-2"}]})),
            ],
        );
        let client = WpsClient::new(transport, "file-1");
        let flow = WpsCreateFlow::new(&client, &store);
        let run_id = store.start_run("manual", "desktop").expect("run");
        let spec = doc_spec_for_sheet(3).expect("spec 03");

        let outcomes = flow.create_doc_records(
            &spec,
            &[
                WpsCreateInput::new(json!({"应收单号": "AR-1"}), "idem-AR-1"),
                WpsCreateInput::new(json!({"应收单号": "AR-2"}), "idem-AR-2"),
            ],
            run_id,
        );

        assert_eq!(outcomes.len(), 2);
        assert!(outcomes
            .iter()
            .all(|outcome| outcome.status == W2WriteStatus::Succeeded));
        assert_eq!(outcomes[0].remote_record_id.as_deref(), Some("R-1"));
        assert_eq!(outcomes[1].remote_record_id.as_deref(), Some("R-2"));
        assert!(outcomes.iter().all(|outcome| outcome.link_id.is_some()));

        let creates = create_calls(&client);
        assert_eq!(creates.len(), 2, "两条记录必须各发一次逐条创建");
        assert_eq!(creates[0].args["idempotency_key"], "idem-AR-1");
        assert_eq!(creates[1].args["idempotency_key"], "idem-AR-2");

        let mut links = link_rows(&store);
        links.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].0, "R-1");
        assert_eq!(links[1].0, "R-2");
        for link in links {
            assert_eq!(link.1, "synced", "回填状态必须是 synced");
            assert_eq!(link.2, None, "voucher_id 必须留空，等待认领");
            assert_eq!(link.3, "03");
        }
    }

    #[test]
    fn create_doc_records_marks_pending_backfill_when_link_write_fails() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let transport = MockTransport::new(
            "s",
            vec![
                // 两条记录各触发一次守卫刷新（只读），然后各一次逐条创建。
                Ok(json!({"records": []})),
                Ok(json!({"records": []})),
                Ok(json!({"records": [{"id": "R-1"}]})),
                Ok(json!({"records": [{"id": "R-2"}]})),
            ],
        );
        let client = WpsClient::new(transport, "file-1");
        let failing = FailLinkFor {
            store: &store,
            failing_key: "idem-AR-2",
        };
        let flow = WpsCreateFlow::with_link_writer(&client, &store, &failing);
        let run_id = store.start_run("manual", "desktop").expect("run");
        let spec = doc_spec_for_sheet(3).expect("spec 03");

        let outcomes = flow.create_doc_records(
            &spec,
            &[
                WpsCreateInput::new(json!({"应收单号": "AR-1"}), "idem-AR-1"),
                WpsCreateInput::new(json!({"应收单号": "AR-2"}), "idem-AR-2"),
            ],
            run_id,
        );

        // 第一条正常回填；第二条远端成功但回填失败 → PendingBackfill，错误不被吞。
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].status, W2WriteStatus::Succeeded);
        assert!(outcomes[0].link_id.is_some());
        assert_eq!(outcomes[0].remote_record_id.as_deref(), Some("R-1"));

        assert_eq!(outcomes[1].status, W2WriteStatus::PendingBackfill);
        assert_eq!(outcomes[1].remote_record_id.as_deref(), Some("R-2"));
        assert!(outcomes[1].link_id.is_none(), "回填失败不得给出 link_id");
        let error = outcomes[1].error.as_deref().expect("回填失败必须带原因");
        assert!(
            error.starts_with("WPS_E_LINK_BACKFILL:"),
            "错误码必须原样保留：{error}"
        );
        assert!(
            error.contains("桩注入：链接写入失败"),
            "具体原因不得被吞：{error}"
        );

        // 远端只发了各一次逐条创建，失败条不重发。
        let creates = create_calls(&client);
        assert_eq!(creates.len(), 2, "两条记录各一次创建，不得因回填失败重发");
        assert_eq!(creates[0].args["idempotency_key"], "idem-AR-1");
        assert_eq!(creates[1].args["idempotency_key"], "idem-AR-2");

        // 失败只影响自己那条：R-1 的链接行完好，R-2 没有链接行。
        let links = link_rows(&store);
        assert_eq!(links.len(), 1, "回填失败不得写半条链接");
        assert_eq!(links[0].0, "R-1");
        assert_eq!(links[0].1, "synced");
        assert_eq!(links[0].2, None);
    }

    #[test]
    fn create_doc_records_guard_reject_sends_no_create_and_keeps_tables() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        // 远端已有同单号记录：守卫刷新后必须判重，整批不得进入写入段。
        let transport = MockTransport::new(
            "s",
            vec![Ok(
                json!({"records": [{"id": "R-OLD", "fields": {"应收单号": "AR-1"}}]}),
            )],
        );
        let client = WpsClient::new(transport, "file-1");
        let flow = WpsCreateFlow::new(&client, &store);
        let run_id = store.start_run("manual", "desktop").expect("run");
        let before = table_counts(&store);
        let spec = doc_spec_for_sheet(3).expect("spec 03");

        let outcomes = flow.create_doc_records(
            &spec,
            &[WpsCreateInput::new(
                json!({"应收单号": "AR-1"}),
                "idem-AR-1",
            )],
            run_id,
        );

        assert_eq!(outcomes[0].status, W2WriteStatus::Rejected);
        assert!(outcomes[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .starts_with("WPS_E_DOC_DUPLICATE"));
        assert!(create_calls(&client).is_empty(), "守卫拒绝不得发写请求");
        assert_eq!(table_counts(&store), before, "守卫拒绝不得写同步表");
        assert!(link_rows(&store).is_empty());
    }

    #[test]
    fn create_doc_records_empty_business_key_rejects_without_any_request() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
        let flow = WpsCreateFlow::new(&client, &store);
        let run_id = store.start_run("manual", "desktop").expect("run");
        let before = table_counts(&store);
        let spec = doc_spec_for_sheet(3).expect("spec 03");

        let outcomes = flow.create_doc_records(
            &spec,
            &[WpsCreateInput::new(
                json!({"客户": "客户-1"}),
                "idem-empty-key",
            )],
            run_id,
        );

        assert_eq!(outcomes[0].status, W2WriteStatus::Rejected);
        assert!(outcomes[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .starts_with("WPS_E_DOC_KEY"));
        assert!(
            client.transport().calls().is_empty(),
            "空业务键必须本地拒绝，一次请求都不发"
        );
        assert_eq!(table_counts(&store), before);
    }

    #[test]
    fn create_doc_records_keeps_first_success_when_second_fails() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let transport = MockTransport::new(
            "s",
            vec![
                Ok(json!({"records": []})),
                Ok(json!({"records": []})),
                Ok(json!({"records": [{"id": "R-1"}]})),
                Err(WpsError::Timeout),
            ],
        );
        let client = WpsClient::new(transport, "file-1");
        let flow = WpsCreateFlow::new(&client, &store);
        let run_id = store.start_run("manual", "desktop").expect("run");
        let spec = doc_spec_for_sheet(3).expect("spec 03");

        let outcomes = flow.create_doc_records(
            &spec,
            &[
                WpsCreateInput::new(json!({"应收单号": "AR-1"}), "idem-AR-1"),
                WpsCreateInput::new(json!({"应收单号": "AR-2"}), "idem-AR-2"),
            ],
            run_id,
        );

        assert_eq!(outcomes[0].status, W2WriteStatus::Succeeded);
        assert!(outcomes[0].link_id.is_some());
        assert_eq!(outcomes[1].status, W2WriteStatus::Unknown);
        assert!(outcomes[1].link_id.is_none());
        assert!(outcomes[1]
            .error
            .as_deref()
            .unwrap_or_default()
            .starts_with("WPS_E_TIMEOUT"));
        assert_eq!(create_calls(&client).len(), 2);
        let links = link_rows(&store);
        assert_eq!(links.len(), 1, "只回填成功的那一条，已成功的不得回滚");
        assert_eq!(links[0].0, "R-1");
    }

    #[test]
    fn create_doc_records_rejects_dedupe_only_sheets() {
        for sheet_id in SHEET_DEDUPE_ONLY {
            let db = db_with_schema();
            let store = WpsStore::from_db(&db);
            let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
            let flow = WpsCreateFlow::new(&client, &store);
            let run_id = store.start_run("manual", "desktop").expect("run");
            let before = table_counts(&store);
            let spec = doc_spec_for_sheet(sheet_id).expect("spec");

            let outcomes = flow.create_doc_records(
                &spec,
                &[WpsCreateInput::new(
                    json!({"编号": format!("NO-{sheet_id}")}),
                    format!("idem-{sheet_id}"),
                )],
                run_id,
            );

            assert_eq!(outcomes[0].status, W2WriteStatus::Rejected);
            assert!(outcomes[0]
                .error
                .as_deref()
                .unwrap_or_default()
                .starts_with("WPS_E_DEDUPE_ONLY"));
            assert!(client.transport().calls().is_empty());
            assert_eq!(table_counts(&store), before);
        }
    }

    #[test]
    fn create_meter_records_uses_09_guard_and_links() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let transport = MockTransport::new(
            "s",
            vec![
                Ok(json!({"records": []})),
                Ok(json!({"records": [{"id": "R-09"}]})),
            ],
        );
        let client = WpsClient::new(transport, "file-1");
        let flow = WpsCreateFlow::new(&client, &store);
        let run_id = store.start_run("manual", "desktop").expect("run");

        let outcomes = flow.create_meter_records(
            "B1A",
            "2026-11",
            &[WpsCreateInput::new(
                json!({"电表": ["B1A"], "期间": "2026-11"}),
                "idem-09-1",
            )],
            run_id,
        );

        assert_eq!(outcomes[0].status, W2WriteStatus::Succeeded);
        assert_eq!(outcomes[0].remote_record_id.as_deref(), Some("R-09"));
        assert_eq!(create_calls(&client).len(), 1);
        let links = link_rows(&store);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, "R-09");
        assert_eq!(links[0].3, "09");
    }

    /// 静态判据：写入只经 `WpsWriteSession` 与 `WpsStore::insert_link`，且无批量符号。
    ///
    /// 关键字按片段拼接，避免检查代码自身把关键字写进被检查的源文本。
    #[test]
    fn module_source_writes_only_through_session_and_link_backfill() {
        let source = include_str!("wps_create.rs");
        for (label, needle) in [
            ("SQL 插入", ["IN", "SERT INTO"].concat()),
            ("SQL 更新", ["UP", "DATE SET"].concat()),
            ("SQL 删除", ["DE", "LETE FROM"].concat()),
            ("直接构造创建参数", ["build", "_create_args"].concat()),
            ("绕过写会话直调", ["client.create", "_record("].concat()),
            ("批量创建符号", ["batch", "_create"].concat()),
        ] {
            assert!(
                !source.contains(&needle),
                "wps_create.rs 不得包含写入/批量符号（{label}）"
            );
        }
        for required in ["WpsWriteSession::new", "insert_link"] {
            assert!(
                source.contains(required),
                "写入段必须只经 {required}，不得另开写路径"
            );
        }
    }
}
