//! 扫描运行（scan-run）的离线契约测试：不连真实端点，只用本地桩传输。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{json, Value};
use voucher_core::services::wps_p2::{
    MemoryConsumption, P2Credential, P2Endpoint, P2Error, P2ScanRun, P2ScanSpec, P2Transport,
};

#[derive(Debug, Clone, Copy)]
enum Mode {
    Stable,
    LeakField,
    Alternate,
    ShortCursor,
}

struct Scripted {
    calls: AtomicUsize,
    mode: Mode,
    bodies: Mutex<Vec<Value>>,
}

impl Scripted {
    fn new(mode: Mode) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            mode,
            bodies: Mutex::new(Vec::new()),
        }
    }
}

struct Tok;

impl P2Credential for Tok {
    fn access_token(&self) -> Result<String, P2Error> {
        Ok("synthetic-scan-token".into())
    }
    fn subject(&self) -> Option<String> {
        Some("tenant-a/reader".into())
    }
}

fn page(sheet_id: i64, cursor: Option<&str>, value: &str) -> Value {
    json!({
        "code": 0,
        "data": { "detail": {
            "records": [{ "id": format!("r{sheet_id}-{value}"), "fields": { "A": value } }],
            "offset": cursor,
        }}
    })
}

impl P2Transport for Scripted {
    fn initialize(
        &self,
        _endpoint: &P2Endpoint,
        _auth: Option<&str>,
        _deadline: Duration,
    ) -> Result<String, P2Error> {
        Ok("scan-session".into())
    }
    fn call(
        &self,
        _endpoint: &P2Endpoint,
        _session: &str,
        _action: &str,
        args: &Value,
        _auth: Option<&str>,
        _deadline: Duration,
    ) -> Result<Value, P2Error> {
        self.bodies.lock().unwrap().push(args.clone());
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let sheet = args["sheet_id"].as_i64().unwrap_or_default();
        let offset = args["offset"].as_str().unwrap_or_default();
        match self.mode {
            Mode::LeakField => Ok(json!({
                "code": 0,
                "data": { "detail": { "records": [{ "id": "leak", "fields": { "A": "v", "C": "未批准" } }], "offset": null } }
            })),
            Mode::Stable => {
                let cursor = if sheet == 1 && offset.is_empty() {
                    Some("c1")
                } else {
                    None
                };
                Ok(page(sheet, cursor, "v"))
            }
            Mode::Alternate => Ok(page(sheet, None, &format!("v{n}"))),
            Mode::ShortCursor => {
                if sheet == 2 && offset.is_empty() {
                    Ok(page(sheet, None, "v"))
                } else {
                    Ok(page(sheet, Some("c2"), "v"))
                }
            }
        }
    }
}

fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 24, 2, minute, 0).unwrap()
}

fn spec(pages: &[(i64, u32)], max_reads: u32, max_total: u32) -> P2ScanSpec {
    let mut sheets = BTreeMap::new();
    let mut page_map = BTreeMap::new();
    for (sheet, count) in pages {
        sheets.insert(*sheet, BTreeSet::from(["A".to_string()]));
        page_map.insert(*sheet, *count);
    }
    P2ScanSpec {
        endpoint: P2Endpoint::parse("https://mcp-center.wps.cn/skill_hub/mcp").unwrap(),
        file_id: "file_scan_fixture".into(),
        grant_id: "A2-SCAN-TEST-01".into(),
        approved_by: "测试签批人".into(),
        sheets,
        pages: page_map,
        page_size: 50,
        window_start: at(0),
        window_end: at(10),
        max_reads,
        max_total,
        terminate_allowed: true,
    }
}

#[test]
fn scan_spec_validation_rejects_out_of_contract_ranges() {
    let mut too_long = spec(&[(1, 1), (2, 1)], 4, 12);
    too_long.window_end = at(40);
    assert!(matches!(too_long.validate(), Err(P2Error::Window(_))));

    let too_few_reads = spec(&[(1, 2), (2, 2)], 6, 12);
    assert!(matches!(
        too_few_reads.validate(),
        Err(P2Error::ExecLimit(_))
    ));

    let mut pages_gap = spec(&[(1, 1), (2, 1)], 4, 12);
    pages_gap.pages.insert(3, 2);
    assert!(matches!(pages_gap.validate(), Err(P2Error::Scope(_))));

    let mut no_signer = spec(&[(1, 1)], 2, 12);
    no_signer.approved_by.clear();
    assert!(matches!(no_signer.validate(), Err(P2Error::NotApproved)));
}

#[test]
fn scan_reads_two_passes_and_consumes_grant_once() {
    let transport = Scripted::new(Mode::Stable);
    let consumption = Arc::new(MemoryConsumption::default());
    let run = P2ScanRun::new(transport, Some(Arc::new(Tok)))
        .with_state(consumption.clone())
        .with_clock(Arc::new(FixedClock(at(1))));
    let outcome = run.run(&spec(&[(1, 2), (2, 1)], 6, 14)).unwrap();
    assert!(outcome.stable);
    assert_eq!(outcome.reads, 6);
    assert_eq!(outcome.pass1.len(), 2);
    assert_eq!(outcome.pass1[0].digest, outcome.pass2[0].digest);

    let again = run.run(&spec(&[(1, 2), (2, 1)], 6, 14));
    assert!(matches!(again, Err(P2Error::AlreadyUsed(_))));
}

#[test]
fn scan_detects_digest_mismatch_between_passes() {
    let run = P2ScanRun::new(Scripted::new(Mode::Alternate), Some(Arc::new(Tok)))
        .with_state(Arc::new(MemoryConsumption::default()))
        .with_clock(Arc::new(FixedClock(at(1))));
    let err = run.run(&spec(&[(1, 1), (2, 1)], 4, 12)).unwrap_err();
    assert_eq!(err.code(), "P2_E_INCONSISTENT");
}

#[test]
fn scan_detects_missing_page_in_second_pass() {
    let run = P2ScanRun::new(Scripted::new(Mode::ShortCursor), Some(Arc::new(Tok)))
        .with_state(Arc::new(MemoryConsumption::default()))
        .with_clock(Arc::new(FixedClock(at(1))));
    let err = run.run(&spec(&[(2, 2)], 4, 12)).unwrap_err();
    assert_eq!(err.code(), "P2_E_INCOMPLETE");
}

#[test]
fn scan_rejects_unapproved_field_before_counting_records() {
    let spec = spec(&[(1, 1)], 2, 12);
    let run = P2ScanRun::new(Scripted::new(Mode::LeakField), Some(Arc::new(Tok)))
        .with_state(Arc::new(MemoryConsumption::default()))
        .with_clock(Arc::new(FixedClock(at(1))));
    let err = run.run(&spec).unwrap_err();
    assert_eq!(err.code(), "P2_E_SCOPE");
    assert!(err.to_string().contains("表 1"));
    assert!(!err.to_string().contains("未批准"));
}

struct FixedClock(DateTime<Utc>);

impl voucher_core::services::wps_p2::P2Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}
