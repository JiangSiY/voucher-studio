//! CLI 库路径守卫：只读命令不得建库、写命令不得隐式建库，拒绝分支零副作用。
//!
//! 判定方式全部用「目标路径不存在 + 运行后仍不存在」，而不是看退出码就完事——
//! `CoreContext::init` 会先 `ensure_all()` 建目录，只看 exit code 抓不到这种副作用。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_voucher-cli")
}

fn unique_root(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "voucher-cli-dbguard-{}-{}-{tag}",
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

/// 只造「既有库」：目录 + voucher.db + 迁移，不经过 `CoreContext`。
fn seed_db(base: &Path) {
    std::fs::create_dir_all(base).expect("建数据目录");
    let conn = voucher_core::db::connection::open(&base.join("voucher.db")).expect("建库");
    voucher_core::db::schema::init(&conn).expect("建表");
}

fn write_csv(dir: &Path, name: &str, body: &str) -> PathBuf {
    std::fs::create_dir_all(dir).expect("建 CSV 目录");
    let path = dir.join(name);
    std::fs::write(&path, body).expect("写 CSV");
    path
}

#[test]
fn read_only_commands_refuse_a_missing_db_without_creating_dirs() {
    let root = unique_root("readonly");
    let data = root.join("data");

    let out = run(&["list-pending", "--data-dir", data.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "stderr：{}", stderr(&out));
    assert!(
        stderr(&out).contains("只读命令不会新建库"),
        "stderr：{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("voucher.db"),
        "stderr：{}",
        stderr(&out)
    );
    assert!(
        !root.exists(),
        "拒绝分支不得建目录：{} 被创建",
        root.display()
    );

    let out = run(&["c5-fingerprint", "--data-dir", data.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "stderr：{}", stderr(&out));
    assert!(
        stderr(&out).contains("只读命令不会新建库"),
        "stderr：{}",
        stderr(&out)
    );
    assert!(
        !root.exists(),
        "拒绝分支不得建目录：{} 被创建",
        root.display()
    );
}

#[test]
fn import_ledger_refuses_a_missing_db_and_creates_nothing() {
    let root = unique_root("import-missing");
    let data = root.join("data");
    let csv = write_csv(
        &unique_root("import-csv"),
        "收款清单.csv",
        "流水序号,日期,金额,账户,对方,项目,分类,摘要,方向\n9001,2026-11-03,12500.00,银行转账,凯诗蓝德,凯诗蓝德-585kW,电费,收电费,收\n",
    );

    let out = run(&[
        "import-ledger",
        "--csv",
        csv.to_str().unwrap(),
        "--kind",
        "shoukuan",
        "--data-dir",
        data.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1), "stderr：{}", stderr(&out));
    assert!(
        stderr(&out).contains("写命令不会隐式建库"),
        "stderr：{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("voucher.db"),
        "stderr：{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("既有初始化路径"),
        "stderr：{}",
        stderr(&out)
    );
    assert!(
        !root.exists(),
        "拒绝分支不得建目录：{} 被创建",
        root.display()
    );
}

#[test]
fn list_pending_still_reads_an_existing_database() {
    let root = unique_root("list-existing");
    seed_db(&root);

    let out = run(&[
        "list-pending",
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "stderr：{}", stderr(&out));
    let items: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout 应是 JSON 数组");
    assert!(items.as_array().is_some_and(|items| items.is_empty()));
    assert!(root.join("voucher.db").is_file());
}

#[test]
fn import_ledger_output_carries_the_database_path() {
    let root = unique_root("import-out");
    seed_db(&root);
    let csv = write_csv(
        &unique_root("import-out-csv"),
        "收款清单.csv",
        "流水序号,日期,金额,账户,对方,项目,分类,摘要,方向\n9002,2026-11-03,1200.00,银行转账,凯诗蓝德,凯诗蓝德-585kW,电费,收电费,收\n",
    );

    let out = run(&[
        "import-ledger",
        "--csv",
        csv.to_str().unwrap(),
        "--kind",
        "shoukuan",
        "--dry-run",
        "--data-dir",
        root.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "stderr：{}", stderr(&out));
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout 应是 JSON");
    let database = payload["database"].as_str().expect("必须带 database");
    assert!(
        Path::new(database).is_absolute(),
        "database 必须是绝对路径：{database}"
    );
    assert_eq!(database, root.join("voucher.db").display().to_string());
}

#[test]
fn c5_fingerprint_output_carries_the_database_path() {
    let root = unique_root("fingerprint");
    seed_db(&root);

    let out = run(&[
        "c5-fingerprint",
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "stderr：{}", stderr(&out));
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout 应是 JSON");
    let database = payload["database"].as_str().expect("必须带 database");
    assert_eq!(database, root.join("voucher.db").display().to_string());
    assert_eq!(payload["records"], serde_json::json!(0));
}
