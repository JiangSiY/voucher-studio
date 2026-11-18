//! voucher-cli：给脚本/自动化用的最小命令行入口。
//!
//! 只做两件事，都是幂等的：
//! - `import-ledger`：把台账导出的三张清单 CSV 导进 voucher（新建/命中已有凭证）
//! - `list-pending`：列出「台账已导、缺附件」的凭证，供打印待归档清单
//!
//! 设计约定：
//! - 不开新依赖（本仓库运行环境无法下载 crate），参数手写解析；
//! - 与 GUI 共用同一个数据库（默认 `~/Library/Application Support/VoucherStudio`），
//!   同一时刻只允许一个写者（数据库文件锁会拦住并发）。
//! - 退出码：0 成功；1 业务错误；2 参数错误。

use std::path::PathBuf;
use std::process::ExitCode;

use voucher_core::services::{ingestion, query};
use voucher_core::CoreContext;

fn usage() -> &'static str {
    "voucher-cli 0.1
用法：
  voucher-cli import-ledger --csv <清单.csv> --kind <shoukuan|fukuan|jiti> [--dry-run] [--data-dir <目录>]
  voucher-cli list-pending [--json] [--data-dir <目录>]
  voucher-cli restore --zip <备份.zip> --target <空目录>

说明：
  import-ledger  台账导出的收款/付款/计提清单 → voucher 凭证（幂等键=流水号/单据号）
  list-pending   列出已导入但还没配图的凭证
  restore        把全量备份还原到一个空目录（冷备接管用：i3 PC 上跑完指向该目录即可）
  --data-dir     指定数据目录（默认 ~/Library/Application Support/VoucherStudio）
"
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn fail(code: u8, msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("错误：{msg}");
    ExitCode::from(code)
}

/// 把全量备份还原到目标目录（冷备接管）。
///
/// 为什么要单独一个命令：网页版没有「重启应用」按钮，恢复走的是
/// 「写 restore.pending → 重启落位」；冷备接管时没有旧进程可重启，
/// 直接把备份摊到空目录、再让 center-server 指向它最省事。
fn restore_into(zip: &std::path::Path, target: &std::path::Path) -> voucher_core::Result<()> {
    use voucher_core::services::backup;
    std::fs::create_dir_all(target)?;
    // 先建一个干净的 ctx（只为了拿到临时目录），恢复结果落在 target/temp 里
    let ctx = CoreContext::init(Some(target.to_path_buf()))?;
    let staging = ctx.paths.temp.join("restore-staging");
    let _ = std::fs::remove_dir_all(&staging);
    let summary = backup::restore_backup(&ctx, zip, &staging)?;

    // 关掉刚建的空库，再落位（否则拷过去的 db 会被占用中的连接写回）
    drop(ctx);

    let staged_db = staging.join("data").join("voucher.db");
    if staged_db.exists() {
        std::fs::copy(&staged_db, target.join("voucher.db"))?;
    }
    let staged_att = staging.join("attachments");
    if staged_att.is_dir() {
        let dest = target.join("attachments");
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&staged_att, &dest)?;
    }
    let staged_cfg = staging.join("config").join("app.json");
    if staged_cfg.exists() {
        let cfg_dir = target.join("config");
        std::fs::create_dir_all(&cfg_dir)?;
        std::fs::copy(&staged_cfg, cfg_dir.join("app.json"))?;
    }
    println!(
        "已还原：{}（附件 {} 个，校验={}）",
        target.display(),
        summary.attachment_count,
        summary.checksums_verified
    );
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        print!("{}", usage());
        return ExitCode::from(2);
    };

    let data_dir = arg_value(&args, "--data-dir").map(PathBuf::from);

    match cmd.as_str() {
        "-h" | "--help" | "help" => {
            print!("{}", usage());
            ExitCode::SUCCESS
        }
        "import-ledger" => {
            let (Some(csv), Some(kind_raw)) =
                (arg_value(&args, "--csv"), arg_value(&args, "--kind"))
            else {
                return fail(2, "import-ledger 需要 --csv 与 --kind");
            };
            let kind = match kind_raw.as_str() {
                "shoukuan" | "收款" => ingestion::LedgerKind::Shoukuan,
                "fukuan" | "付款" => ingestion::LedgerKind::Fukuan,
                "jiti" | "计提" => ingestion::LedgerKind::Jiti,
                other => return fail(2, format!("未知 kind：{other}")),
            };
            let csv = PathBuf::from(csv);
            if !csv.exists() {
                return fail(1, format!("CSV 不存在：{}", csv.display()));
            }
            let dry_run = args.iter().any(|a| a == "--dry-run");
            let ctx = match CoreContext::init(data_dir) {
                Ok(c) => c,
                Err(e) => return fail(1, e),
            };
            match ingestion::import_ledger_csv(&ctx, &csv, kind, dry_run) {
                Ok(sum) => {
                    println!("{}", serde_json::to_string_pretty(&sum).unwrap_or_default());
                    ExitCode::SUCCESS
                }
                Err(e) => fail(1, e),
            }
        }
        "restore" => {
            let (Some(zip), Some(target)) =
                (arg_value(&args, "--zip"), arg_value(&args, "--target"))
            else {
                return fail(2, "restore 需要 --zip 与 --target");
            };
            let zip = PathBuf::from(zip);
            if !zip.exists() {
                return fail(1, format!("备份不存在：{}", zip.display()));
            }
            let target = PathBuf::from(target);
            match restore_into(&zip, &target) {
                Ok(summary) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&summary).unwrap_or_default()
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => fail(1, e),
            }
        }
        "list-pending" => {
            let ctx = match CoreContext::init(data_dir) {
                Ok(c) => c,
                Err(e) => return fail(1, e),
            };
            match query::list_pending_ledger(&ctx) {
                Ok(items) => {
                    if args.iter().any(|a| a == "--json") {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&items).unwrap_or_default()
                        );
                    } else if items.is_empty() {
                        println!("没有待归档凭证。");
                    } else {
                        println!("待归档 {} 张：", items.len());
                        for it in items {
                            let yuan = it
                                .amount_cents
                                .map(|c| format!("{:.2}", c as f64 / 100.0))
                                .unwrap_or_else(|| "-".into());
                            println!(
                                "  {}  {}  {}  ¥{}  {}",
                                it.voucher_date,
                                it.voucher_no,
                                it.ledger_ref,
                                yuan,
                                it.counterparty
                            );
                        }
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(1, e),
            }
        }
        other => fail(2, format!("未知命令：{other}\n\n{}", usage())),
    }
}
