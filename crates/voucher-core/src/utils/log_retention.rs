//! 本地日志保留策略（S0-11）：只删除普通文件，不跟随符号链接。
//!
//! `tauri-plugin-log` 只按大小轮转；30 天清理由应用启动时执行一次。

use std::path::Path;
use std::time::{Duration, SystemTime};

/// 删除 `dir` 下修改时间早于 `days` 天的普通文件；返回删除文件数。
pub fn cleanup_older_than(dir: &Path, days: u64) -> std::io::Result<usize> {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(days.saturating_mul(24 * 60 * 60)))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut removed = 0usize;
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if !metadata.file_type().is_file() {
            continue;
        }
        let modified = metadata.modified()?;
        if modified < cutoff {
            std::fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "voucher-log-retention-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create dir");
        dir
    }

    #[test]
    fn removes_old_files_and_keeps_recent_and_directories() {
        let dir = temp_dir();
        let old = dir.join("old.log");
        let recent = dir.join("recent.log");
        fs::write(&old, b"old").expect("write old");
        fs::write(&recent, b"new").expect("write recent");
        let file = fs::OpenOptions::new()
            .write(true)
            .open(&old)
            .expect("open old");
        file.set_modified(SystemTime::now() - Duration::from_secs(31 * 24 * 60 * 60))
            .expect("set mtime");
        fs::create_dir(dir.join("nested")).expect("create nested");

        let removed = cleanup_older_than(&dir, 30).expect("cleanup");
        assert_eq!(removed, 1);
        assert!(!old.exists());
        assert!(recent.exists());
        assert!(dir.join("nested").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
