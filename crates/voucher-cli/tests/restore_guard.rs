//! `restore --target` 的覆盖守卫：判不过一个字都不写；放行时连 WAL 一起移入备份。
//!
//! 断言全部落在文件系统上（len / mtime / 目录项），而不是只看退出码——
//! 守卫若排错了位置（先 `CoreContext::init` 再判），退出码仍然可能是 2，
//! 但生产库已经被建库+迁移改写过了。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::SystemTime;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_voucher-cli")
}

fn unique_root(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "voucher-cli-restore-{}-{}-{tag}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

fn run(args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .output()
        .expect("运行 voucher-cli")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// 造一个「既有库」：目录 + voucher.db + 迁移。
fn seed_db(base: &Path) {
    std::fs::create_dir_all(base).expect("建目录");
    let conn = voucher_core::db::connection::open(&base.join("voucher.db")).expect("建库");
    voucher_core::db::schema::init(&conn).expect("建表");
}

/// 用真实备份链路造一份可被 `restore` 接受的 zip，避免手搓 zip 布局。
fn make_backup(dir: &Path) -> PathBuf {
    let src = dir.join("source");
    std::fs::create_dir_all(&src).expect("建源目录");
    let ctx = voucher_core::CoreContext::init(Some(src)).expect("源 ctx");
    let info = voucher_core::services::backup::run_backup(&ctx).expect("备份");
    PathBuf::from(info.path)
}

/// (文件名, len, mtime)：用来证明「一个字都没写」。
fn snapshot(dir: &Path) -> Vec<(String, u64, Option<SystemTime>)> {
    let mut rows: Vec<(String, u64, Option<SystemTime>)> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| {
                    let path = entry.path();
                    let meta = entry.metadata().ok();
                    (
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or_default()
                            .to_string(),
                        meta.as_ref().map(|meta| meta.len()).unwrap_or_default(),
                        meta.and_then(|meta| meta.modified().ok()),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

fn pre_restore_dirs(target: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(target.join("backups"))
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("pre-restore-"))
                })
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

#[test]
fn existing_database_without_flag_is_refused_and_left_untouched() {
    let root = unique_root("no-flag");
    let target = root.join("target");
    seed_db(&target);
    std::fs::write(target.join("voucher.db-wal"), b"wal-bytes").expect("写 wal");
    let before = snapshot(&target);
    let zip = make_backup(&root);

    let out = run(&[
        "restore",
        "--zip",
        zip.to_str().unwrap(),
        "--target",
        target.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2), "stderr：{}", stderr(&out));
    assert!(
        stderr(&out).contains("恢复会覆盖该库"),
        "stderr：{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("--allow-overwrite"),
        "stderr：{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains(&target.join("voucher.db").display().to_string()),
        "stderr：{}",
        stderr(&out)
    );
    assert_eq!(snapshot(&target), before, "拒绝分支不得改动目标目录");
    assert!(!target.join("backups").exists(), "拒绝分支不得建 backups/");
}

#[test]
fn flag_moves_the_old_database_and_wal_into_pre_restore_backup() {
    let root = unique_root("flag");
    let target = root.join("target");
    seed_db(&target);
    std::fs::write(target.join("voucher.db-wal"), b"wal-bytes").expect("写 wal");
    let original_db = std::fs::read(target.join("voucher.db")).expect("读库");
    let original_wal = std::fs::read(target.join("voucher.db-wal")).expect("读 wal");
    let zip = make_backup(&root);

    let out = run(&[
        "restore",
        "--zip",
        zip.to_str().unwrap(),
        "--target",
        target.to_str().unwrap(),
        "--allow-overwrite",
    ]);
    assert_eq!(out.status.code(), Some(0), "stderr：{}", stderr(&out));
    let dirs = pre_restore_dirs(&target);
    assert_eq!(dirs.len(), 1, "放行后应恰好一份覆盖前备份：{dirs:?}");
    let backup = &dirs[0];
    assert!(
        stdout(&out).contains(&backup.display().to_string()),
        "stdout 应输出副本路径：{}",
        stdout(&out)
    );
    assert_eq!(
        std::fs::read(backup.join("voucher.db")).expect("备份里的库"),
        original_db,
        "备份里的库必须逐字节等于被覆盖前的库"
    );
    assert_eq!(
        std::fs::read(backup.join("voucher.db-wal")).expect("备份里的 wal"),
        original_wal,
        "WAL 必须一起进备份，不能被留在原地"
    );
    assert!(
        !target.join("voucher.db-wal").is_file(),
        "旧 WAL 不得留在目标目录（会被回放到新库）"
    );
    assert!(stdout(&out).contains("已还原"), "stdout：{}", stdout(&out));
    assert!(target.join("voucher.db").is_file(), "还原后的库应存在");
}

#[test]
fn fresh_target_keeps_the_old_behavior() {
    let root = unique_root("fresh");
    let target = root.join("target");
    let zip = make_backup(&root);

    let out = run(&[
        "restore",
        "--zip",
        zip.to_str().unwrap(),
        "--target",
        target.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "stderr：{}", stderr(&out));
    assert!(!stdout(&out).contains("覆盖前副本"), "新目录不该有副本提示");
    assert!(
        pre_restore_dirs(&target).is_empty(),
        "新目录不该有 pre-restore 备份"
    );
    assert!(target.join("voucher.db").is_file());
}

#[test]
fn empty_target_directory_also_keeps_the_old_behavior() {
    let root = unique_root("empty");
    let target = root.join("target");
    std::fs::create_dir_all(&target).expect("建空目录");
    let zip = make_backup(&root);

    let out = run(&[
        "restore",
        "--zip",
        zip.to_str().unwrap(),
        "--target",
        target.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "stderr：{}", stderr(&out));
    assert!(pre_restore_dirs(&target).is_empty());
    assert!(target.join("voucher.db").is_file());
}

#[test]
fn zero_byte_database_is_treated_as_new() {
    let root = unique_root("zero");
    let target = root.join("target");
    std::fs::create_dir_all(&target).expect("建目录");
    std::fs::write(target.join("voucher.db"), b"").expect("写空库");
    let zip = make_backup(&root);

    let out = run(&[
        "restore",
        "--zip",
        zip.to_str().unwrap(),
        "--target",
        target.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "stderr：{}", stderr(&out));
    assert!(pre_restore_dirs(&target).is_empty(), "0 字节库不算覆盖");
    assert!(target.join("voucher.db").metadata().unwrap().len() > 0);
}

#[test]
fn existing_pre_restore_backup_also_requires_the_flag() {
    let root = unique_root("pre-existing");
    let target = root.join("target");
    let backup = target.join("backups").join("pre-restore-20260101T000000Z");
    std::fs::create_dir_all(&backup).expect("建旧备份目录");
    std::fs::write(backup.join("voucher.db"), b"old-db").expect("写旧副本");
    let zip = make_backup(&root);
    let before = snapshot(&backup);

    let out = run(&[
        "restore",
        "--zip",
        zip.to_str().unwrap(),
        "--target",
        target.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2), "stderr：{}", stderr(&out));
    assert!(
        stderr(&out).contains("覆盖前副本"),
        "stderr：{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("--allow-overwrite"),
        "stderr：{}",
        stderr(&out)
    );
    assert_eq!(snapshot(&backup), before, "旧备份产物不得被改动");
    assert!(!target.join("voucher.db").exists(), "拒绝分支不得建库");
}
