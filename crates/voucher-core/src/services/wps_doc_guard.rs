//! 单号表（02/03/04/06/05/07/08/10/11）创建前门（A 主线，只读）。
//!
//! 创建带单号的记录前，必须整表刷新对应 sheet 并检查本地索引：
//! 刷新不完整、坏记录被跳过或已存在同单号记录时一律拒绝创建。
//! 本模块不发送任何 WPS 写请求；不改 `wps_09.rs`、不改表结构、
//! 不改写入路径、不接真实 HTTP。
//!
//! # 边界声明（2026-11-20 评审修正）
//!
//! - 本模块**仅检查单号重复**，**不检查主数据引用是否存在**（如 02 的客户
//!   Link、03/04 的项目 Link）。主数据路径由审批问题 5 的修订单独处理。
//! - `sheet_id` 来源：`WpsDocNoSpec` 常量（本文件）+ `wps_pull`/`wps_index`
//!   参数贯穿；**不在此处新增硬编码表号以外的常量**（09 门与 `wps_index`
//!   已有的 `9` 属既有约定，本模块不重复定义 9）。
//! - `view_id` 一律传 `None`：整表刷新，不做视图缩窄（减少为未验证优化项）。

use crate::services::wps::{WpsClient, WpsStore, WpsTransport};
use crate::services::wps_pull::pull_sheet;
use std::fmt;

/// 单号表的 sheet 描述：`sheet_id` + `record_type`（与 `wps_index.business_key_for` 表号一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WpsDocNoSpec {
    pub sheet_id: i64,
    pub record_type: &'static str,
}

/// 单号表预设常量；字段名依据 `财务工作流/设计/项目台账_v2_设计.md`
/// （02 项目编号 L92 / 03 应收单号 L119 / 04 应付单号 L137 / 06 流水号 L174 /
/// 05 调整编号 L159 / 07 发票号 L192 / 08 表号 L197 / 10 SKU L205 / 11 单号 L210）。
pub const DOC_SPEC_02: WpsDocNoSpec = WpsDocNoSpec {
    sheet_id: 2,
    record_type: "02",
};
pub const DOC_SPEC_03: WpsDocNoSpec = WpsDocNoSpec {
    sheet_id: 3,
    record_type: "03",
};
pub const DOC_SPEC_04: WpsDocNoSpec = WpsDocNoSpec {
    sheet_id: 4,
    record_type: "04",
};
pub const DOC_SPEC_05: WpsDocNoSpec = WpsDocNoSpec {
    sheet_id: 5,
    record_type: "05",
};
pub const DOC_SPEC_06: WpsDocNoSpec = WpsDocNoSpec {
    sheet_id: 6,
    record_type: "06",
};
pub const DOC_SPEC_07: WpsDocNoSpec = WpsDocNoSpec {
    sheet_id: 7,
    record_type: "07",
};
pub const DOC_SPEC_08: WpsDocNoSpec = WpsDocNoSpec {
    sheet_id: 8,
    record_type: "08",
};
pub const DOC_SPEC_10: WpsDocNoSpec = WpsDocNoSpec {
    sheet_id: 10,
    record_type: "10",
};
pub const DOC_SPEC_11: WpsDocNoSpec = WpsDocNoSpec {
    sheet_id: 11,
    record_type: "11",
};

/// 按 sheet_id 取单号表规格；仅供 W2 预检等已刷新索引的调用方复用判定。
pub fn doc_spec_for_sheet(sheet_id: i64) -> Option<WpsDocNoSpec> {
    match sheet_id {
        2 => Some(DOC_SPEC_02),
        3 => Some(DOC_SPEC_03),
        4 => Some(DOC_SPEC_04),
        5 => Some(DOC_SPEC_05),
        6 => Some(DOC_SPEC_06),
        7 => Some(DOC_SPEC_07),
        8 => Some(DOC_SPEC_08),
        10 => Some(DOC_SPEC_10),
        11 => Some(DOC_SPEC_11),
        _ => None,
    }
}

/// 单号创建门的拒绝原因；`code()` 直接对应方案错误码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WpsDocNoError {
    /// 单号为空；本地拒绝，0 次请求。
    Key(String),
    /// 整表刷新不完整、存在 skipped 或无法查询索引；禁止创建。
    Stale(String),
    /// 已存在同单号记录；禁止重复创建。
    Duplicate(String),
}

impl WpsDocNoError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Key(_) => "WPS_E_DOC_KEY",
            Self::Stale(_) => "WPS_E_DOC_STALE",
            Self::Duplicate(_) => "WPS_E_DOC_DUPLICATE",
        }
    }
}

impl fmt::Display for WpsDocNoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(detail) | Self::Stale(detail) | Self::Duplicate(detail) => {
                write!(f, "{}: {detail}", self.code())
            }
        }
    }
}

impl std::error::Error for WpsDocNoError {}

/// 单号创建前门。判定顺序固定（与 09 门对称）：
/// key 校验（0 请求）→ 整表刷新 → complete/skipped → 查本地索引 → 放行。
pub struct WpsDocNoCreateGuard;

impl WpsDocNoCreateGuard {
    /// 索引已由同一轮全历史刷新发布时，只做单号重复判定（不再发请求）。
    ///
    /// W2 预检使用本方法复用单号门的判定，不重复刷新整表。
    pub fn ensure_index_unique(
        store: &WpsStore<'_>,
        spec: &WpsDocNoSpec,
        business_key: &str,
    ) -> std::result::Result<(), WpsDocNoError> {
        let key = business_key.trim();
        if key.is_empty() {
            return Err(WpsDocNoError::Key("单号为空".into()));
        }
        let found = store
            .find_index_by_business_key(spec.sheet_id, key)
            .map_err(|err| WpsDocNoError::Stale(format!("读取索引失败: {err}")))?;
        if !found.is_empty() {
            return Err(WpsDocNoError::Duplicate(format!(
                "表 {} 已有单号 {key}",
                spec.sheet_id
            )));
        }
        Ok(())
    }

    /// 只读检查；通过后调用方才可发起单号表创建。
    ///
    /// `spec` 指定目标表（sheet_id + record_type），`doc_no` 为待创建的单号文本。
    pub fn ensure_create_allowed<T: WpsTransport>(
        client: &WpsClient<T>,
        store: &WpsStore<'_>,
        spec: &WpsDocNoSpec,
        doc_no: &str,
    ) -> std::result::Result<(), WpsDocNoError> {
        let doc_no = doc_no.trim();
        if doc_no.is_empty() {
            return Err(WpsDocNoError::Key(
                "单号不能为空；未发起任何 WPS 请求".into(),
            ));
        }

        // 强制整表刷新，避免拿本地陈旧索引判断重复；view_id 恒为 None。
        let report = pull_sheet(client, store, spec.sheet_id, spec.record_type, None, 500)
            .map_err(|err| WpsDocNoError::Stale(format!("刷新本地失败：{err}")))?;
        if !report.complete || report.skipped > 0 {
            return Err(WpsDocNoError::Stale(format!(
                "刷新不完整：complete={} skipped={} error={} error_code={}",
                report.complete,
                report.skipped,
                report.error.as_deref().unwrap_or("-"),
                report.error_code.as_deref().unwrap_or("-"),
            )));
        }

        let rows = store
            .find_index_by_business_key(spec.sheet_id, doc_no)
            .map_err(|err| WpsDocNoError::Stale(format!("索引查询失败：{err}")))?;
        if let Some(row) = rows.first() {
            return Err(WpsDocNoError::Duplicate(format!(
                "已存在同单号记录：{}",
                row.record_id
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{schema, Database};
    use crate::services::wps::{MockTransport, WpsClient, WpsError};
    use rusqlite::Connection;
    use serde_json::json;

    fn db_with_schema() -> Database {
        let conn = Connection::open_in_memory().expect("open memory db");
        schema::init(&conn).expect("init schema");
        Database::new(conn)
    }

    fn empty_page() -> serde_json::Value {
        json!({"records": []})
    }

    fn page(records: serde_json::Value) -> serde_json::Value {
        json!({"records": records, "has_more": false})
    }

    fn actions(client: &WpsClient<MockTransport>) -> Vec<String> {
        client
            .transport()
            .calls()
            .into_iter()
            .map(|call| call.action)
            .collect()
    }

    #[test]
    fn empty_doc_no_rejects_without_any_request() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(MockTransport::new("session-1", vec![]), "file-1");
        let err = WpsDocNoCreateGuard::ensure_create_allowed(&client, &store, &DOC_SPEC_03, "  ")
            .expect_err("empty doc no must fail");
        assert_eq!(err.code(), "WPS_E_DOC_KEY");
        assert_eq!(client.transport().initialize_calls(), 0);
        assert!(client.transport().calls().is_empty());
    }

    #[test]
    fn complete_empty_pull_allows_create_and_never_writes() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(
            MockTransport::new("session-1", vec![Ok(empty_page())]),
            "file-1",
        );
        WpsDocNoCreateGuard::ensure_create_allowed(&client, &store, &DOC_SPEC_03, "AR-1")
            .expect("empty table allows create");
        assert_eq!(client.transport().initialize_calls(), 1);
        assert_eq!(actions(&client), vec!["dbsheet.list_records".to_string()]);
        // 断言发给 mock 的 sheet_id/record_type 透传正确
        let calls = client.transport().calls();
        assert_eq!(calls[0].args["sheet_id"], json!(3));
        assert!(calls[0].args.get("view_id").is_none());
    }

    #[test]
    fn incomplete_refresh_rejects_stale_with_readable_error() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(
            MockTransport::new(
                "session-1",
                vec![
                    Err(WpsError::RateLimited),
                    Err(WpsError::RateLimited),
                    Err(WpsError::RateLimited),
                ],
            ),
            "file-1",
        );
        let err = WpsDocNoCreateGuard::ensure_create_allowed(&client, &store, &DOC_SPEC_03, "AR-1")
            .expect_err("incomplete refresh must fail");
        assert_eq!(err.code(), "WPS_E_DOC_STALE");
        let text = err.to_string();
        assert!(text.contains("WPS_E_RATE"), "必须暴露 error_code：{text}");
        assert!(text.contains("error_code=WPS_E_RATE"), "{text}");
        assert!(actions(&client)
            .iter()
            .all(|action| action == "dbsheet.list_records"));
    }

    #[test]
    fn skipped_record_in_complete_pull_rejects_stale() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(
            MockTransport::new(
                "session-1",
                vec![Ok(page(json!([
                    {"fields": {"应收单号": "AR-1"}}
                ])))],
            ),
            "file-1",
        );
        let err = WpsDocNoCreateGuard::ensure_create_allowed(&client, &store, &DOC_SPEC_03, "AR-1")
            .expect_err("skipped record must fail");
        assert_eq!(err.code(), "WPS_E_DOC_STALE");
        assert!(err.to_string().contains("skipped=1"), "{err}");
        assert!(actions(&client)
            .iter()
            .all(|action| action == "dbsheet.list_records"));
    }

    #[test]
    fn duplicate_same_doc_no_rejects_duplicate() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(
            MockTransport::new(
                "session-1",
                vec![Ok(page(json!([
                    {"id": "Dl9", "fields": {"应收单号": "AR-1"}}
                ])))],
            ),
            "file-1",
        );
        let err = WpsDocNoCreateGuard::ensure_create_allowed(&client, &store, &DOC_SPEC_03, "AR-1")
            .expect_err("duplicate must fail");
        assert_eq!(err.code(), "WPS_E_DOC_DUPLICATE");
        assert!(err.to_string().contains("Dl9"), "{err}");
        assert!(actions(&client)
            .iter()
            .all(|action| action == "dbsheet.list_records"));
    }

    #[test]
    fn different_doc_no_is_not_duplicate() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(
            MockTransport::new(
                "session-1",
                vec![Ok(page(json!([
                    {"id": "Dl9", "fields": {"应收单号": "AR-1"}}
                ])))],
            ),
            "file-1",
        );
        WpsDocNoCreateGuard::ensure_create_allowed(&client, &store, &DOC_SPEC_03, "AR-2")
            .expect("different doc no is allowed");
    }

    #[test]
    fn guard_never_writes() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(
            MockTransport::new(
                "session-1",
                vec![Ok(page(json!([
                    {"id": "Dl9", "fields": {"应收单号": "AR-1"}}
                ])))],
            ),
            "file-1",
        );
        let err = WpsDocNoCreateGuard::ensure_create_allowed(&client, &store, &DOC_SPEC_03, "AR-1")
            .expect_err("duplicate must fail");
        assert_eq!(err.code(), "WPS_E_DOC_DUPLICATE");
        // 无写请求断言：全程只允许 list_records，不允许 create_records / update_records。
        let calls = actions(&client);
        assert!(
            !calls
                .iter()
                .any(|a| a == "dbsheet.create_records" || a == "dbsheet.update_records"),
            "doc guard 不得发起写请求，实际调用：{calls:?}"
        );
        assert!(
            calls.iter().all(|a| a == "dbsheet.list_records"),
            "{calls:?}"
        );
    }
}
