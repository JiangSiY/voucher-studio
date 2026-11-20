//! WPS 分页全表拉取（W1-B2，只读）。
//!
//! 只负责把 WPS `list_records` 的原始 JSON 解析成索引行，并维护
//! `wps_scans` 生命周期；不含 09 创建门、不含写入、不含真实 HTTP。
//!
use chrono::{SecondsFormat, Utc};
use serde_json::Value;

use crate::services::wps::{WpsClient, WpsError, WpsStore, WpsTransport};
use crate::services::wps_index::{business_key_for, WpsIndexRow};
use crate::Result;

/// 单页上限与 W0 实测一致；调用方再大也不超过该值。
pub const MAX_PAGE_SIZE: usize = 500;
/// 防止服务端 offset 异常造成无限分页。
pub const MAX_PULL_PAGES: usize = 1_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullPage {
    pub rows: Vec<WpsIndexRow>,
    pub raw_records: usize,
    pub skipped: usize,
    pub has_more: Option<bool>,
    pub total: Option<usize>,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullReport {
    pub sheet_id: i64,
    pub record_type: String,
    pub view_id: Option<String>,
    pub pages: usize,
    pub raw_records: usize,
    pub indexed: usize,
    pub skipped: usize,
    pub complete: bool,
    pub error: Option<String>,
    pub error_code: Option<String>,
}

/// 对外使用的报告别名；B3 09 门按 `complete` / `skipped` / `error_code` 判定。
pub type WpsPullReport = PullReport;

/// 解析一页 `list_records` 响应。
///
/// 容错规则：缺少 `id`、`fields` 非对象、记录非对象的条目计入 `skipped`，
/// 不阻断整页；响应缺少 records 数组才返回 `WPS_E_PULL_MALFORMED`。
pub fn parse_list_page(
    sheet_id: i64,
    record_type: &str,
    payload: &Value,
) -> std::result::Result<PullPage, WpsError> {
    let records = records_array(payload).ok_or_else(|| {
        WpsError::PullMalformed(
            "响应缺少 records 数组（支持 records / data.records / data 数组）".into(),
        )
    })?;
    let mut page = PullPage {
        raw_records: records.len(),
        has_more: lookup_bool(payload, &["has_more", "hasMore"]),
        total: lookup_usize(payload, &["total", "total_count"]),
        next_offset: lookup_usize(payload, &["next_offset", "nextOffset"]),
        ..PullPage::default()
    };
    let pulled_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

    for record in records {
        let Some(object) = record.as_object() else {
            page.skipped += 1;
            continue;
        };
        let Some(record_id) = object
            .get("id")
            .or_else(|| object.get("recordId"))
            .and_then(scalar_text)
        else {
            page.skipped += 1;
            continue;
        };
        let Some(fields) = object.get("fields").and_then(Value::as_object) else {
            page.skipped += 1;
            continue;
        };
        let fields = Value::Object(fields.clone());
        let (business_key, business_key_kind) = business_key_for(sheet_id, &fields);
        let remote_modified_at = object
            .get("lastModifiedTime")
            .or_else(|| object.get("last_modified_at"))
            .or_else(|| object.get("modifiedTime"))
            .and_then(scalar_text);
        page.rows.push(WpsIndexRow::new(
            sheet_id,
            record_id,
            record_type,
            business_key,
            business_key_kind,
            remote_modified_at,
            fields.to_string(),
            pulled_at.clone(),
        ));
    }
    Ok(page)
}

/// 分页拉取整张表。远端/解析错误写入 `wps_scans.error` 后返回报告，
/// 不抛出；只有本地数据库错误才返回 `Err`。
pub fn pull_sheet<T: WpsTransport>(
    client: &WpsClient<T>,
    store: &WpsStore<'_>,
    sheet_id: i64,
    record_type: &str,
    view_id: Option<&str>,
    page_size: usize,
) -> Result<PullReport> {
    pull_sheet_with_limit(
        client,
        store,
        sheet_id,
        record_type,
        view_id,
        page_size,
        MAX_PULL_PAGES,
    )
}

/// 与 `pull_sheet` 相同，但显式给出页上限；测试用，生产固定 `MAX_PULL_PAGES`。
fn pull_sheet_with_limit<T: WpsTransport>(
    client: &WpsClient<T>,
    store: &WpsStore<'_>,
    sheet_id: i64,
    record_type: &str,
    view_id: Option<&str>,
    page_size: usize,
    max_pages: usize,
) -> Result<PullReport> {
    let page_size = page_size.clamp(1, MAX_PAGE_SIZE);
    let max_pages = max_pages.max(1);
    let scan_id = store.begin_scan(sheet_id, view_id)?;
    let mut report = PullReport {
        sheet_id,
        record_type: record_type.to_string(),
        view_id: view_id.map(str::to_string),
        pages: 0,
        raw_records: 0,
        indexed: 0,
        skipped: 0,
        complete: false,
        error: None,
        error_code: None,
    };

    if client.session_id().is_none() {
        if let Err(err) = client.initialize() {
            report.error = Some(format!("{}: {err}", err.code()));
            report.error_code = Some(err.code().to_string());
            store.finish_scan(scan_id, 0, false, report.error.as_deref())?;
            return Ok(report);
        }
    }

    let mut offset = 0usize;
    loop {
        if report.pages >= max_pages {
            report.error = Some(format!("WPS_E_PULL_MAX_PAGES: 超过 {max_pages} 页仍未结束"));
            report.error_code = Some("WPS_E_PULL_MAX_PAGES".to_string());
            store.finish_scan(scan_id, report.pages, false, report.error.as_deref())?;
            return Ok(report);
        }

        let payload = match client.list_records(sheet_id, view_id, offset, page_size) {
            Ok(payload) => payload,
            Err(err) => {
                report.error = Some(format!("{}: {err}", err.code()));
                report.error_code = Some(err.code().to_string());
                store.finish_scan(scan_id, report.pages, false, report.error.as_deref())?;
                return Ok(report);
            }
        };
        let page = match parse_list_page(sheet_id, record_type, &payload) {
            Ok(page) => page,
            Err(err) => {
                report.error = Some(format!("{}: {err}", err.code()));
                report.error_code = Some(err.code().to_string());
                store.finish_scan(scan_id, report.pages, false, report.error.as_deref())?;
                return Ok(report);
            }
        };

        if let Err(err) = store.upsert_index_rows(&page.rows) {
            let message = err.to_string();
            store.finish_scan(scan_id, report.pages, false, Some(&message))?;
            return Err(err);
        }
        report.pages += 1;
        report.raw_records += page.raw_records;
        report.indexed += page.rows.len();
        report.skipped += page.skipped;

        if page.raw_records == 0 {
            break;
        }
        let previous = offset;
        offset = page
            .next_offset
            .unwrap_or_else(|| previous + page.raw_records);
        if offset <= previous {
            report.error = Some(format!(
                "WPS_E_PULL_OFFSET: offset 未前进（{previous} → {offset}）"
            ));
            report.error_code = Some("WPS_E_PULL_OFFSET".to_string());
            store.finish_scan(scan_id, report.pages, false, report.error.as_deref())?;
            return Ok(report);
        }
        if page.has_more == Some(false) {
            break;
        }
        if let Some(total) = page.total {
            if offset >= total {
                break;
            }
        }
        if page.has_more != Some(true) && page.raw_records < page_size {
            break;
        }
    }

    store.finish_scan(scan_id, report.pages, true, None)?;
    report.complete = true;
    Ok(report)
}

fn records_array(payload: &Value) -> Option<&Vec<Value>> {
    if let Some(array) = payload.as_array() {
        return Some(array);
    }
    payload
        .get("records")
        .and_then(Value::as_array)
        .or_else(|| payload.pointer("/data/records").and_then(Value::as_array))
        .or_else(|| payload.get("data").and_then(Value::as_array))
}

fn lookup_bool(payload: &Value, keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Some(value) = payload.get(*key).and_then(Value::as_bool) {
            return Some(value);
        }
        if let Some(value) = payload
            .pointer(&format!("/data/{key}"))
            .and_then(Value::as_bool)
        {
            return Some(value);
        }
    }
    None
}

fn lookup_usize(payload: &Value, keys: &[&str]) -> Option<usize> {
    for key in keys {
        for value in [payload.get(*key), payload.pointer(&format!("/data/{key}"))]
            .into_iter()
            .flatten()
        {
            if let Some(number) = value.as_u64() {
                return usize::try_from(number).ok();
            }
            if let Some(text) = value.as_str() {
                if let Ok(number) = text.parse::<usize>() {
                    return Some(number);
                }
            }
        }
    }
    None
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{schema, Database};
    use crate::services::wps::{MockTransport, WpsClient, WpsError};
    use rusqlite::{params, Connection};
    use serde_json::json;

    fn db_with_schema() -> Database {
        let conn = Connection::open_in_memory().expect("open memory db");
        schema::init(&conn).expect("init schema");
        Database::new(conn)
    }

    fn scan_row(db: &Database, scan_id: i64) -> (i64, Option<String>, Option<String>) {
        db.with_conn(|conn| {
            conn.query_row(
                "SELECT complete, error, finished_at FROM wps_scans WHERE id = ?1",
                params![scan_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(Into::into)
        })
        .expect("query scan")
    }

    #[test]
    fn parse_page_extracts_rows_and_page_metadata() {
        let payload = json!({
            "records": [
                {"id": "r1", "lastModifiedTime": "2026/11/19 10:00:00", "fields": {"应收单号": "AR-1", "金额": 100}},
                {"id": "r2", "fields": {"应收单号": "AR-2"}}
            ],
            "total": 2,
            "has_more": false
        });
        let page = parse_list_page(3, "03", &payload).expect("parse");
        assert_eq!(page.raw_records, 2);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.skipped, 0);
        assert_eq!(page.total, Some(2));
        assert_eq!(page.has_more, Some(false));
        assert_eq!(page.rows[0].business_key, "AR-1");
        assert_eq!(page.rows[0].business_key_kind, "doc_no");
        assert_eq!(
            page.rows[0].remote_modified_at.as_deref(),
            Some("2026/11/19 10:00:00")
        );
        assert_eq!(page.rows[1].business_key, "AR-2");
    }

    #[test]
    fn parse_page_skips_bad_records_without_failing() {
        let payload = json!({
            "records": [
                {"fields": {"应收单号": "AR-1"}},
                {"id": "r2", "fields": "not-an-object"},
                null,
                {"id": "r4", "fields": {}}
            ]
        });
        let page = parse_list_page(3, "03", &payload).expect("parse");
        assert_eq!(page.raw_records, 4);
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.skipped, 3);
        assert_eq!(page.rows[0].business_key, "");
        assert_eq!(page.rows[0].business_key_kind, "none");
    }

    #[test]
    fn parse_page_accepts_data_array_and_camel_case_has_more() {
        let payload = json!({
            "data": [
                {"id": "r1", "fields": {"应收单号": "AR-1"}}
            ],
            "hasMore": true,
            "nextOffset": 1
        });
        let page = parse_list_page(3, "03", &payload).expect("parse");
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.has_more, Some(true));
        assert_eq!(page.next_offset, Some(1));
    }

    #[test]
    fn parse_page_rejects_payload_without_records() {
        let err = parse_list_page(3, "03", &json!({"items": []})).expect_err("must fail");
        assert_eq!(err.code(), "WPS_E_PULL_MALFORMED");
        assert!(err.to_string().contains("WPS_E_PULL_MALFORMED"));
    }

    #[test]
    fn pull_sheet_pages_until_short_page_and_marks_scan_complete() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let transport = MockTransport::new(
            "session-1",
            vec![
                Ok(json!({
                    "records": [
                        {"id": "r1", "fields": {"应收单号": "AR-1"}},
                        {"id": "r2", "fields": {"应收单号": "AR-2"}}
                    ]
                })),
                Ok(json!({
                    "records": [
                        {"id": "r3", "fields": {"应收单号": "AR-3"}}
                    ]
                })),
            ],
        );
        let client = WpsClient::new(transport, "file-1");
        let report = pull_sheet(&client, &store, 3, "03", Some("view-1"), 2).expect("pull");
        assert!(report.complete);
        assert_eq!(report.pages, 2);
        assert_eq!(report.raw_records, 3);
        assert_eq!(report.indexed, 3);
        assert!(report.error.is_none());
        assert_eq!(store.count_index(3).expect("count"), 3);

        let (complete, error, finished): (i64, Option<String>, Option<String>) = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT complete, error, finished_at FROM wps_scans WHERE sheet_id = 3",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(Into::into)
            })
            .expect("query scan");
        assert_eq!(complete, 1);
        assert!(error.is_none());
        assert!(finished.is_some());
    }

    #[test]
    fn pull_sheet_marks_complete_false_when_max_pages_reached() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let page = || {
            Ok(json!({
                "records": [
                    {"id": "r1", "fields": {"应收单号": "AR-1"}},
                    {"id": "r2", "fields": {"应收单号": "AR-2"}}
                ],
                "has_more": true
            }))
        };
        let transport = MockTransport::new("session-1", vec![page(), page(), page()]);
        let client = WpsClient::new(transport, "file-1");
        let report = pull_sheet_with_limit(&client, &store, 3, "03", None, 2, 2).expect("pull");
        assert!(!report.complete);
        assert_eq!(report.pages, 2);
        assert_eq!(report.error_code.as_deref(), Some("WPS_E_PULL_MAX_PAGES"));
        assert!(report
            .error
            .as_deref()
            .is_some_and(|error| error.contains("WPS_E_PULL_MAX_PAGES")));

        let scan_id: i64 = db
            .with_conn(|conn| {
                conn.query_row("SELECT MAX(id) FROM wps_scans", [], |row| row.get(0))
                    .map_err(Into::into)
            })
            .expect("scan id");
        let (complete, error, _) = scan_row(&db, scan_id);
        assert_eq!(complete, 0);
        assert!(error
            .as_deref()
            .is_some_and(|error| error.contains("WPS_E_PULL_MAX_PAGES")));
    }

    #[test]
    fn pull_sheet_marks_scan_incomplete_on_offset_regression() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let transport = MockTransport::new(
            "session-1",
            vec![Ok(json!({
                "records": [
                    {"id": "r1", "fields": {"应收单号": "AR-1"}},
                    {"id": "r2", "fields": {"应收单号": "AR-2"}}
                ],
                "next_offset": 0
            }))],
        );
        let client = WpsClient::new(transport, "file-1");
        let report = pull_sheet(&client, &store, 3, "03", None, 2).expect("pull");
        assert!(!report.complete);
        assert_eq!(report.pages, 1);
        assert!(report
            .error
            .as_deref()
            .is_some_and(|error| error.contains("WPS_E_PULL_OFFSET")));

        let scan_id: i64 = db
            .with_conn(|conn| {
                conn.query_row("SELECT MAX(id) FROM wps_scans", [], |row| row.get(0))
                    .map_err(Into::into)
            })
            .expect("scan id");
        let (complete, error, _) = scan_row(&db, scan_id);
        assert_eq!(complete, 0);
        assert!(error
            .as_deref()
            .is_some_and(|error| error.contains("WPS_E_PULL_OFFSET")));
    }

    #[test]
    fn pull_sheet_marks_scan_incomplete_on_remote_error() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let transport = MockTransport::new(
            "session-1",
            vec![
                Err(WpsError::RateLimited),
                Err(WpsError::RateLimited),
                Err(WpsError::RateLimited),
            ],
        );
        let client = WpsClient::new(transport, "file-1");
        let report = pull_sheet(&client, &store, 3, "03", None, 500).expect("pull");
        assert!(!report.complete);
        assert_eq!(report.pages, 0);
        assert_eq!(report.error_code.as_deref(), Some("WPS_E_RATE"));
        assert!(report
            .error
            .as_deref()
            .is_some_and(|error| error.contains("WPS_E_RATE")));

        let scan_id: i64 = db
            .with_conn(|conn| {
                conn.query_row("SELECT MAX(id) FROM wps_scans", [], |row| row.get(0))
                    .map_err(Into::into)
            })
            .expect("scan id");
        let (complete, error, _) = scan_row(&db, scan_id);
        assert_eq!(complete, 0);
        assert!(error
            .as_deref()
            .is_some_and(|error| error.contains("WPS_E_RATE")));
    }

    #[test]
    fn pull_sheet_marks_scan_incomplete_on_payload_error() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let transport = MockTransport::new("session-1", vec![Ok(json!({"items": []}))]);
        let client = WpsClient::new(transport, "file-1");
        let report = pull_sheet(&client, &store, 3, "03", None, 500).expect("pull");
        assert!(!report.complete);
        assert_eq!(report.error_code.as_deref(), Some("WPS_E_PULL_MALFORMED"));
        assert!(report
            .error
            .as_deref()
            .is_some_and(|error| error.contains("WPS_E_PULL_MALFORMED")));
        let scan_id: i64 = db
            .with_conn(|conn| {
                conn.query_row("SELECT MAX(id) FROM wps_scans", [], |row| row.get(0))
                    .map_err(Into::into)
            })
            .expect("scan id");
        let (complete, error, _) = scan_row(&db, scan_id);
        assert_eq!(complete, 0);
        assert!(error
            .as_deref()
            .is_some_and(|error| error.contains("WPS_E_PULL_MALFORMED")));
    }
}
