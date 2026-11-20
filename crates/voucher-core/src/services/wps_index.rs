//! WPS 本地记录索引（W1-B1，只读）。
//!
//! 只负责把 WPS 原始记录落到 `wps_record_index`，供预检与 09 刷新型查询使用；
//! 不解析业务字段、不写 `wps_links`、不进入写入路径。
//!
use rusqlite::{params, OptionalExtension, Row};
use serde_json::Value;

use crate::services::wps::WpsStore;
use crate::{CoreError, Result};

pub const KIND_DOC_NO: &str = "doc_no";
pub const KIND_METER_PERIOD: &str = "meter_period";
pub const KIND_NONE: &str = "none";

/// `wps_record_index` 的一行；`fields_json` 保留 WPS 原始字段对象。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WpsIndexRow {
    pub sheet_id: i64,
    pub record_id: String,
    pub record_type: String,
    pub business_key: String,
    pub business_key_kind: String,
    pub remote_modified_at: Option<String>,
    pub fields_json: String,
    pub pulled_at: String,
}

impl WpsIndexRow {
    /// 便捷构造；`fields_json` 由调用方保证是合法 JSON 文本。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sheet_id: i64,
        record_id: impl Into<String>,
        record_type: impl Into<String>,
        business_key: impl Into<String>,
        business_key_kind: impl Into<String>,
        remote_modified_at: Option<String>,
        fields_json: impl Into<String>,
        pulled_at: impl Into<String>,
    ) -> Self {
        Self {
            sheet_id,
            record_id: record_id.into(),
            record_type: record_type.into(),
            business_key: business_key.into(),
            business_key_kind: business_key_kind.into(),
            remote_modified_at,
            fields_json: fields_json.into(),
            pulled_at: pulled_at.into(),
        }
    }
}

/// 扫描生命周期：写入 `wps_scans`，返回 scan_id。
impl<'a> WpsStore<'a> {
    pub(crate) fn begin_scan(&self, sheet_id: i64, view_id: Option<&str>) -> Result<i64> {
        self.database().with_conn(|conn| {
            conn.execute(
                "INSERT INTO wps_scans(sheet_id, view_id, started_at)
                 VALUES(?1, ?2, datetime('now'))",
                params![sheet_id, view_id],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// 结束扫描。只有自然结束才允许 `complete=true`；失败时保留 `error` 文本。
    pub(crate) fn finish_scan(
        &self,
        scan_id: i64,
        pages: usize,
        complete: bool,
        error: Option<&str>,
    ) -> Result<()> {
        if complete && error.is_some() {
            return Err(CoreError::Invalid(
                "WPS_E_SCAN_INCONSISTENT: complete=true 时 error 必须为空".to_string(),
            ));
        }
        self.database().with_conn(|conn| {
            conn.execute(
                "UPDATE wps_scans
                    SET finished_at = datetime('now'),
                        pages = ?2,
                        complete = ?3,
                        error = ?4
                  WHERE id = ?1",
                params![scan_id, pages as i64, i64::from(complete), error],
            )?;
            Ok(())
        })
    }

    /// 页面级 UPSERT；调用方负责只传已完整解析的页。
    pub(crate) fn upsert_index_rows(&self, rows: &[WpsIndexRow]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        self.database().with_tx(|tx| {
            let mut stmt = tx.prepare(
                "INSERT INTO wps_record_index
                   (sheet_id, record_id, record_type, business_key,
                    business_key_kind, remote_modified_at, fields_json, pulled_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(sheet_id, record_id) DO UPDATE SET
                   record_type = excluded.record_type,
                   business_key = excluded.business_key,
                   business_key_kind = excluded.business_key_kind,
                   remote_modified_at = excluded.remote_modified_at,
                   fields_json = excluded.fields_json,
                   pulled_at = excluded.pulled_at",
            )?;
            for row in rows {
                stmt.execute(params![
                    row.sheet_id,
                    row.record_id,
                    row.record_type,
                    row.business_key,
                    row.business_key_kind,
                    row.remote_modified_at,
                    row.fields_json,
                    row.pulled_at,
                ])?;
            }
            Ok(())
        })
    }

    /// 同一业务键允许重复（WPS 允许重复单号/同表同期），返回全部命中。
    #[allow(dead_code)] // W2 预检接入；B2 仅写索引
    pub(crate) fn find_index_by_business_key(
        &self,
        sheet_id: i64,
        key: &str,
    ) -> Result<Vec<WpsIndexRow>> {
        self.database().with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT sheet_id, record_id, record_type, business_key,
                        business_key_kind, remote_modified_at, fields_json, pulled_at
                   FROM wps_record_index
                  WHERE sheet_id = ?1 AND business_key = ?2
                  ORDER BY id",
            )?;
            let rows = stmt
                .query_map(params![sheet_id, key], map_index_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    #[allow(dead_code)] // W2 预检接入；B2 仅写索引
    pub(crate) fn find_index_by_record_id(
        &self,
        sheet_id: i64,
        record_id: &str,
    ) -> Result<Option<WpsIndexRow>> {
        self.database().with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT sheet_id, record_id, record_type, business_key,
                            business_key_kind, remote_modified_at, fields_json, pulled_at
                       FROM wps_record_index
                      WHERE sheet_id = ?1 AND record_id = ?2",
                    params![sheet_id, record_id],
                    map_index_row,
                )
                .optional()?;
            Ok(row)
        })
    }

    #[allow(dead_code)] // W2 预检接入；B2 仅写索引
    pub(crate) fn count_index(&self, sheet_id: i64) -> Result<usize> {
        self.database().with_conn(|conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM wps_record_index WHERE sheet_id = ?1",
                params![sheet_id],
                |row| row.get(0),
            )?;
            Ok(n as usize)
        })
    }
}

fn map_index_row(row: &Row<'_>) -> rusqlite::Result<WpsIndexRow> {
    Ok(WpsIndexRow {
        sheet_id: row.get(0)?,
        record_id: row.get(1)?,
        record_type: row.get(2)?,
        business_key: row.get(3)?,
        business_key_kind: row.get(4)?,
        remote_modified_at: row.get(5)?,
        fields_json: row.get(6)?,
        pulled_at: row.get(7)?,
    })
}

/// 从 WPS `fields` 提取业务键；只做字段读取，不做业务解析。
///
/// - 09：`电表` Link 的第一个 id + `\u{1f}` + `期间`；
/// - 02/03/04/05/06/07/08/10/11：对应单号/编号字段；
/// - 其余或缺失：`("", KIND_NONE)`，调用方不得用空键查询。
///
/// 字段名依据 `财务工作流/设计/项目台账_v2_设计.md`：
/// - 05 结算调整「调整编号」L159；
/// - 07 发票「发票号」L192；
/// - 08 电表「表号」L197；
/// - 10 物料「SKU」L205；
/// - 02 项目「项目编号」L92、03 应收「应收单号」L119、04 应付「应付单号」
///   L137、06 资金流水「流水号」L174、11 出入库「单号」L210（同文件对应表主字段）。
pub fn business_key_for(sheet_id: i64, fields: &Value) -> (String, &'static str) {
    if sheet_id == 9 {
        let meter = fields.get("电表").and_then(link_first_id);
        let period = fields.get("期间").and_then(field_text);
        return match (meter, period) {
            (Some(meter), Some(period)) if !meter.is_empty() && !period.is_empty() => {
                (format!("{meter}\u{1f}{period}"), KIND_METER_PERIOD)
            }
            _ => (String::new(), KIND_NONE),
        };
    }

    let field = match sheet_id {
        1 => "名称",
        2 => "项目编号",
        3 => "应收单号",
        4 => "应付单号",
        5 => "调整编号",
        6 => "流水号",
        7 => "发票号",
        8 => "表号",
        10 => "SKU",
        11 => "单号",
        _ => return (String::new(), KIND_NONE),
    };
    match fields.get(field).and_then(field_text) {
        Some(key) if !key.is_empty() => (key, KIND_DOC_NO),
        _ => (String::new(), KIND_NONE),
    }
}

fn field_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn link_first_id(value: &Value) -> Option<String> {
    if let Some(items) = value.as_array() {
        return items
            .first()
            .and_then(|item| item.as_str())
            .map(str::to_string);
    }
    if let Some(object) = value.as_object() {
        if let Some(items) = object.get("linkedRecordIds").and_then(Value::as_array) {
            return items
                .first()
                .and_then(|item| item.as_str())
                .map(str::to_string);
        }
        if let Some(id) = object.get("linkedRecordId").and_then(Value::as_str) {
            return Some(id.to_string());
        }
    }
    value.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{schema, Database};
    use rusqlite::Connection;
    use serde_json::json;

    fn db_with_schema() -> Database {
        let conn = Connection::open_in_memory().expect("open memory db");
        schema::init(&conn).expect("init schema");
        Database::new(conn)
    }

    fn row(record_id: &str, key: &str, kind: &str) -> WpsIndexRow {
        WpsIndexRow::new(
            3,
            record_id,
            "03",
            key,
            kind,
            None,
            r#"{"应收单号":"AR-1"}"#,
            "2026-11-19T00:00:00Z",
        )
    }

    #[test]
    fn begin_and_finish_scan_roundtrip() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let scan_id = store.begin_scan(9, None).expect("begin scan");
        store
            .finish_scan(scan_id, 3, true, None)
            .expect("finish scan");

        let (pages, complete, finished): (i64, i64, Option<String>) = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT pages, complete, finished_at FROM wps_scans WHERE id = ?1",
                    params![scan_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(Into::into)
            })
            .expect("query scan");
        assert_eq!(pages, 3);
        assert_eq!(complete, 1);
        assert!(finished.is_some());
    }

    #[test]
    fn finish_scan_records_error_and_incomplete() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let scan_id = store.begin_scan(3, Some("view-1")).expect("begin");
        store
            .finish_scan(scan_id, 0, false, Some("timeout"))
            .expect("finish");

        let (complete, error): (i64, Option<String>) = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT complete, error FROM wps_scans WHERE id = ?1",
                    params![scan_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(Into::into)
            })
            .expect("query scan");
        assert_eq!(complete, 0);
        assert_eq!(error.as_deref(), Some("timeout"));

        let inconsistent = store.begin_scan(3, None).expect("begin inconsistent scan");
        let err = store
            .finish_scan(inconsistent, 1, true, Some("boom"))
            .expect_err("complete=true 且 error 非空必须拒绝");
        assert!(
            matches!(err, CoreError::Invalid(ref message) if message.contains("WPS_E_SCAN_INCONSISTENT")),
            "unexpected error: {err:?}"
        );
        let still_open: Option<String> = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT finished_at FROM wps_scans WHERE id = ?1",
                    params![inconsistent],
                    |row| row.get(0),
                )
                .map_err(Into::into)
            })
            .expect("query inconsistent scan");
        assert!(still_open.is_none(), "拒绝后不得写入 finished_at");
    }

    #[test]
    fn index_upsert_is_idempotent_and_updates_fields() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let mut first = row("rec-1", "AR-1", KIND_DOC_NO);
        store
            .upsert_index_rows(&[first.clone()])
            .expect("first upsert");
        first.fields_json = r#"{"应收单号":"AR-1","金额":1100}"#.to_string();
        store.upsert_index_rows(&[first]).expect("second upsert");

        assert_eq!(store.count_index(3).expect("count"), 1);
        let found = store
            .find_index_by_record_id(3, "rec-1")
            .expect("find")
            .expect("row");
        assert!(found.fields_json.contains("1100"));
    }

    #[test]
    fn index_find_by_business_key_returns_duplicates() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        store
            .upsert_index_rows(&[
                row("rec-1", "AR-1", KIND_DOC_NO),
                row("rec-2", "AR-1", KIND_DOC_NO),
            ])
            .expect("upsert");
        let found = store.find_index_by_business_key(3, "AR-1").expect("find");
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn index_09_composite_key_hits_only_same_period() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let key = "B1A\u{1f}2026-11".to_string();
        let mut october = row("meter-oct", "B1A\u{1f}2026-10", KIND_METER_PERIOD);
        october.sheet_id = 9;
        october.record_type = "09".to_string();
        let mut november = row("meter-nov", &key, KIND_METER_PERIOD);
        november.sheet_id = 9;
        november.record_type = "09".to_string();
        store
            .upsert_index_rows(&[october, november])
            .expect("upsert");
        let found = store.find_index_by_business_key(9, &key).expect("find");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].record_id, "meter-nov");
    }

    #[test]
    fn empty_doc_no_is_marked_none_and_not_used_as_key() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let (key, kind) = business_key_for(3, &json!({ "应收单号": "  " }));
        assert_eq!(key, "");
        assert_eq!(kind, KIND_NONE);
        let mut missing = row("rec-missing", "", KIND_NONE);
        missing.fields_json = r#"{"应收单号":""}"#.to_string();
        store.upsert_index_rows(&[missing]).expect("upsert");
        assert!(store
            .find_index_by_business_key(3, "")
            .expect("find empty")
            .iter()
            .any(|row| row.business_key_kind == KIND_NONE));
        assert!(store
            .find_index_by_business_key(3, "AR-1")
            .expect("find real")
            .is_empty());
    }

    #[test]
    fn business_key_for_reads_doc_no_and_meter_period() {
        let (key, kind) = business_key_for(3, &json!({ "应收单号": " AR-1 " }));
        assert_eq!(key, "AR-1");
        assert_eq!(kind, KIND_DOC_NO);

        let (key, kind) = business_key_for(9, &json!({ "电表": ["B1A"], "期间": "2026-11" }));
        assert_eq!(key, "B1A\u{1f}2026-11");
        assert_eq!(kind, KIND_METER_PERIOD);

        let (key, kind) = business_key_for(9, &json!({ "期间": "2026-11" }));
        assert_eq!(key, "");
        assert_eq!(kind, KIND_NONE);
    }

    #[test]
    fn business_key_for_uses_documented_main_fields_for_05_07_08_10() {
        // 依据 财务工作流/设计/项目台账_v2_设计.md：
        // 05 L159「调整编号」、07 L192「发票号」、08 L197「表号」、10 L205「SKU」。
        let cases = [
            (5, "调整编号", "ADJ-1"),
            (7, "发票号", "INV-1"),
            (8, "表号", "METER-1"),
            (10, "SKU", "SKU-1"),
        ];
        for (sheet_id, field, value) in cases {
            let (key, kind) = business_key_for(sheet_id, &json!({ field: value }));
            assert_eq!(key, value, "sheet {sheet_id} 主字段 {field}");
            assert_eq!(kind, KIND_DOC_NO);
        }
    }
}
