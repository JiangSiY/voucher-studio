//! 工作单元（对应 v4.3 的 `db/unit_of_work.py`）
//!
//! 单连接 + 互斥锁：桌面应用足够；显式事务保证「凭证+附件+快照」
//! 这类多表写入要么全成、要么全不动。

use std::sync::Mutex;

use rusqlite::{Connection, Transaction};

use crate::Result;

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
        }
    }

    /// 只读或单语句操作
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self.conn.lock().unwrap();
        f(&guard)
    }

    /// 显式事务：闭包返回 Err 时整体回滚
    pub fn with_tx<T>(&self, f: impl FnOnce(&Transaction) -> Result<T>) -> Result<T> {
        let mut guard = self.conn.lock().unwrap();
        let tx = guard.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }
}
