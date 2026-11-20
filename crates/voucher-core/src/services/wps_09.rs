//! 09 抄表创建前门（W1-B3，只读）。
//!
//! 创建 09 前必须刷新整张 09 表；刷新不完整、坏记录被跳过或已存在同电表同期间
//! 记录时，一律拒绝创建。本模块不发送任何 WPS 写请求。
#![allow(dead_code)] // B3 尚未接生产；W2/W3 接入后移除

use crate::services::wps::{WpsClient, WpsStore, WpsTransport};
use crate::services::wps_pull::pull_sheet;
use std::fmt;

/// 09 创建门的拒绝原因；`code()` 直接对应方案错误码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wps09Error {
    /// 电表记录 id 或期间为空；本地拒绝，0 次请求。
    Key(String),
    /// 09 全表刷新不完整、存在 skipped 或无法查询索引；禁止创建。
    Stale(String),
    /// 已存在同电表同期间记录；禁止重复创建。
    Duplicate(String),
}

impl Wps09Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Key(_) => "WPS_E_09_KEY",
            Self::Stale(_) => "WPS_E_09_STALE",
            Self::Duplicate(_) => "WPS_E_09_DUPLICATE",
        }
    }
}

impl fmt::Display for Wps09Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(detail) | Self::Stale(detail) | Self::Duplicate(detail) => {
                write!(f, "{}: {detail}", self.code())
            }
        }
    }
}

impl std::error::Error for Wps09Error {}

/// 09 创建前门。判定顺序固定：
/// key 校验（0 请求）→ 刷新 09 全表 → complete/skipped 检查 → 查本地索引 → 放行。
pub struct Wps09CreateGuard;

impl Wps09CreateGuard {
    /// 只读检查；通过后调用方才可发起 09 创建。
    ///
    /// `meter_record_id` 是 08 电表的 WPS 记录 id，`period` 是 09 的期间文本
    /// （如 `2026-11`）。两者构成 09 的业务键。
    pub fn ensure_create_allowed<T: WpsTransport>(
        client: &WpsClient<T>,
        store: &WpsStore<'_>,
        meter_record_id: &str,
        period: &str,
    ) -> std::result::Result<(), Wps09Error> {
        let meter = meter_record_id.trim();
        let period = period.trim();
        if meter.is_empty() || period.is_empty() {
            return Err(Wps09Error::Key(
                "电表记录 id 与期间都不能为空；未发起任何 WPS 请求".into(),
            ));
        }

        // 强制刷新整张 09 表，避免拿本地陈旧索引判断重复。
        let report = pull_sheet(client, store, 9, "09", None, 500)
            .map_err(|err| Wps09Error::Stale(format!("09 刷新本地失败：{err}")))?;
        if !report.complete || report.skipped > 0 {
            return Err(Wps09Error::Stale(format!(
                "09 刷新不完整：complete={} skipped={} error={} error_code={}",
                report.complete,
                report.skipped,
                report.error.as_deref().unwrap_or("-"),
                report.error_code.as_deref().unwrap_or("-"),
            )));
        }

        let key = format!("{meter}\u{1f}{period}");
        let rows = store
            .find_index_by_business_key(9, &key)
            .map_err(|err| Wps09Error::Stale(format!("09 索引查询失败：{err}")))?;
        if let Some(row) = rows.first() {
            return Err(Wps09Error::Duplicate(format!(
                "09 已存在同电表同期间记录：{}",
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
    fn missing_key_rejects_without_any_request() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(MockTransport::new("session-1", vec![]), "file-1");
        let err = Wps09CreateGuard::ensure_create_allowed(&client, &store, " ", "2026-11")
            .expect_err("empty meter must fail");
        assert_eq!(err.code(), "WPS_E_09_KEY");
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
        Wps09CreateGuard::ensure_create_allowed(&client, &store, "B1A", "2026-11")
            .expect("empty 09 allows create");
        assert_eq!(client.transport().initialize_calls(), 1);
        assert_eq!(actions(&client), vec!["dbsheet.list_records".to_string()]);
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
        let err = Wps09CreateGuard::ensure_create_allowed(&client, &store, "B1A", "2026-11")
            .expect_err("incomplete refresh must fail");
        assert_eq!(err.code(), "WPS_E_09_STALE");
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
                    {"fields": {"电表": ["B1A"], "期间": "2026-11"}}
                ])))],
            ),
            "file-1",
        );
        let err = Wps09CreateGuard::ensure_create_allowed(&client, &store, "B1A", "2026-11")
            .expect_err("skipped record must fail");
        assert_eq!(err.code(), "WPS_E_09_STALE");
        assert!(err.to_string().contains("skipped=1"), "{err}");
        assert!(actions(&client)
            .iter()
            .all(|action| action == "dbsheet.list_records"));
    }

    #[test]
    fn duplicate_same_meter_period_rejects_duplicate() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(
            MockTransport::new(
                "session-1",
                vec![Ok(page(json!([
                    {"id": "DmC", "fields": {"电表": ["B1A"], "期间": "2026-11"}}
                ])))],
            ),
            "file-1",
        );
        let err = Wps09CreateGuard::ensure_create_allowed(&client, &store, "B1A", "2026-11")
            .expect_err("duplicate must fail");
        assert_eq!(err.code(), "WPS_E_09_DUPLICATE");
        assert!(err.to_string().contains("DmC"), "{err}");
        assert!(actions(&client)
            .iter()
            .all(|action| action == "dbsheet.list_records"));
    }

    #[test]
    fn malformed_payload_is_stale_with_readable_error_code() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(
            MockTransport::new("session-1", vec![Ok(json!({"items": []}))]),
            "file-1",
        );
        let err = Wps09CreateGuard::ensure_create_allowed(&client, &store, "B1A", "2026-11")
            .expect_err("malformed 09 page must fail");
        assert_eq!(err.code(), "WPS_E_09_STALE");
        let text = err.to_string();
        assert!(text.contains("WPS_E_PULL_MALFORMED"), "{text}");
    }

    #[test]
    fn different_period_is_not_duplicate() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(
            MockTransport::new(
                "session-1",
                vec![Ok(page(json!([
                    {"id": "DmA", "fields": {"电表": ["B1A"], "期间": "2026-10"}}
                ])))],
            ),
            "file-1",
        );
        Wps09CreateGuard::ensure_create_allowed(&client, &store, "B1A", "2026-11")
            .expect("different period is allowed");
    }
}
