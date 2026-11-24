//! WPS 同步状态（只读）：最近同步/扫描时刻 + 待处理队列。
//!
//! 三条硬约束（对应 t7 契约）：
//! - **只读本地 SQLite**：不构造 WPS 客户端、不读凭据、不发任何网络请求；
//! - **不回显业务数据**：待处理项只给 `sheet_id` / `idempotency_key` / 错误码 / 状态，
//!   字段值与远端 payload 一律不返回（前端空态与红点只需要计数与码）；
//! - **时间一律 `…Z`**：库里既有 SQLite `datetime('now')`（`YYYY-MM-DD HH:MM:SS`，UTC），
//!   也有历史 `to_rfc3339()` 写的 `+00:00`，统一归一为 RFC3339 UTC（与 `wpsTime.ts` 同规则）。

use serde::Serialize;

use voucher_core::CoreContext;

use crate::ApiResult;

/// 待处理队列上限：超出部分只看 `pending_count` / `unknown_count`，避免拖慢界面。
const PENDING_LIMIT: i64 = 20;

/// 结果不明且没有错误文本时的兜底码（不新造错误码，仅占位展示）。
const UNKNOWN_CODE: &str = "WPS_E_UNKNOWN";

/// 待处理队列的一条：只含码与状态，不含字段值/远端 payload。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WpsPendingItem {
    pub sheet_id: i64,
    pub idempotency_key: String,
    /// 稳定错误码（`WPS_E_*` 前缀）。
    pub code: String,
    /// 本地状态：`pending_backfill`（远端已成功、回填未完成）或 `unknown`（结果不明）。
    pub state: String,
}

/// 同步状态总览（D1–D3 空态的真实数据来源）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WpsSyncStatus {
    /// `wps_sync_runs` 最近一次 `status='ok'` 的完成时刻；没有记录则 `None`。
    pub last_sync_at: Option<String>,
    /// `wps_scans` 最近一次 `complete=1` 的发布时刻；没有记录则 `None`。
    pub last_scan_complete: Option<String>,
    /// `wps_links.state='pending_backfill'` 的条数。
    pub pending_count: i64,
    /// `wps_write_intents.state='unknown'`（结果不明）的条数。
    pub unknown_count: i64,
    /// 最多 20 条待处理项，按更新时间倒序。
    pub pending_items: Vec<WpsPendingItem>,
}

impl WpsSyncStatus {
    /// 空态：与空库调用结果一致，供前端在没有数据源时复用同一形状。
    pub fn empty() -> Self {
        Self {
            last_sync_at: None,
            last_scan_complete: None,
            pending_count: 0,
            unknown_count: 0,
            pending_items: Vec::new(),
        }
    }
}

/// 读取同步状态。参数只有 `ctx`：命令体不接受 file_id / 数据目录，目标永远是本机库。
pub fn wps_sync_status(ctx: &CoreContext) -> ApiResult<WpsSyncStatus> {
    let status = ctx.db.with_conn(|conn| {
        // 最近一次成功运行的完成时刻（无行 → None；query_row 的 no-rows 分支在这里不需要）。
        let mut runs = conn.prepare(
            "SELECT finished_at FROM wps_sync_runs
              WHERE status = 'ok' AND finished_at IS NOT NULL
              ORDER BY finished_at DESC, id DESC LIMIT 1",
        )?;
        let mut rows = runs.query(())?;
        let last_sync_at = match rows.next()? {
            Some(row) => utc_z(&row.get::<_, String>(0)?),
            None => None,
        };

        // 最近一次「完整」扫描的发布时刻（半截扫描不算）。
        let mut scans = conn.prepare(
            "SELECT finished_at FROM wps_scans
              WHERE complete = 1 AND finished_at IS NOT NULL
              ORDER BY finished_at DESC, id DESC LIMIT 1",
        )?;
        let mut rows = scans.query(())?;
        let last_scan_complete = match rows.next()? {
            Some(row) => utc_z(&row.get::<_, String>(0)?),
            None => None,
        };

        let pending_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM wps_links WHERE state = 'pending_backfill'",
            (),
            |row| row.get(0),
        )?;
        let unknown_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM wps_write_intents WHERE state = 'unknown'",
            (),
            |row| row.get(0),
        )?;

        // 两条来源合并排序后截断：链接回填失败 + 写入结果不明。
        let mut items: Vec<(String, i64, WpsPendingItem)> = Vec::new();

        let mut links = conn.prepare(
            "SELECT id, sheet_id, idempotency_key, updated_at
               FROM wps_links WHERE state = 'pending_backfill'
               ORDER BY updated_at DESC, id DESC LIMIT ?1",
        )?;
        let mut cursor = links.query([PENDING_LIMIT])?;
        while let Some(row) = cursor.next()? {
            items.push((
                row.get::<_, String>(3)?,
                row.get::<_, i64>(0)?,
                WpsPendingItem {
                    sheet_id: row.get(1)?,
                    idempotency_key: row.get(2)?,
                    code: "WPS_E_LINK_BACKFILL".to_string(),
                    state: "pending_backfill".to_string(),
                },
            ));
        }

        let mut intents = conn.prepare(
            "SELECT id, sheet_id, idempotency_key, state, error, updated_at
               FROM wps_write_intents WHERE state = 'unknown'
               ORDER BY updated_at DESC, id DESC LIMIT ?1",
        )?;
        let mut cursor = intents.query([PENDING_LIMIT])?;
        while let Some(row) = cursor.next()? {
            let error: Option<String> = row.get(4)?;
            items.push((
                row.get::<_, String>(5)?,
                row.get::<_, i64>(0)?,
                WpsPendingItem {
                    sheet_id: row.get(1)?,
                    idempotency_key: row.get(2)?,
                    code: error_code(error.as_deref()),
                    state: row.get(3)?,
                },
            ));
        }

        items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        items.truncate(PENDING_LIMIT as usize);

        Ok(WpsSyncStatus {
            last_sync_at,
            last_scan_complete,
            pending_count,
            unknown_count,
            pending_items: items.into_iter().map(|(_, _, item)| item).collect(),
        })
    })?;
    Ok(status)
}

/// 从 `error` 文本取稳定错误码：`WPS_E_XXX: 说明` → `WPS_E_XXX`；取不到则兜底码。
fn error_code(error: Option<&str>) -> String {
    let Some(raw) = error else {
        return UNKNOWN_CODE.to_string();
    };
    let head = raw.split(':').next().unwrap_or("").trim();
    if head.starts_with("WPS_E_") {
        head.to_string()
    } else {
        UNKNOWN_CODE.to_string()
    }
}

/// 归一为 RFC3339 UTC（`…Z`）。
///
/// 覆盖库里真实存在的两种形态：SQLite `datetime('now')` 的 `YYYY-MM-DD HH:MM:SS`（UTC）
/// 与历史 `to_rfc3339()` 的 `…+00:00`；其它带非零偏移的值不属于本库产出，按未知处理（`None`），
/// 避免把本地时间当 UTC 展示。
fn utc_z(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(body) = raw.strip_suffix('Z').or_else(|| raw.strip_suffix('z')) {
        return Some(format!("{}Z", body.replace(' ', "T")));
    }
    if let Some(body) = raw.strip_suffix("+00:00") {
        return Some(format!("{}Z", body.replace(' ', "T")));
    }
    let body = raw.replace(' ', "T");
    let bytes = body.as_bytes();
    // `YYYY-MM-DDTHH:MM:SS`：无时区后缀即 SQLite 的 UTC。
    if bytes.len() == 19 && bytes[10] == b'T' && bytes[13] == b':' && bytes[16] == b':' {
        return Some(format!("{body}Z"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_z_accepts_all_local_storage_shapes() {
        assert_eq!(
            utc_z("2026-11-24 05:03:00").as_deref(),
            Some("2026-11-24T05:03:00Z")
        );
        assert_eq!(
            utc_z("2026-11-24T05:03:00+00:00").as_deref(),
            Some("2026-11-24T05:03:00Z")
        );
        assert_eq!(
            utc_z("2026-11-24T05:03:00Z").as_deref(),
            Some("2026-11-24T05:03:00Z")
        );
        assert_eq!(utc_z("  ").as_deref(), None);
        assert_eq!(utc_z("2026-11-24T05:03:00+08:00").as_deref(), None);
    }

    #[test]
    fn error_code_keeps_only_the_stable_prefix() {
        assert_eq!(error_code(Some("WPS_E_TIMEOUT: 请求超时")), "WPS_E_TIMEOUT");
        assert_eq!(error_code(Some("超时")), UNKNOWN_CODE);
        assert_eq!(error_code(None), UNKNOWN_CODE);
    }

    /// 静态判据：本模块不得出现客户端/网络符号（只读本地库）。
    #[test]
    fn source_never_builds_a_wps_client() {
        let src = include_str!("wps_status.rs");
        for needle in [
            ["Wps", "Client"].concat(),
            ["Wps", "Transport"].concat(),
            ["create", "_record("].concat(),
            ["list", "_records("].concat(),
            ["reqwest", "::"].concat(),
        ] {
            assert!(!src.contains(&needle), "wps_status 不得出现 {needle}");
        }
    }
}
