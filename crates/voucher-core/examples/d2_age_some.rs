//! D-2 真机验收：往数据库注入「老于 N 天」的增强版（修改文件 mtime）
//!
//! 用法：cargo run -p voucher-core --example d2_age_some
//!
//! 行为：
//! - 找到第 1、第 2 张增强版文件
//! - 把它们的 mtime 改到 60 天前
//! - 其余保持「今天」
//!
//! 配合设置页「早于 N 天」策略测试。

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use voucher_core::db::repos::attachment_repo;
use voucher_core::CoreContext;

fn main() {
    let data_dir = PathBuf::from(std::env::var("APPDATA").expect("APPDATA"))
        .join("VoucherStudio")
        .join("data");
    let ctx = CoreContext::init(Some(data_dir.clone())).expect("init");
    println!("✓ 数据目录: {}", data_dir.display());

    let atts = ctx
        .db
        .with_conn(attachment_repo::list_enhanced)
        .expect("list_enhanced");
    if atts.is_empty() {
        println!("无增强版，跳过（请先 c_step_seed 注入 + d2_seed_hits 写命中）");
        return;
    }
    println!("找到 {} 张增强版", atts.len());

    let now = SystemTime::now();
    let sixty_days_ago = now - Duration::from_secs(60 * 86_400);

    for (i, a) in atts.iter().enumerate() {
        if i >= 2 {
            break; // 只改前 2 张
        }
        let Some(rel) = &a.enhanced_path else {
            continue;
        };
        let abs = ctx.paths.attachment_abs(rel);
        if !abs.exists() {
            println!("  - id={} 文件不存在，跳过", a.id);
            continue;
        }
        let r = std::fs::File::options().write(true).open(&abs);
        match r {
            Ok(f) => {
                if let Err(e) = f.set_modified(sixty_days_ago) {
                    println!("  - id={} 改 mtime 失败: {e}", a.id);
                } else {
                    println!(
                        "  - id={} voucher_id={} mtime 改为 60 天前 ({})",
                        a.id,
                        a.voucher_id,
                        abs.display()
                    );
                }
            }
            Err(e) => println!("  - id={} 打开失败: {e}", a.id),
        }
    }
    println!("=== 完成 ===");
}
