//! 定时调度（对应 v4.3 的 `scheduler.py`）
//!
//! 纯计算：判断到期 + 产出下次运行时间；实际定时器由 Tauri 层的 tokio 任务驱动，
//! 这样核心逻辑可以独立测试。

use chrono::{DateTime, Duration, Utc};

use crate::services::backup::KEY_LAST_BACKUP;
use crate::{CoreContext, Result};

/// 上次运行时间（存 config 表的 RFC3339）是否已到期
pub fn is_due(last_run: Option<&str>, interval_hours: u64) -> bool {
    match last_run {
        None => true,
        Some(s) => match DateTime::parse_from_rfc3339(s) {
            Ok(t) => Utc::now() - t.with_timezone(&Utc) >= Duration::hours(interval_hours as i64),
            Err(_) => true, // 解析不了视为从没跑过
        },
    }
}

/// 是否应该自动备份（开关 + 到期）
pub fn backup_due(ctx: &CoreContext) -> Result<bool> {
    let cfg = ctx.config();
    if !cfg.backup.auto_enabled {
        return Ok(false);
    }
    let last = ctx
        .db
        .with_conn(|conn| crate::db::repos::config_repo::get(conn, KEY_LAST_BACKUP))?;
    Ok(is_due(last.as_deref(), cfg.backup.interval_hours))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_logic() {
        assert!(is_due(None, 24));
        let fresh = (Utc::now() - Duration::hours(1)).to_rfc3339();
        assert!(!is_due(Some(&fresh), 24));
        let stale = (Utc::now() - Duration::hours(25)).to_rfc3339();
        assert!(is_due(Some(&stale), 24));
        assert!(is_due(Some("not-a-date"), 24));
    }
}
