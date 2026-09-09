//! 工作单元（对应 v4.3 的 `db/unit_of_work.py`）
//!
//! 单连接 + 互斥锁：桌面应用足够；显式事务保证「凭证+附件+快照」
//! 这类多表写入要么全成、要么全不动。
//!
//! 并发约定（体检 Q-2/Q-31 修复）：
//! - `with_conn` / `with_tx` 的闭包内**禁止再次调用**二者：本锁是 std Mutex、非重入，
//!   再取一次锁会直接死锁且编译器不报错。代码用线程本地重入护栏在进入闭包前拦截
//!   **同线程**重入并返回 `CoreError::Invalid`。
//! - 护栏基于 `thread_local`，跨线程互不可见：若在闭包内派生线程（`spawn` /
//!   `spawn_blocking` / `par_iter`）后再取连接，仍可能死锁，须在代码约定层面禁止。
//! - 锁被上一持锁线程 panic 污染（poisoned）时，用 `into_inner()` 恢复继续使用并记录日志，
//!   不静默恢复（连接可能残留未提交事务）。

use std::cell::Cell;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, Transaction};

use crate::{CoreError, Result};

// 当前线程是否已持有连接锁（检测同线程重入）。
thread_local! {
    static HOLDING: Cell<bool> = const { Cell::new(false) };
}

/// RAII 重入护栏：进入临界区置位，`Drop` 复位。
///
/// 必须用 RAII 而非在函数尾 `set(false)`：`with_tx` 内有三个 `?` 早退点
/// （`transaction()` / 闭包 / `commit()`），尾部复位会被跳过导致 `HOLDING` 永久为 true，
/// 此后该线程每次取连接都误报「重入」——护栏反杀正常路径。
/// 成功返回 / `Err` 早退 / panic 展开三条路径都由 `Drop` 覆盖。
struct ReentryGuard;

impl Drop for ReentryGuard {
    fn drop(&mut self) {
        HOLDING.set(false);
    }
}

/// 进入临界区。已持锁 → `Err`；**此时不创建守卫**，标志位由外层持有者负责复位。
/// （若在此误建守卫，内层 `Drop` 会清掉外层还持有的标志，护栏当场开洞。）
#[track_caller]
fn enter() -> Result<ReentryGuard> {
    if HOLDING.replace(true) {
        log::error!(
            "检测到 Database 连接重入：闭包内禁止再次调用 with_conn/with_tx（std Mutex 非重入，会死锁）"
        );
        return Err(CoreError::Invalid("内部错误：数据库连接重入".into()));
    }
    Ok(ReentryGuard)
}

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
        }
    }

    /// 取连接锁；中毒（上一持锁线程 panic）时恢复继续使用并记录日志。
    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        match self.conn.lock() {
            Ok(guard) => Ok(guard),
            Err(poisoned) => {
                log::error!(
                    "数据库连接锁中毒：上一持锁线程 panic，已恢复继续使用（连接可能残留未提交事务）"
                );
                Ok(poisoned.into_inner())
            }
        }
    }

    /// 只读或单语句操作。
    ///
    /// # 注意
    /// 闭包内**禁止再次调用** `with_conn` / `with_tx`（std Mutex 非重入，再次取锁会死锁），
    /// 也禁止派生线程/并行迭代后再取连接（跨线程死锁，线程本地护栏无法检出）。
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let _guard = enter()?;
        let conn = self.lock()?;
        f(&conn)
    }

    /// 显式事务：闭包返回 Err 时整体回滚。
    ///
    /// # 注意
    /// 同 [`Database::with_conn`]：闭包内禁止再次调用二者，禁止派生线程后再取连接。
    pub fn with_tx<T>(&self, f: impl FnOnce(&Transaction) -> Result<T>) -> Result<T> {
        let _guard = enter()?;
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// 同线程闭包内再次取连接：应返回「重入」错误而非死锁。
    #[test]
    fn nested_with_conn_returns_reentry_error_not_deadlock() {
        let db = Database::new(Connection::open_in_memory().unwrap());
        let outer: Result<()> = db.with_conn(|_| db.with_conn(|_| Ok(())));
        assert!(matches!(outer, Err(CoreError::Invalid(msg)) if msg.contains("重入")));
    }

    /// 锁中毒恢复：手动让持锁线程 panic 后，调用路径仍可用、日志已记录。
    #[test]
    fn poisoned_conn_lock_recovers_and_stays_usable() {
        let db = Database::new(Connection::open_in_memory().unwrap());
        std::thread::scope(|s| {
            let handle = s.spawn(|| {
                let _guard = db.conn.lock().unwrap();
                panic!("模拟持锁线程 panic");
            });
            // 显式 join 捕获 panic，scope 结束时不会再次展开
            assert!(handle.join().is_err());
        });
        // 锁已中毒：with_conn 内部应恢复并仍可用
        let result = db.with_conn(|conn| {
            conn.execute_batch("CREATE TABLE t (id INTEGER)")
                .map_err(CoreError::Db)
        });
        assert!(result.is_ok());
    }
}
