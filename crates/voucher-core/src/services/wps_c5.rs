//! C5 一致性证据（本地）：对本地索引里的**全部字段**做指纹，用于两次扫描的稳定性比对。
//!
//! 边界（务必连同代码一起读）：
//!
//! - 这里只回答「本地索引在两次扫描之间是否逐字段一致」，**不能**证明「WPS 源端与本地一致」，
//!   因为 `dbsheet.list_records` 不提供快照 token，源端在两次扫描之间若发生过但被两次读到同样
//!   的（错误）值，也会被判为稳定。
//! - 因此本模块产出的是**辅助证据**：它能排除「扫描期间数据在变」这一类常见风险，但不能替代
//!   真实副本 C5 验收，也不能解除 `writes_blocked`。
//!
//! 指纹口径：
//!
//! - 逐表按 `record_id` 升序，把 `(record_id, 规范化后的 fields_json)` 以 `|` 连接成行；
//! - `fields_json` 先解析再重新序列化（`serde_json` 的 map 有序），避免键序/空白造成假差异；
//! - 单表指纹 = `sha256`（十六进制，小写）；整库指纹 = 对 `sheet_id|records|digest` 行再取 `sha256`。
use sha2::{Digest, Sha256};

use crate::services::wps::WpsStore;
use crate::{CoreError, Result};

/// 单表指纹。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetFingerprint {
    pub sheet_id: i64,
    pub records: u64,
    /// 逐记录全字段指纹（十六进制）。
    pub digest: String,
}

/// 整库指纹。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanFingerprint {
    pub sheets: Vec<SheetFingerprint>,
    pub records: u64,
    /// 覆盖所有表的全库指纹（十六进制）。
    pub combined: String,
}

/// 两次指纹的比对结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintComparison {
    /// 两次指纹逐表一致（记录数、指纹都相同，且表集合相同）。
    pub stable: bool,
    /// 发生变化的表。
    pub changed_sheets: Vec<i64>,
    /// 人类可读的原因（只含表号与计数，不含任何字段值）。
    pub reasons: Vec<String>,
}

impl ScanFingerprint {
    /// 与另一次扫描比对；只返回表号与计数，不回显字段内容。
    pub fn compare(&self, other: &ScanFingerprint) -> FingerprintComparison {
        let mut changed = Vec::new();
        let mut reasons = Vec::new();
        if self.records != other.records {
            reasons.push(format!(
                "记录总数不同：{} vs {}",
                self.records, other.records
            ));
        }
        let mut sheet_ids: Vec<i64> = self
            .sheets
            .iter()
            .map(|sheet| sheet.sheet_id)
            .chain(other.sheets.iter().map(|sheet| sheet.sheet_id))
            .collect();
        sheet_ids.sort_unstable();
        sheet_ids.dedup();
        for sheet_id in sheet_ids {
            let left = self.sheets.iter().find(|s| s.sheet_id == sheet_id);
            let right = other.sheets.iter().find(|s| s.sheet_id == sheet_id);
            match (left, right) {
                (Some(left), Some(right))
                    if left.records == right.records && left.digest == right.digest => {}
                (Some(left), Some(right)) => {
                    changed.push(sheet_id);
                    reasons.push(format!(
                        "表 {sheet_id} 字段指纹不同：{} 条/{} vs {} 条/{}",
                        left.records,
                        short(&left.digest),
                        right.records,
                        short(&right.digest)
                    ));
                }
                (Some(left), None) => {
                    changed.push(sheet_id);
                    reasons.push(format!(
                        "表 {sheet_id} 只在第一次扫描出现（{} 条）",
                        left.records
                    ));
                }
                (None, Some(right)) => {
                    changed.push(sheet_id);
                    reasons.push(format!(
                        "表 {sheet_id} 只在第二次扫描出现（{} 条）",
                        right.records
                    ));
                }
                (None, None) => {}
            }
        }
        FingerprintComparison {
            stable: changed.is_empty(),
            changed_sheets: changed,
            reasons,
        }
    }
}

impl<'a> WpsStore<'a> {
    /// 计算当前本地索引的全字段指纹；只读，不发网、不改数据。
    pub fn c5_scan_fingerprint(&self) -> Result<ScanFingerprint> {
        let per_sheet: Vec<(i64, u64, String)> = self.database().with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT sheet_id, record_id, fields_json
                   FROM wps_record_index
               ORDER BY sheet_id, record_id",
            )?;
            let mut rows = stmt.query([])?;
            let mut out: Vec<(i64, u64, String)> = Vec::new();
            let mut current: Option<(i64, Sha256, u64)> = None;
            while let Some(row) = rows.next()? {
                let sheet_id: i64 = row.get(0)?;
                let record_id: String = row.get(1)?;
                let fields_json: String = row.get(2)?;
                let canonical = canonical_fields(&fields_json)?;
                if current.as_ref().map(|(id, _, _)| *id) != Some(sheet_id) {
                    if let Some((id, hasher, count)) = current.take() {
                        out.push((id, count, hex(hasher.finalize())));
                    }
                    current = Some((sheet_id, Sha256::new(), 0));
                }
                if let Some((_, hasher, count)) = current.as_mut() {
                    hasher.update(record_id.as_bytes());
                    hasher.update(b"|");
                    hasher.update(canonical.as_bytes());
                    hasher.update(b"\n");
                    *count += 1;
                }
            }
            if let Some((id, hasher, count)) = current.take() {
                out.push((id, count, hex(hasher.finalize())));
            }
            Ok(out)
        })?;

        let mut sheets = Vec::with_capacity(per_sheet.len());
        let mut records = 0u64;
        let mut combined = Sha256::new();
        for (sheet_id, count, digest) in per_sheet {
            records += count;
            combined.update(format!("{sheet_id}|{count}|{digest}\n").as_bytes());
            sheets.push(SheetFingerprint {
                sheet_id,
                records: count,
                digest,
            });
        }
        Ok(ScanFingerprint {
            sheets,
            records,
            combined: hex(combined.finalize()),
        })
    }
}

/// 解析后重新序列化，消除键序与空白差异；非法 JSON 直接报错（不静默跳过）。
fn canonical_fields(fields_json: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(fields_json)
        .map_err(|err| CoreError::Invalid(format!("字段 JSON 无法解析：{err}")))?;
    serde_json::to_string(&value)
        .map_err(|err| CoreError::Invalid(format!("字段 JSON 无法序列化：{err}")))
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    let mut out = String::with_capacity(bytes.as_ref().len() * 2);
    for byte in bytes.as_ref() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn short(digest: &str) -> &str {
    digest.get(..12).unwrap_or(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{schema, Database};
    use rusqlite::Connection;

    fn store() -> Database {
        let conn = Connection::open_in_memory().expect("open memory db");
        schema::init(&conn).expect("init schema");
        Database::new(conn)
    }

    fn insert(db: &Database, sheet_id: i64, record_id: &str, fields: &str) {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO wps_record_index
                   (sheet_id, record_id, record_type, business_key, business_key_kind,
                    remote_modified_at, fields_json, pulled_at)
                 VALUES (?1, ?2, '03', '', 'none', NULL, ?3, '2026-11-23T00:00:00Z')",
                rusqlite::params![sheet_id, record_id, fields],
            )?;
            Ok(())
        })
        .expect("insert index row");
    }

    #[test]
    fn identical_index_yields_identical_fingerprint() {
        let db = store();
        insert(&db, 3, "r1", r#"{"code":"A","amount":1}"#);
        insert(&db, 3, "r2", r#"{"code":"B","amount":2}"#);
        let store = WpsStore::from_db(&db);
        let first = store.c5_scan_fingerprint().expect("first");
        let second = store.c5_scan_fingerprint().expect("second");
        assert_eq!(first, second);
        assert_eq!(first.records, 2);
        assert!(first.compare(&second).stable);
    }

    #[test]
    fn field_change_is_detected_per_sheet() {
        let db = store();
        insert(&db, 3, "r1", r#"{"code":"A","amount":1}"#);
        insert(&db, 3, "r2", r#"{"code":"B","amount":2}"#);
        let store = WpsStore::from_db(&db);
        let before = store.c5_scan_fingerprint().expect("before");

        db.with_conn(|conn| {
            conn.execute(
                "UPDATE wps_record_index SET fields_json = '{\"code\":\"B\",\"amount\":3}' \
                  WHERE sheet_id = 3 AND record_id = 'r2'",
                [],
            )?;
            Ok(())
        })
        .expect("update");

        let after = store.c5_scan_fingerprint().expect("after");
        let cmp = before.compare(&after);
        assert!(!cmp.stable);
        assert_eq!(cmp.changed_sheets, vec![3]);
        assert!(cmp.reasons.iter().any(|r| r.contains("表 3")));
    }

    #[test]
    fn key_order_and_whitespace_do_not_create_false_diff() {
        let db = store();
        insert(&db, 3, "r1", r#"{"code":"A","amount":1}"#);
        let store = WpsStore::from_db(&db);
        let before = store.c5_scan_fingerprint().expect("before");
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE wps_record_index SET fields_json = '{ \"amount\": 1, \"code\": \"A\" }'",
                [],
            )?;
            Ok(())
        })
        .expect("update");
        let after = store.c5_scan_fingerprint().expect("after");
        assert!(before.compare(&after).stable);
    }

    #[test]
    fn added_and_removed_sheets_are_reported() {
        let db = store();
        insert(&db, 3, "r1", r#"{"code":"A"}"#);
        let store = WpsStore::from_db(&db);
        let before = store.c5_scan_fingerprint().expect("before");
        insert(&db, 9, "r9", r#"{"code":"Z"}"#);
        let after = store.c5_scan_fingerprint().expect("after");
        let cmp = before.compare(&after);
        assert!(!cmp.stable);
        assert_eq!(cmp.changed_sheets, vec![9]);
    }

    #[test]
    fn mock_pull_is_stable_and_drift_is_detected() {
        // 用 Mock 全量拉取填本地索引，再比对两次指纹；随后手工制造一处字段漂移做反向对照。
        let db = store();
        let store = WpsStore::from_db(&db);
        let run = crate::services::wps_w2_dev::dev_pull(&store).expect("mock pull");
        assert!(run.pull.as_ref().expect("pull").complete);

        let first = store.c5_scan_fingerprint().expect("first");
        let second = store.c5_scan_fingerprint().expect("second");
        assert!(first.records > 0);
        assert_eq!(first, second);
        assert!(first.compare(&second).stable);
        println!(
            "C5_FP records={} combined={} sheets={} stable=true",
            first.records,
            &first.combined[..12],
            first.sheets.len()
        );

        // 反向对照：只改一条记录的字段值，指纹必须变，且指到具体表。
        let target_sheet = first
            .sheets
            .iter()
            .find(|sheet| sheet.records > 0)
            .expect("non-empty sheet")
            .sheet_id;
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE wps_record_index SET fields_json = '{\"code\":\"DRIFT\"}' \
                  WHERE sheet_id = ?1 \
                    AND record_id = (SELECT record_id FROM wps_record_index WHERE sheet_id = ?1 LIMIT 1)",
                rusqlite::params![target_sheet],
            )?;
            Ok(())
        })
        .expect("drift");
        let drifted = store.c5_scan_fingerprint().expect("drifted");
        let cmp = first.compare(&drifted);
        assert!(!cmp.stable);
        assert_eq!(cmp.changed_sheets, vec![target_sheet]);
        println!(
            "C5_FP_DRIFT changed_sheets={:?} reason={:?}",
            cmp.changed_sheets, cmp.reasons
        );
    }

    #[test]
    fn invalid_fields_json_is_an_error_not_a_silent_skip() {
        let db = store();
        insert(&db, 3, "r1", "not-json");
        let store = WpsStore::from_db(&db);
        assert!(store.c5_scan_fingerprint().is_err());
    }
}
