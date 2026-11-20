//! W2 Mock 业务链路（v0.28）。
//!
//! 本模块只在 `transport=Mock` 下运行：复用 W1 的分页拉取、索引、单号守卫和 09
//! 守卫，把「拉取 → 整批预检 → 逐条写入 → 本地链接回填」串成可测试的链路。
//! 不包含真实 HTTP、生产写入、自动调度、删除/墓碑/三方合并。
//!
//! 写入前先落 `wps_write_intents`；远端明确成功后写 `wps_links`。结果不明或本地
//! 回填失败时停止后续记录，后续项只标记 `NotAttempted`，不自动重试。

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use chrono::{DateTime, FixedOffset, NaiveDate};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use crate::services::wps::{extract_remote_record_id, WpsClient, WpsStore, WpsTransport};
use crate::services::wps_09::Wps09CreateGuard;
use crate::services::wps_doc_guard::{doc_spec_for_sheet, WpsDocNoCreateGuard};
use crate::services::wps_index::business_key_for;
use crate::services::wps_pull::{pull_sheet, PullReport};
use crate::{CoreError, Result};

/// 2026-11-01（含）进入新流程；之前为历史只读/查重/引用。
pub const SYNC_CUTOFF_DATE: &str = "2026-11-01";

/// 只做原记录引用的基础资料表（不代建、不写回）。
pub const SHEET_REFERENCE_ONLY: [i64; 3] = [1, 8, 10];
/// 首批只做全表查重、不生成业务行的表。
pub const SHEET_DEDUPE_ONLY: [i64; 2] = [5, 7];

/// 首批拉取的一张表；`record_type` 用于索引与错误提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct W2Sheet {
    pub sheet_id: i64,
    pub record_type: &'static str,
}

/// 首批全部 11 张业务表；00 说明表不纳入。
pub const W2_SHEETS: [W2Sheet; 11] = [
    W2Sheet {
        sheet_id: 1,
        record_type: "01",
    },
    W2Sheet {
        sheet_id: 2,
        record_type: "02",
    },
    W2Sheet {
        sheet_id: 3,
        record_type: "03",
    },
    W2Sheet {
        sheet_id: 4,
        record_type: "04",
    },
    W2Sheet {
        sheet_id: 5,
        record_type: "05",
    },
    W2Sheet {
        sheet_id: 6,
        record_type: "06",
    },
    W2Sheet {
        sheet_id: 7,
        record_type: "07",
    },
    W2Sheet {
        sheet_id: 8,
        record_type: "08",
    },
    W2Sheet {
        sheet_id: 9,
        record_type: "09",
    },
    W2Sheet {
        sheet_id: 10,
        record_type: "10",
    },
    W2Sheet {
        sheet_id: 11,
        record_type: "11",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum W2Mode {
    /// 默认：WPS 唯一录入，软件只拉取与展示。
    #[default]
    PullOnly,
    /// 仅测试（Mock）显式启用软件写入。
    MockWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntryOwner {
    #[default]
    Wps,
    Software,
}

/// 首批运行配置。默认 `PullOnly`；软件写入必须显式 `MockWrite + Software + 互斥已确认`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct W2Config {
    pub mode: W2Mode,
    pub entry_owner: EntryOwner,
    pub entry_exclusive: bool,
}

impl Default for W2Config {
    fn default() -> Self {
        Self {
            mode: W2Mode::PullOnly,
            entry_owner: EntryOwner::Wps,
            entry_exclusive: false,
        }
    }
}

impl W2Config {
    /// 测试/受控 Mock 写配置：必须由调用方明确使用，不来自生产默认。
    pub fn mock_write() -> Self {
        Self {
            mode: W2Mode::MockWrite,
            entry_owner: EntryOwner::Software,
            entry_exclusive: true,
        }
    }

    fn validate_write(&self) -> std::result::Result<(), W2Reject> {
        if self.mode != W2Mode::MockWrite {
            return Err(W2Reject::new(
                "WPS_E_ENTRY_PERMISSION",
                "首批默认 WPS 唯一录入，软件只拉取；未显式启用 Mock 写",
                None,
            ));
        }
        if self.entry_owner != EntryOwner::Software || !self.entry_exclusive {
            return Err(W2Reject::new(
                "WPS_E_ENTRY_PERMISSION",
                "软件写入要求录入权已交接且互斥已确认",
                None,
            ));
        }
        Ok(())
    }
}

/// 业务日期分类；`NeedsReview` 保留原因，调用方不得用录入日兜底。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateClass {
    New(NaiveDate),
    Legacy(NaiveDate),
    NeedsReview(String),
}

/// 按自然日切分；支持 `YYYY-MM-DD`、`YYYY/MM/DD` 和 RFC3339（按 Asia/Shanghai）。
pub fn classify_date(raw: &str) -> DateClass {
    let text = raw.trim();
    if text.is_empty() {
        return DateClass::NeedsReview("日期为空".into());
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        if let Some(offset) = FixedOffset::east_opt(8 * 3600) {
            return classify_naive(dt.with_timezone(&offset).date_naive());
        }
        return DateClass::NeedsReview("无法确定东八区时区偏移".into());
    }
    let head = text
        .split_once('T')
        .map(|(date, _)| date)
        .or_else(|| text.split_once(' ').map(|(date, _)| date))
        .unwrap_or(text);
    for format in ["%Y-%m-%d", "%Y/%m/%d"] {
        if let Ok(date) = NaiveDate::parse_from_str(head, format) {
            return classify_naive(date);
        }
    }
    DateClass::NeedsReview(format!("无法解析日期: {text}"))
}

fn classify_naive(date: NaiveDate) -> DateClass {
    let day = date.format("%Y-%m-%d").to_string();
    if day.as_str() < SYNC_CUTOFF_DATE {
        DateClass::Legacy(date)
    } else {
        DateClass::New(date)
    }
}

/// 09 期间分类；只接受 `YYYY-MM`，`2026-11` 起为新流程。
pub fn classify_period(raw: &str) -> DateClass {
    let text = raw.trim();
    let Some((year, month)) = text.split_once('-') else {
        return DateClass::NeedsReview(format!("无法解析期间: {text}"));
    };
    let (Ok(year), Ok(month)) = (year.parse::<i32>(), month.parse::<u32>()) else {
        return DateClass::NeedsReview(format!("无法解析期间: {text}"));
    };
    if !(1..=12).contains(&month) {
        return DateClass::NeedsReview(format!("期间月份无效: {text}"));
    }
    let Some(date) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return DateClass::NeedsReview(format!("期间无效: {text}"));
    };
    classify_naive(date)
}

/// 待创建记录；`fields` 即 WPS `fields` 对象。
#[derive(Debug, Clone)]
pub struct W2CreateRequest {
    pub sheet_id: i64,
    pub record_type: String,
    pub fields: Value,
    /// 02/03/04/06/11 的业务日期；09 用 `period`。
    pub business_date: Option<String>,
    /// 09 期间（YYYY-MM）；其他表可为空。
    pub period: Option<String>,
    /// 必填引用：(sheet_id, 业务键)；用于确认基础资料记录存在。
    pub required_refs: Vec<(i64, String)>,
    /// 必填引用原记录 ID：(sheet_id, record_id)；#63 B 方案要求引用 WPS 维护的 01/08/10 原 ID。
    pub required_ref_ids: Vec<(i64, String)>,
    /// 本地目标凭证；为空时链接保持 `unclaimed`。
    pub target_voucher_id: Option<i64>,
    pub idempotency_key: String,
}

impl W2CreateRequest {
    pub fn new(
        sheet_id: i64,
        record_type: impl Into<String>,
        fields: Value,
        idempotency_key: impl Into<String>,
    ) -> Self {
        Self {
            sheet_id,
            record_type: record_type.into(),
            fields,
            business_date: None,
            period: None,
            required_refs: Vec::new(),
            required_ref_ids: Vec::new(),
            target_voucher_id: None,
            idempotency_key: idempotency_key.into(),
        }
    }

    fn business_identity(&self) -> (String, &'static str) {
        business_key_for(self.sheet_id, &self.fields)
    }

    fn date_text(&self) -> Option<&str> {
        if self.sheet_id == 9 {
            self.period
                .as_deref()
                .or_else(|| self.fields.get("期间").and_then(Value::as_str))
        } else {
            self.business_date.as_deref()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct W2Reject {
    pub code: &'static str,
    pub message: String,
    pub index: Option<usize>,
}

impl W2Reject {
    fn new(code: &'static str, message: impl Into<String>, index: Option<usize>) -> Self {
        Self {
            code,
            message: message.into(),
            index,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct W2Preflight {
    pub business_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum W2WriteStatus {
    Succeeded,
    Unknown,
    Rejected,
    PendingBackfill,
    NotAttempted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct W2Outcome {
    pub idempotency_key: String,
    pub status: W2WriteStatus,
    pub remote_record_id: Option<String>,
    pub link_id: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct W2BatchReport {
    pub run_id: Option<i64>,
    pub outcomes: Vec<W2Outcome>,
    pub stopped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct W2PullSheetReport {
    pub sheet_id: i64,
    pub report: PullReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct W2PullReport {
    pub sheets: Vec<W2PullSheetReport>,
    pub complete: bool,
    /// 任一张表不完整/有坏页时阻断依赖本轮索引的写入。
    pub writes_blocked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum W2SyncStatus {
    Displayed,
    Linked,
    Posted,
    Missing,
}

/// W2 业务链路；只在 Mock 下使用。
pub struct W2Sync<'a, T: WpsTransport> {
    client: &'a WpsClient<T>,
    store: &'a WpsStore<'a>,
}

/// 应用层 Mock 入口：固定 `W2Config::mock_write()`，不读配置、不接 UI、不发起真实 HTTP。
///
/// 调用方（桌面命令/center-server/测试）只需传入本地 store、Mock transport 与批次请求；
/// 预检、逐条写入、意图、回填和停止规则都在 `W2Sync` 内完成。
pub fn run_mock_batch<T: WpsTransport>(
    store: &WpsStore<'_>,
    transport: T,
    requests: &[W2CreateRequest],
) -> Result<W2BatchReport> {
    let client = WpsClient::new(transport, "mock://w2-batch");
    let sync = W2Sync::new(&client, store);
    let pull = sync.pull_all_history()?;
    if pull.writes_blocked {
        return Err(CoreError::Invalid(
            "WPS_E_PULL_BLOCKED: 全历史拉取不完整，整批阻断写入".into(),
        ));
    }
    sync.execute(&W2Config::mock_write(), requests)
}

impl<'a, T: WpsTransport> W2Sync<'a, T> {
    pub fn new(client: &'a WpsClient<T>, store: &'a WpsStore<'a>) -> Self {
        Self { client, store }
    }

    /// 首批 11 张表各跑一条全历史分页链；不传 view_id、不按日期切分。
    pub fn pull_all_history(&self) -> Result<W2PullReport> {
        let mut sheets = Vec::with_capacity(W2_SHEETS.len());
        let mut complete = true;
        for sheet in W2_SHEETS {
            let report = pull_sheet(
                self.client,
                self.store,
                sheet.sheet_id,
                sheet.record_type,
                None,
                500,
            )?;
            if !report.complete || report.skipped > 0 {
                complete = false;
            }
            sheets.push(W2PullSheetReport {
                sheet_id: sheet.sheet_id,
                report,
            });
        }
        Ok(W2PullReport {
            sheets,
            complete,
            writes_blocked: !complete,
        })
    }

    /// 整批预检；任何一项不通过即整批拒绝，调用方不得进入写入。
    pub fn preflight(
        &self,
        config: &W2Config,
        requests: &[W2CreateRequest],
    ) -> std::result::Result<W2Preflight, W2Reject> {
        config.validate_write()?;
        if requests.is_empty() {
            return Err(W2Reject::new("WPS_E_BATCH_EMPTY", "创建批次为空", None));
        }
        let mut batch_keys: HashSet<(i64, String)> = HashSet::new();
        let mut business_keys = Vec::with_capacity(requests.len());
        for (index, request) in requests.iter().enumerate() {
            if request.idempotency_key.trim().is_empty() {
                return Err(W2Reject::new(
                    "WPS_E_IDEMPOTENCY_MISSING",
                    "幂等键为空",
                    Some(index),
                ));
            }
            if SHEET_REFERENCE_ONLY.contains(&request.sheet_id) {
                return Err(W2Reject::new(
                    "WPS_E_REFERENCE_ONLY",
                    format!("表 {} 仅引用 WPS 原记录，不代建", request.sheet_id),
                    Some(index),
                ));
            }
            if SHEET_DEDUPE_ONLY.contains(&request.sheet_id) {
                return Err(W2Reject::new(
                    "WPS_E_DEDUPE_ONLY",
                    format!("表 {} 首批仅全表查重，不生成业务行", request.sheet_id),
                    Some(index),
                ));
            }
            if !matches!(request.sheet_id, 2 | 3 | 4 | 6 | 9 | 11) {
                return Err(W2Reject::new(
                    "WPS_E_SHEET_UNSUPPORTED",
                    format!("表 {} 不在首批写入范围", request.sheet_id),
                    Some(index),
                ));
            }
            let Some(date_text) = request.date_text() else {
                return Err(W2Reject::new(
                    "WPS_E_DATE_MISSING",
                    "缺少业务日期/期间，不能自动分类",
                    Some(index),
                ));
            };
            match if request.sheet_id == 9 {
                classify_period(date_text)
            } else {
                classify_date(date_text)
            } {
                DateClass::New(_) => {}
                DateClass::Legacy(_) => {
                    return Err(W2Reject::new(
                        "WPS_E_LEGACY_READONLY",
                        "2026-11-01 之前的记录仅历史查重/引用，不写入",
                        Some(index),
                    ));
                }
                DateClass::NeedsReview(reason) => {
                    return Err(W2Reject::new("WPS_E_DATE_INVALID", reason, Some(index)));
                }
            }

            let (business_key, kind) = request.business_identity();
            if business_key.is_empty() || kind == "none" {
                return Err(W2Reject::new(
                    "WPS_E_BUSINESS_KEY_MISSING",
                    "业务键为空，不能预检重复",
                    Some(index),
                ));
            }
            if !batch_keys.insert((request.sheet_id, business_key.clone())) {
                return Err(W2Reject::new(
                    "WPS_E_BATCH_DUPLICATE",
                    format!("批内业务键重复: {business_key}"),
                    Some(index),
                ));
            }
            if request.sheet_id == 9 {
                Wps09CreateGuard::ensure_index_unique(self.store, &business_key).map_err(
                    |err| {
                        let code = if err.code() == "WPS_E_09_DUPLICATE" {
                            "WPS_E_HISTORY_DUPLICATE"
                        } else {
                            err.code()
                        };
                        W2Reject::new(code, err.to_string(), Some(index))
                    },
                )?;
            } else if let Some(spec) = doc_spec_for_sheet(request.sheet_id) {
                WpsDocNoCreateGuard::ensure_index_unique(self.store, &spec, &business_key)
                    .map_err(|err| {
                        let code = if err.code() == "WPS_E_DOC_DUPLICATE" {
                            "WPS_E_HISTORY_DUPLICATE"
                        } else {
                            err.code()
                        };
                        W2Reject::new(code, err.to_string(), Some(index))
                    })?;
            } else {
                return Err(W2Reject::new(
                    "WPS_E_SHEET_UNSUPPORTED",
                    format!("表 {} 不在首批写入范围", request.sheet_id),
                    Some(index),
                ));
            }
            if self
                .store
                .idempotency_key(request.idempotency_key.trim())
                .map_err(|err| {
                    W2Reject::new(
                        "WPS_E_IDEMPOTENCY_STATE",
                        format!("读取幂等键失败: {err}"),
                        Some(index),
                    )
                })?
                .is_some()
            {
                return Err(W2Reject::new(
                    "WPS_E_IDEMPOTENCY_STATE",
                    "幂等键已存在，拒绝自动重试",
                    Some(index),
                ));
            }
            for (sheet_id, record_id) in &request.required_ref_ids {
                if !SHEET_REFERENCE_ONLY.contains(sheet_id) {
                    return Err(W2Reject::new(
                        "WPS_E_REF_INVALID",
                        format!("表 {sheet_id} 不是 01/08/10 基础资料引用表"),
                        Some(index),
                    ));
                }
                if record_id.trim().is_empty() {
                    return Err(W2Reject::new(
                        "WPS_E_REF_MISSING",
                        "必填引用原记录 ID 为空",
                        Some(index),
                    ));
                }
                let found = self
                    .store
                    .find_index_by_record_id(*sheet_id, record_id)
                    .map_err(|err| {
                        W2Reject::new(
                            "WPS_E_INDEX_READ",
                            format!("读取索引失败: {err}"),
                            Some(index),
                        )
                    })?;
                if found.is_none() {
                    return Err(W2Reject::new(
                        "WPS_E_REF_STALE",
                        format!("引用原记录不存在或已失效: 表 {sheet_id} / {record_id}"),
                        Some(index),
                    ));
                }
            }
            for (sheet_id, key) in &request.required_refs {
                if key.trim().is_empty() {
                    return Err(W2Reject::new(
                        "WPS_E_REF_MISSING",
                        "必填引用为空",
                        Some(index),
                    ));
                }
                let found = self
                    .store
                    .find_index_by_business_key(*sheet_id, key)
                    .map_err(|err| {
                        W2Reject::new(
                            "WPS_E_INDEX_READ",
                            format!("读取索引失败: {err}"),
                            Some(index),
                        )
                    })?;
                if found.is_empty() {
                    return Err(W2Reject::new(
                        "WPS_E_REF_MISSING",
                        format!("引用不存在: 表 {sheet_id} / {key}"),
                        Some(index),
                    ));
                }
            }
            if let Some(voucher_id) = request.target_voucher_id {
                let exists: bool = self
                    .store
                    .database()
                    .with_conn(|conn| {
                        let n: i64 = conn.query_row(
                            "SELECT COUNT(*) FROM vouchers WHERE id = ?1",
                            params![voucher_id],
                            |row| row.get(0),
                        )?;
                        Ok(n > 0)
                    })
                    .map_err(|err| {
                        W2Reject::new(
                            "WPS_E_TARGET_MISSING",
                            format!("读取目标凭证失败: {err}"),
                            Some(index),
                        )
                    })?;
                if !exists {
                    return Err(W2Reject::new(
                        "WPS_E_TARGET_MISSING",
                        format!("目标凭证 #{voucher_id} 不存在"),
                        Some(index),
                    ));
                }
            }
            business_keys.push(business_key);
        }
        Ok(W2Preflight { business_keys })
    }

    /// 预检通过后逐条创建；任一条停止即把后续标为 `NotAttempted`。
    pub fn execute(
        &self,
        config: &W2Config,
        requests: &[W2CreateRequest],
    ) -> Result<W2BatchReport> {
        let preflight = match self.preflight(config, requests) {
            Ok(value) => value,
            Err(reject) => {
                let mut outcomes: Vec<W2Outcome> = requests
                    .iter()
                    .map(|request| W2Outcome {
                        idempotency_key: request.idempotency_key.clone(),
                        status: W2WriteStatus::Rejected,
                        remote_record_id: None,
                        link_id: None,
                        error: Some(format!("{}: {}", reject.code, reject.message)),
                    })
                    .collect();
                // 只有触发拒绝的那一条及之后标为拒绝；前面未被尝试的也按拒绝处理，整批零写入。
                outcomes.iter_mut().for_each(|outcome| {
                    if outcome.status != W2WriteStatus::Rejected {
                        outcome.status = W2WriteStatus::NotAttempted;
                    }
                });
                return Ok(W2BatchReport {
                    run_id: None,
                    outcomes,
                    stopped: true,
                });
            }
        };

        if self.client.session_id().is_none() {
            if let Err(err) = self.client.initialize() {
                return Ok(W2BatchReport {
                    run_id: None,
                    outcomes: requests
                        .iter()
                        .map(|request| W2Outcome {
                            idempotency_key: request.idempotency_key.clone(),
                            status: W2WriteStatus::Rejected,
                            remote_record_id: None,
                            link_id: None,
                            error: Some(format!("WPS_E_SESSION: {err}")),
                        })
                        .collect(),
                    stopped: true,
                });
            }
        }
        let run_id = self.store.start_run("manual", "desktop")?;
        let batch_id = format!("w2-{run_id}");
        let mut outcomes = Vec::with_capacity(requests.len());
        let mut stopped = false;

        for (index, request) in requests.iter().enumerate() {
            if stopped {
                outcomes.push(W2Outcome {
                    idempotency_key: request.idempotency_key.clone(),
                    status: W2WriteStatus::NotAttempted,
                    remote_record_id: None,
                    link_id: None,
                    error: None,
                });
                continue;
            }

            let business_key = preflight.business_keys[index].clone();
            if let Err(err) = self.insert_intent(run_id, &batch_id, index, request, &business_key) {
                outcomes.push(W2Outcome {
                    idempotency_key: request.idempotency_key.clone(),
                    status: W2WriteStatus::Rejected,
                    remote_record_id: None,
                    link_id: None,
                    error: Some(format!("WPS_E_INTENT: {err}")),
                });
                stopped = true;
                continue;
            }
            if let Err(err) = self.store.insert_idempotency_key(
                request.idempotency_key.trim(),
                run_id,
                &request.record_type,
            ) {
                self.update_intent_state(
                    request.idempotency_key.trim(),
                    "remote_rejected",
                    None,
                    Some(&err.to_string()),
                )?;
                outcomes.push(W2Outcome {
                    idempotency_key: request.idempotency_key.clone(),
                    status: W2WriteStatus::Rejected,
                    remote_record_id: None,
                    link_id: None,
                    error: Some(format!("WPS_E_IDEMPOTENCY_STATE: {err}")),
                });
                stopped = true;
                continue;
            }

            let payload = json!({ "fields": request.fields });
            match self.client.create_record(
                request.sheet_id,
                &payload,
                request.idempotency_key.trim(),
            ) {
                Ok(value) => {
                    let Some(remote_id) = extract_remote_record_id(&value) else {
                        self.store.mark_idempotency_key(
                            request.idempotency_key.trim(),
                            "unknown",
                            None,
                        )?;
                        self.update_intent_state(
                            request.idempotency_key.trim(),
                            "unknown",
                            None,
                            Some("创建响应缺少记录 id"),
                        )?;
                        outcomes.push(W2Outcome {
                            idempotency_key: request.idempotency_key.clone(),
                            status: W2WriteStatus::Unknown,
                            remote_record_id: None,
                            link_id: None,
                            error: Some("WPS_E_REMOTE_ID: 响应缺少记录 id".into()),
                        });
                        stopped = true;
                        continue;
                    };
                    self.store.mark_idempotency_key(
                        request.idempotency_key.trim(),
                        "succeeded",
                        Some(&remote_id),
                    )?;
                    self.update_intent_state(
                        request.idempotency_key.trim(),
                        "remote_succeeded",
                        Some(&remote_id),
                        None,
                    )?;

                    match self.write_link(request, &remote_id) {
                        Ok(link_id) => {
                            self.update_intent_state(
                                request.idempotency_key.trim(),
                                "linked",
                                Some(&remote_id),
                                None,
                            )?;
                            outcomes.push(W2Outcome {
                                idempotency_key: request.idempotency_key.clone(),
                                status: W2WriteStatus::Succeeded,
                                remote_record_id: Some(remote_id),
                                link_id: Some(link_id),
                                error: None,
                            });
                        }
                        Err(err) => {
                            self.update_intent_state(
                                request.idempotency_key.trim(),
                                "pending_backfill",
                                Some(&remote_id),
                                Some(&err.to_string()),
                            )?;
                            outcomes.push(W2Outcome {
                                idempotency_key: request.idempotency_key.clone(),
                                status: W2WriteStatus::PendingBackfill,
                                remote_record_id: Some(remote_id),
                                link_id: None,
                                error: Some(format!("WPS_E_LINK_BACKFILL: {err}")),
                            });
                            stopped = true;
                        }
                    }
                }
                Err(err) => {
                    self.store.mark_idempotency_key(
                        request.idempotency_key.trim(),
                        "unknown",
                        None,
                    )?;
                    self.update_intent_state(
                        request.idempotency_key.trim(),
                        "unknown",
                        None,
                        Some(&err.to_string()),
                    )?;
                    outcomes.push(W2Outcome {
                        idempotency_key: request.idempotency_key.clone(),
                        status: W2WriteStatus::Unknown,
                        remote_record_id: None,
                        link_id: None,
                        error: Some(err.to_string()),
                    });
                    stopped = true;
                }
            }
        }

        let created = outcomes
            .iter()
            .filter(|outcome| outcome.status == W2WriteStatus::Succeeded)
            .count() as i64;
        self.store.finish_run(
            run_id,
            if stopped { "partial" } else { "ok" },
            requests.len() as i64,
            created,
            0,
            0,
            outcomes
                .iter()
                .filter(|outcome| outcome.status == W2WriteStatus::Unknown)
                .count() as i64,
            outcomes
                .iter()
                .filter(|outcome| {
                    matches!(
                        outcome.status,
                        W2WriteStatus::Rejected
                            | W2WriteStatus::Unknown
                            | W2WriteStatus::PendingBackfill
                    )
                })
                .count() as i64,
            0,
            None,
        )?;
        Ok(W2BatchReport {
            run_id: Some(run_id),
            outcomes,
            stopped,
        })
    }

    fn insert_intent(
        &self,
        run_id: i64,
        batch_id: &str,
        index: usize,
        request: &W2CreateRequest,
        business_key: &str,
    ) -> Result<()> {
        let hash = stable_hash(&request.fields);
        self.store.database().with_conn(|conn| {
            conn.execute(
                "INSERT INTO wps_write_intents
                    (run_id, batch_id, sequence, sheet_id, record_type, business_key,
                     target_voucher_id, idempotency_key, payload_hash, state, created_at, updated_at)
                 VALUES
                    (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                     'intent', datetime('now'), datetime('now'))",
                params![
                    run_id,
                    batch_id,
                    index as i64,
                    request.sheet_id,
                    request.record_type,
                    business_key,
                    request.target_voucher_id,
                    request.idempotency_key.trim(),
                    hash,
                ],
            )?;
            Ok(())
        })
    }

    fn update_intent_state(
        &self,
        idempotency_key: &str,
        state: &str,
        remote_record_id: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        self.store.database().with_conn(|conn| {
            conn.execute(
                "UPDATE wps_write_intents
                    SET state = ?2,
                        remote_record_id = COALESCE(?3, remote_record_id),
                        error = ?4,
                        updated_at = datetime('now')
                  WHERE idempotency_key = ?1",
                params![idempotency_key, state, remote_record_id, error],
            )?;
            Ok(())
        })
    }

    fn write_link(&self, request: &W2CreateRequest, remote_id: &str) -> Result<i64> {
        let state = if request.target_voucher_id.is_some() {
            "linked"
        } else {
            "unclaimed"
        };
        self.store.database().with_tx(|tx| {
            tx.execute(
                "INSERT INTO wps_links
                    (voucher_id, sheet_id, record_id, doc_type, origin, fields_json,
                     base_json, remote_json, targets_json, idempotency_key, state,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'wps', ?5, '{}', '{}', '{}', ?6, ?7,
                         datetime('now'), datetime('now'))
                 ON CONFLICT(sheet_id, record_id) DO UPDATE SET
                    voucher_id = excluded.voucher_id,
                    state = excluded.state,
                    fields_json = excluded.fields_json,
                    idempotency_key = excluded.idempotency_key,
                    updated_at = datetime('now')",
                params![
                    request.target_voucher_id,
                    request.sheet_id,
                    remote_id,
                    request.record_type,
                    request.fields.to_string(),
                    request.idempotency_key.trim(),
                    state,
                ],
            )?;
            let id: i64 = tx.query_row(
                "SELECT id FROM wps_links WHERE sheet_id = ?1 AND record_id = ?2",
                params![request.sheet_id, remote_id],
                |row| row.get(0),
            )?;
            Ok(id)
        })
    }

    /// 展示/关联/入账状态；`posted` 只由既有流程显式写入，不因同步自动设置。
    pub fn status(&self, sheet_id: i64, record_id: &str) -> Result<W2SyncStatus> {
        let link: Option<String> = self.store.database().with_conn(|conn| {
            conn.query_row(
                "SELECT state FROM wps_links WHERE sheet_id = ?1 AND record_id = ?2",
                params![sheet_id, record_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        })?;
        if let Some(state) = link {
            return Ok(match state.as_str() {
                "posted" => W2SyncStatus::Posted,
                "linked" => W2SyncStatus::Linked,
                _ => W2SyncStatus::Displayed,
            });
        }
        let exists: bool = self.store.database().with_conn(|conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM wps_record_index WHERE sheet_id = ?1 AND record_id = ?2",
                params![sheet_id, record_id],
                |row| row.get(0),
            )?;
            Ok(n > 0)
        })?;
        Ok(if exists {
            W2SyncStatus::Displayed
        } else {
            W2SyncStatus::Missing
        })
    }

    /// 恢复只补本地链：必须提供能证明本次请求归属的证据；证据为空直接拒绝。
    pub fn recover_link(
        &self,
        intent_id: i64,
        remote_record_id: &str,
        evidence: &str,
    ) -> Result<i64> {
        if evidence.trim().is_empty() {
            return Err(CoreError::Invalid(
                "WPS_E_EVIDENCE_REQUIRED: 缺少归属证据，不能补链".into(),
            ));
        }
        if remote_record_id.trim().is_empty() {
            return Err(CoreError::Invalid(
                "WPS_E_REMOTE_ID: 缺少远端记录 ID".into(),
            ));
        }
        let row: Option<(i64, i64, Option<i64>, String, String)> =
            self.store.database().with_conn(|conn| {
                conn.query_row(
                    "SELECT sheet_id, id, target_voucher_id, record_type, idempotency_key
                       FROM wps_write_intents WHERE id = ?1",
                    params![intent_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(Into::into)
            })?;
        let Some((sheet_id, _id, target_voucher_id, record_type, idempotency_key)) = row else {
            return Err(CoreError::NotFound(format!("写入意图 #{intent_id} 不存在")));
        };
        let request = W2CreateRequest {
            sheet_id,
            record_type,
            fields: json!({}),
            business_date: None,
            period: None,
            required_refs: Vec::new(),
            required_ref_ids: Vec::new(),
            target_voucher_id,
            idempotency_key: idempotency_key.clone(),
        };
        let link_id = self.write_link(&request, remote_record_id)?;
        self.update_intent_state(
            &idempotency_key,
            "linked",
            Some(remote_record_id),
            Some(evidence),
        )?;
        Ok(link_id)
    }

    #[cfg(test)]
    fn intent_state(&self, idempotency_key: &str) -> Result<Option<String>> {
        self.store.database().with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT state FROM wps_write_intents WHERE idempotency_key = ?1",
                    params![idempotency_key],
                    |row| row.get(0),
                )
                .optional()?)
        })
    }
}

fn stable_hash(value: &Value) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{schema, Database};
    use crate::services::wps::{MockTransport, WpsError};
    use crate::services::wps_index::WpsIndexRow;
    use rusqlite::Connection;
    use serde_json::json;

    fn db_with_schema() -> Database {
        let conn = Connection::open_in_memory().expect("open memory db");
        schema::init(&conn).expect("init schema");
        Database::new(conn)
    }

    fn request(key: &str, date: &str) -> W2CreateRequest {
        let mut request =
            W2CreateRequest::new(3, "03", json!({"应收单号": key}), format!("idem-{key}"));
        request.business_date = Some(date.to_string());
        request
    }

    #[test]
    fn classify_date_covers_cutoff_invalid_and_timezone() {
        assert!(matches!(classify_date("2026-10-31"), DateClass::Legacy(_)));
        assert!(matches!(classify_date("2026-11-01"), DateClass::New(_)));
        assert!(matches!(classify_date("2026/11/01"), DateClass::New(_)));
        assert!(matches!(
            classify_date("2026-10-31T16:30:00Z"),
            DateClass::New(_)
        ));
        assert!(matches!(
            classify_date("2026-13-01"),
            DateClass::NeedsReview(_)
        ));
        assert!(matches!(classify_date(""), DateClass::NeedsReview(_)));
    }

    #[test]
    fn classify_period_uses_month_boundary() {
        assert!(matches!(classify_period("2026-10"), DateClass::Legacy(_)));
        assert!(matches!(classify_period("2026-11"), DateClass::New(_)));
        assert!(matches!(
            classify_period("2026-13"),
            DateClass::NeedsReview(_)
        ));
    }

    #[test]
    fn preflight_requires_explicit_mock_write_permission() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
        let sync = W2Sync::new(&client, &store);
        let err = sync
            .preflight(&W2Config::default(), &[request("AR-1", "2026-11-01")])
            .expect_err("default pull-only");
        assert_eq!(err.code, "WPS_E_ENTRY_PERMISSION");
    }

    #[test]
    fn preflight_rejects_batch_duplicate_before_any_request() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
        let sync = W2Sync::new(&client, &store);
        let mut second = request("AR-1", "2026-11-01");
        second.idempotency_key = "idem-2".into();
        let err = sync
            .preflight(
                &W2Config::mock_write(),
                &[request("AR-1", "2026-11-01"), second],
            )
            .expect_err("duplicate");
        assert_eq!(err.code, "WPS_E_BATCH_DUPLICATE");
    }

    #[test]
    fn preflight_rejects_legacy_invalid_and_missing_ref() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
        let sync = W2Sync::new(&client, &store);

        let legacy = sync
            .preflight(&W2Config::mock_write(), &[request("AR-1", "2026-10-31")])
            .expect_err("legacy");
        assert_eq!(legacy.code, "WPS_E_LEGACY_READONLY");

        let invalid = sync
            .preflight(&W2Config::mock_write(), &[request("AR-2", "2026-13-01")])
            .expect_err("invalid");
        assert_eq!(invalid.code, "WPS_E_DATE_INVALID");

        let mut with_ref = request("AR-3", "2026-11-01");
        with_ref.required_refs = vec![(1, "客户-1".into())];
        let missing = sync
            .preflight(&W2Config::mock_write(), &[with_ref])
            .expect_err("missing ref");
        assert_eq!(missing.code, "WPS_E_REF_MISSING");
    }

    #[test]
    fn preflight_b_policy_requires_existing_original_reference_id() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        store
            .upsert_index_rows(&[WpsIndexRow::new(
                1,
                "dev-01-A",
                "01",
                "客户A",
                "doc_no",
                None,
                "{}",
                "2026-11-20T00:00:00Z",
            )])
            .expect("seed ref");
        let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
        let sync = W2Sync::new(&client, &store);

        let mut stale = request("AR-REF", "2026-11-01");
        stale.required_ref_ids = vec![(1, "dev-01-MISSING".into())];
        let err = sync
            .preflight(&W2Config::mock_write(), &[stale])
            .expect_err("stale ref");
        assert_eq!(err.code, "WPS_E_REF_STALE");

        let mut valid = request("AR-REF", "2026-11-01");
        valid.required_ref_ids = vec![(1, "dev-01-A".into())];
        assert!(sync.preflight(&W2Config::mock_write(), &[valid]).is_ok());
    }

    #[test]
    fn preflight_rejects_history_duplicate() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        store
            .upsert_index_rows(&[WpsIndexRow::new(
                3,
                "R-OLD",
                "03",
                "AR-1",
                "doc_no",
                None,
                "{}",
                "2026-11-20T00:00:00Z",
            )])
            .expect("seed index");
        let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
        let sync = W2Sync::new(&client, &store);
        let err = sync
            .preflight(&W2Config::mock_write(), &[request("AR-1", "2026-11-01")])
            .expect_err("history");
        assert_eq!(err.code, "WPS_E_HISTORY_DUPLICATE");
    }

    #[test]
    fn preflight_blocks_reference_and_dedupe_tables() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let client = WpsClient::new(MockTransport::new("s", vec![]), "file-1");
        let sync = W2Sync::new(&client, &store);
        for (sheet, record_type) in [(1, "01"), (5, "05"), (7, "07"), (8, "08"), (10, "10")] {
            let mut request = W2CreateRequest::new(
                sheet,
                record_type,
                json!({"客户": "客户-1"}),
                format!("idem-{record_type}"),
            );
            request.business_date = Some("2026-11-01".into());
            let err = sync
                .preflight(&W2Config::mock_write(), &[request])
                .expect_err("table must not create");
            assert!(
                err.code == "WPS_E_REFERENCE_ONLY" || err.code == "WPS_E_DEDUPE_ONLY",
                "unexpected code for {record_type}: {}",
                err.code
            );
            assert!(client.transport().calls().is_empty());
        }
    }

    #[test]
    fn application_mock_entry_runs_preflight_and_one_write() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let mut responses: Vec<std::result::Result<Value, WpsError>> = (0..W2_SHEETS.len())
            .map(|_| Ok(json!({"records": []})))
            .collect();
        responses.push(Ok(json!({"records": [{"id": "R-ENTRY"}]})));
        let transport = MockTransport::new("session-1", responses);
        let report = run_mock_batch(&store, transport, &[request("AR-ENTRY", "2026-11-01")])
            .expect("mock entry");
        assert_eq!(report.outcomes[0].status, W2WriteStatus::Succeeded);
        assert_eq!(
            report.outcomes[0].remote_record_id.as_deref(),
            Some("R-ENTRY")
        );
    }

    #[test]
    fn application_mock_entry_blocks_on_incomplete_pull() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let mut responses: Vec<std::result::Result<Value, WpsError>> =
            (0..3).map(|_| Err(WpsError::RateLimited)).collect();
        responses.extend((0..W2_SHEETS.len() - 1).map(|_| Ok(json!({"records": []}))));
        let transport = MockTransport::new("session-1", responses);
        let err = run_mock_batch(&store, transport, &[request("AR-BLOCKED", "2026-11-01")])
            .expect_err("incomplete pull must block");
        assert!(err.to_string().contains("WPS_E_PULL_BLOCKED"));
    }

    #[test]
    fn application_mock_entry_blocks_on_inconsistent_scan() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let empty = || Ok(json!({"records": []}));
        let mut responses = vec![
            Ok(json!({
                "records": [{"id": "r1", "fields": {"应收单号": "AR-1"}}],
                "total": 2,
                "has_more": true
            })),
            Ok(json!({
                "records": [{"id": "r1", "fields": {"应收单号": "AR-1"}}],
                "total": 2,
                "has_more": false
            })),
        ];
        responses.extend((0..10).map(|_| empty()));
        let transport = MockTransport::new("session-1", responses);
        let err = run_mock_batch(&store, transport, &[request("AR-1", "2026-11-01")])
            .expect_err("inconsistent scan must block writes");
        assert!(err.to_string().contains("WPS_E_PULL_BLOCKED"));
    }

    #[test]
    fn execute_creates_once_and_links_without_batch_api() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let transport =
            MockTransport::new("session-1", vec![Ok(json!({"records": [{"id": "R-NEW"}]}))]);
        let client = WpsClient::new(transport, "file-1");
        let sync = W2Sync::new(&client, &store);
        let report = sync
            .execute(&W2Config::mock_write(), &[request("AR-NEW", "2026-11-01")])
            .expect("execute");
        assert!(!report.stopped);
        assert_eq!(report.outcomes[0].status, W2WriteStatus::Succeeded);
        assert_eq!(
            report.outcomes[0].remote_record_id.as_deref(),
            Some("R-NEW")
        );
        assert!(report.outcomes[0].link_id.is_some());
        let calls = client.transport().calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].action, "dbsheet.create_records");
        assert_eq!(calls[0].args["idempotency_key"], "idem-AR-NEW");
        assert_eq!(
            sync.status(3, "R-NEW").expect("status"),
            W2SyncStatus::Displayed
        );
        assert_eq!(
            sync.intent_state("idem-AR-NEW").expect("intent"),
            Some("linked".into())
        );
    }

    #[test]
    fn preflight_history_duplicate_on_second_page_blocks_before_create() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let empty = || Ok(json!({"records": []}));
        let mut responses = vec![
            empty(),
            empty(),
            Ok(json!({
                "records": [{"id": "r0", "fields": {"应收单号": "AR-0"}}],
                "total": 2,
                "has_more": true
            })),
            Ok(json!({
                "records": [{"id": "r1", "fields": {"应收单号": "AR-1"}}],
                "total": 2,
                "has_more": false
            })),
        ];
        responses.extend((0..8).map(|_| empty()));
        let transport = MockTransport::new("session-1", responses);
        let report =
            run_mock_batch(&store, transport, &[request("AR-1", "2026-11-01")]).expect("run");
        assert!(report.stopped);
        assert_eq!(report.outcomes[0].status, W2WriteStatus::Rejected);
        assert!(report.outcomes[0]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("WPS_E_HISTORY_DUPLICATE")));
    }

    #[test]
    fn restart_after_lost_response_does_not_replay_and_recovery_needs_evidence() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let transport = MockTransport::new("session-1", vec![Err(WpsError::Timeout)]);
        let client = WpsClient::new(transport, "file-1");
        let sync = W2Sync::new(&client, &store);
        let original = request("AR-LOST", "2026-11-01");
        sync.execute(&W2Config::mock_write(), std::slice::from_ref(&original))
            .expect("first execute");
        let intent_id: i64 = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT id FROM wps_write_intents WHERE idempotency_key = ?1",
                    params!["idem-AR-LOST"],
                    |row| row.get(0),
                )
                .map_err(Into::into)
            })
            .expect("intent");

        let transport2 = MockTransport::new("session-2", vec![]);
        let client2 = WpsClient::new(transport2, "file-1");
        let sync2 = W2Sync::new(&client2, &store);
        let err = sync2
            .preflight(&W2Config::mock_write(), std::slice::from_ref(&original))
            .expect_err("must reject replay");
        assert_eq!(err.code, "WPS_E_IDEMPOTENCY_STATE");
        assert!(client2.transport().calls().is_empty());

        assert!(sync2
            .recover_link(intent_id, "R-REMOTE", "remote id + 归属证据")
            .is_ok());
        assert_eq!(
            sync2.status(3, "R-REMOTE").expect("status"),
            W2SyncStatus::Displayed
        );
        assert!(client2.transport().calls().is_empty());
    }

    #[test]
    fn execute_stops_on_timeout_and_marks_next_not_attempted() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let transport = MockTransport::new("session-1", vec![Err(WpsError::Timeout)]);
        let client = WpsClient::new(transport, "file-1");
        let sync = W2Sync::new(&client, &store);
        let second = request("AR-2", "2026-11-01");
        let report = sync
            .execute(
                &W2Config::mock_write(),
                &[request("AR-1", "2026-11-01"), second],
            )
            .expect("execute");
        assert!(report.stopped);
        assert_eq!(report.outcomes[0].status, W2WriteStatus::Unknown);
        assert_eq!(report.outcomes[1].status, W2WriteStatus::NotAttempted);
        assert_eq!(client.transport().calls().len(), 1);
        assert_eq!(
            sync.intent_state("idem-AR-1").expect("intent"),
            Some("unknown".into())
        );
    }

    #[test]
    fn execute_stops_on_link_backfill_failure() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        db.with_conn(|conn| {
            conn.execute_batch("DROP TABLE wps_links")?;
            Ok(())
        })
        .expect("drop links");
        let transport =
            MockTransport::new("session-1", vec![Ok(json!({"records": [{"id": "R-NEW"}]}))]);
        let client = WpsClient::new(transport, "file-1");
        let sync = W2Sync::new(&client, &store);
        let report = sync
            .execute(&W2Config::mock_write(), &[request("AR-NEW", "2026-11-01")])
            .expect("execute");
        assert!(report.stopped);
        assert_eq!(report.outcomes[0].status, W2WriteStatus::PendingBackfill);
        assert_eq!(
            sync.intent_state("idem-AR-NEW").expect("intent"),
            Some("pending_backfill".into())
        );
    }

    #[test]
    fn recover_link_requires_evidence_and_never_replays_create() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let transport = MockTransport::new("session-1", vec![Err(WpsError::Timeout)]);
        let client = WpsClient::new(transport, "file-1");
        let sync = W2Sync::new(&client, &store);
        sync.execute(&W2Config::mock_write(), &[request("AR-NEW", "2026-11-01")])
            .expect("execute");
        let intent_id: i64 = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT id FROM wps_write_intents WHERE idempotency_key = ?1",
                    params!["idem-AR-NEW"],
                    |row| row.get(0),
                )
                .map_err(Into::into)
            })
            .expect("intent id");
        assert!(sync.recover_link(intent_id, "R-NEW", "  ").is_err());
        assert_eq!(client.transport().calls().len(), 1);
        sync.recover_link(intent_id, "R-NEW", "远端响应与请求哈希一致")
            .expect("recover");
        assert_eq!(client.transport().calls().len(), 1);
    }
}
