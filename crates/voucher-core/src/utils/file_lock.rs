//! 文件锁（对应 v4.3 的 `file_lock.py`）
//!
//! 语义：同一数据目录同一时刻只允许一个实例写入。
//! 实现：POSIX `flock` 咨询锁——被其他实例持有时返回 `EWOULDBLOCK` → `LockHeld`；
//! 进程退出后锁自动释放，强杀残留的锁文件下次启动按「陈锁」接管（语义与旧实现一致）。
//! Tauri 层另有单实例插件，这里做第二道防线。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{CoreError, Result};

pub struct FileLockGuard {
    file: Option<File>,
    path: PathBuf,
}

impl FileLockGuard {
    /// 尝试获取锁；被其他实例持有时返回 [`CoreError::LockHeld`]
    pub fn try_acquire(path: &Path) -> Result<Self> {
        // Unix 无强制锁（文件存在也总能 open），互斥完全靠 flock：
        // 被持有时返回 EWOULDBLOCK → LockHeld；陈锁（无进程持有）flock 直接成功 → 接管。
        use std::os::unix::io::AsRawFd;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .map_err(|_| CoreError::LockHeld)?;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            return Err(CoreError::LockHeld);
        }
        let _ = writeln!(file, "{}", std::process::id());
        Ok(Self {
            file: Some(file),
            path: path.to_path_buf(),
        })
    }
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        // 先关句柄（释放 flock）再删文件
        self.file = None;
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_acquire_and_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join(".lock");

        let guard = FileLockGuard::try_acquire(&lock_path).unwrap();
        // 同一路径二次获取：句柄独占 → LockHeld
        assert!(matches!(
            FileLockGuard::try_acquire(&lock_path),
            Err(CoreError::LockHeld)
        ));

        drop(guard);
        // 释放后可重新获取
        assert!(FileLockGuard::try_acquire(&lock_path).is_ok());
    }

    /// 崩溃残留的陈锁（无进程持有句柄的普通文件）应能被直接接管，而非误报 LockHeld
    #[test]
    fn stale_lock_file_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join(".lock");
        std::fs::write(&lock_path, "34468").unwrap(); // 模拟强杀进程后的残留

        let guard = FileLockGuard::try_acquire(&lock_path).expect("陈锁应被接管而非报 LockHeld");
        drop(guard);
        assert!(!lock_path.exists(), "接管释放后锁文件应被清理");
    }
}
