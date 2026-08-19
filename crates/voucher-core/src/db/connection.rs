//! 连接管理（对应 v4.3 的 `db/connection.py`）

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::Result;

/// 打开数据库并应用推荐 PRAGMA：
/// - WAL：读写并发更好，崩溃恢复能力强
/// - foreign_keys：附件/明细随主表级联删除
/// - busy_timeout：避开偶发锁冲突
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;",
    )?;
    Ok(conn)
}
