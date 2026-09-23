//! GFS 分层保留（日/周/月）+ 孤儿附件包回收：本地 backups/ 与异盘目录共用同一套语义。

use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate};

use super::{read_manifest, BUNDLE_PREFIX, SNAPSHOT_PREFIX};
use crate::utils::config::BackupConfig;
use crate::Result;
// ---------- GFS 分层保留（日/周/月） ----------

/// 从备份文件名解析日期（voucher-backup-YYYYMMDD-HHMMSS.zip）
fn date_of_file_name(name: &str) -> Option<NaiveDate> {
    let digits: String = name
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take(8)
        .collect();
    NaiveDate::parse_from_str(&digits, "%Y%m%d").ok()
}

/// GFS 选择（纯函数，便于测试）：输入按新→旧排序的备份日期，
/// 返回逐项「是否保留」标记。基准取「最新备份」而非今天——停机多日仍能正确分层。
fn gfs_keep(
    dates: &[NaiveDate],
    keep_daily: u32,
    keep_weekly: u32,
    keep_monthly: u32,
) -> Vec<bool> {
    if dates.is_empty() {
        return Vec::new();
    }
    let mut keep = vec![true; dates.len()];
    let newest = dates[0];

    // 周序号 / 月序号（ISO 周；月用 (年, 月)）
    let week_of = |d: NaiveDate| d.iso_week().week() as i32 + d.iso_week().year() * 100;
    let month_of = |d: NaiveDate| d.month() as i32 + d.year() * 12;

    let mut weeks_seen: Vec<i32> = Vec::new();
    let mut months_seen: Vec<i32> = Vec::new();
    let mut weekly_left = keep_weekly;
    let mut monthly_left = keep_monthly;

    for (i, d) in dates.iter().enumerate() {
        let age_days = (newest - *d).num_days();
        if keep_daily > 0 && age_days < keep_daily as i64 {
            continue; // 日备窗口内：默认保留
        }
        let w = week_of(*d);
        let m = month_of(*d);
        let in_weekly = weeks_seen.contains(&w);
        let in_monthly = months_seen.contains(&m);
        if weekly_left > 0 && !in_weekly {
            // 该周第一份（最新）→ 周备
            weeks_seen.push(w);
            months_seen.push(m);
            weekly_left -= 1;
            continue;
        }
        if monthly_left > 0 && !in_monthly {
            // 该月第一份（最新）→ 月备
            months_seen.push(m);
            monthly_left -= 1;
            continue;
        }
        keep[i] = false;
    }
    keep
}

/// GFS + 孤儿附件包回收（文件系统版）：本地 backups/ 与异盘目录共用同一语义——
/// 快照+全量合并序列按 GFS 保留；日期解析失败的文件保留且计入 GC 引用；
/// 保留快照引用的附件包留下，其余回收。
///
/// 这一语义曾在本地（enforce_retention）与异盘（sync_backup_set_to_secondary）
/// 各写一遍并开始分叉；统一收口到这里，任何保留策略调整只改一处。
pub(super) fn fs_retention(dir: &Path, cfg: &BackupConfig) -> Result<()> {
    // 扫描可恢复备份（快照 + 全量；附件包不进序列）
    let mut dated: Vec<(NaiveDate, PathBuf)> = Vec::new();
    let mut kept: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let is_backup = (name.starts_with("voucher-backup-") || name.starts_with(SNAPSHOT_PREFIX))
            && name.ends_with(".zip");
        if !is_backup {
            continue;
        }
        match date_of_file_name(name) {
            Some(d) => dated.push((d, path)),
            None => kept.push(path), // 解析不出日期（外部拷入等）：保留且计入引用
        }
    }
    dated.sort_by_key(|(d, _)| std::cmp::Reverse(*d)); // 新→旧

    let keep_flags = gfs_keep(
        &dated.iter().map(|(d, _)| *d).collect::<Vec<_>>(),
        cfg.keep_daily,
        cfg.keep_weekly,
        cfg.keep_monthly,
    );
    for (i, keep) in keep_flags.iter().enumerate() {
        if *keep {
            kept.push(dated[i].1.clone());
        } else {
            let _ = std::fs::remove_file(&dated[i].1);
        }
    }
    gc_orphan_bundles(dir, &kept)?;
    Ok(())
}

/// 回收孤儿附件包：`kept` 为保留的备份文件路径（快照读 manifest 引用，全量自含附件）。
/// 目录内未被引用的 voucher-attachments-*.zip 一律删除。
///
/// **fail-closed（2026-09-23 审计 B2）**：只要有**任何**保留中的备份读不出 manifest
/// （age 加密备份改了/清了口令、文件损坏等），就无法判断附件包的引用关系，此时**整轮跳过**
/// 回收。旧实现是 fail-open——读不出即视为「无引用」，于是「改加密口令 → 下次备份」会把
/// 旧快照引用的附件包当孤儿删掉，那些快照从此永久不可恢复，而界面仍报「备份成功」。
fn gc_orphan_bundles(dir: &Path, kept: &[PathBuf]) -> Result<()> {
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut unreadable: Vec<&PathBuf> = Vec::new();
    for p in kept {
        match read_manifest(p) {
            Some(m) => {
                if let Some(n) = m.attachments_bundle {
                    referenced.insert(n);
                }
            }
            None => unreadable.push(p),
        }
    }
    if !unreadable.is_empty() {
        log::warn!(
            "跳过孤儿附件包回收：{} 份保留中的备份读不出 manifest（口令变更或文件损坏？），\
             无法判断引用关系，宁可不删。示例：{}",
            unreadable.len(),
            unreadable[0].display()
        );
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with(BUNDLE_PREFIX) && name.ends_with(".zip") && !referenced.contains(name) {
            let _ = std::fs::remove_file(&path);
            log::info!("回收孤儿附件包：{name}");
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// 2026-09-23 缺陷锁（审计 B2）：保留中的备份读不出 manifest 时（改/清加密口令、
    /// 文件损坏），**不得**把附件包当孤儿删掉——旧实现 fail-open，会让那些快照永久不可恢复。
    #[test]
    fn unreadable_manifest_makes_gc_skip_bundles() {
        let dir = std::env::temp_dir().join(format!("vs-retention-gc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 「保留中」但读不出 manifest 的备份（内容不是 zip）
        let snap = dir.join(format!("{SNAPSHOT_PREFIX}20260923-000000.zip"));
        std::fs::write(&snap, b"not-a-zip").unwrap();
        // 它的附件包：旧实现会把它当孤儿删掉
        let bundle = dir.join(format!("{BUNDLE_PREFIX}deadbeef.zip"));
        std::fs::write(&bundle, b"bundle").unwrap();

        gc_orphan_bundles(&dir, std::slice::from_ref(&snap)).unwrap();

        assert!(
            bundle.is_file(),
            "manifest 读不出时必须整轮跳过回收，不得删附件包"
        );
        assert!(snap.is_file(), "保留中的备份本身当然也不能删");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gfs_keeps_daily_window_weekly_and_monthly() {
        // 2026-08-03/10/17 均为周一；同周配对：08-10+08-11、08-03+08-04、07-14+07-15
        let dates = vec![
            d("2026-08-18"), // 最新（日备窗口内）
            d("2026-08-17"), // 日备窗口内
            d("2026-08-11"), // 周备 #1（第 33 周）
            d("2026-08-10"), // 同周更旧 → 删
            d("2026-08-04"), // 周备 #2（第 32 周）
            d("2026-08-03"), // 同周更旧 → 删
            d("2026-07-15"), // 周备 #3（第 29 周）
            d("2026-07-14"), // 同周更旧 → 删
            d("2026-06-10"), // 月备 #1（6 月）
        ];
        let keep = gfs_keep(&dates, 2, 3, 2);
        assert_eq!(
            keep,
            vec![true, true, true, false, true, false, true, false, true]
        );
    }

    #[test]
    fn gfs_zero_daily_zero_weekly() {
        // 只留月备：每月最新一份，共 2 份
        let dates = vec![
            d("2026-08-18"), // 月备 #1（8 月）
            d("2026-07-04"), // 月备 #2（7 月）
            d("2026-06-02"), // 超出 → 删
            d("2026-05-03"), // 超出 → 删
        ];
        let keep = gfs_keep(&dates, 0, 0, 2);
        assert_eq!(keep, vec![true, true, false, false]);
    }

    #[test]
    fn gfs_empty() {
        assert!(gfs_keep(&[], 7, 4, 6).is_empty());
    }

    #[test]
    fn date_parsing_from_file_name() {
        assert_eq!(
            date_of_file_name("voucher-backup-20260818-093000.zip"),
            Some(d("2026-08-18"))
        );
        assert_eq!(date_of_file_name("random.zip"), None);
    }
}
