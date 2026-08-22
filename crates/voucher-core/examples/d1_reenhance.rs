//! D-1 验收辅助：对主库内所有 `enhanced_path IS NULL` 的附件跑一次 7 步增强，
//! 写回 attachments.enhanced_path，使列表缩略图能渲染增强版。
//!
//! 用法：cargo run -p voucher-core --example d1_reenhance
//!
//! 默认指向 %APPDATA%/VoucherStudio/data；可 --target <base_dir> 覆盖。

use std::path::PathBuf;

use voucher_core::db::repos::attachment_repo;
use voucher_core::services::enhance;
use voucher_core::CoreContext;

fn main() {
    let mut target: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--target" => target = args.next().map(PathBuf::from),
            other => panic!("未知参数: {other}"),
        }
    }
    let target = target.unwrap_or_else(|| {
        let appdata = std::env::var("APPDATA").expect("APPDATA");
        PathBuf::from(appdata).join("VoucherStudio").join("data")
    });

    let ctx = CoreContext::init(Some(target.clone())).expect("init target");
    let base = ctx.paths.base.clone();
    let attachments_root = ctx.paths.attachments.clone();
    println!("✓ 数据目录: {}", base.display());
    println!("✓ 附件根:   {}", attachments_root.display());

    // 收集所有 enhanced_path IS NULL 的附件
    let todo = ctx
        .db
        .with_conn(|conn| {
            use rusqlite::params;
            let mut stmt = conn
                .prepare("SELECT id, file_path FROM attachments WHERE enhanced_path IS NULL OR enhanced_path = ''")?;
            let mut rows = stmt.query_map(params![], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            let mut out = Vec::new();
            while let Some(r) = rows.next().transpose()? {
                out.push(r);
            }
            Ok::<_, voucher_core::CoreError>(out)
        })
        .expect("query");
    println!("待增强: {} 张", todo.len());

    let mut ok = 0usize;
    let mut fail = 0usize;
    for (id, rel) in todo {
        let abs = attachments_root.join(rel.replace('/', "\\"));
        println!("  - id={}  src={}", id, abs.display());
        match enhance::write_enhanced(&abs, &rel.replace('\\', "/"), &enhance::PipelineOptions::default()) {
            Some(new_rel) => {
                let res = ctx.db.with_conn(|conn| attachment_repo::set_enhanced(conn, id, &new_rel));
                match res {
                    Ok(_) => {
                        println!("    ✓ 增强成功: {}", new_rel);
                        ok += 1;
                    }
                    Err(e) => {
                        println!("    ✗ 写库失败: {e}");
                        fail += 1;
                    }
                }
            }
            None => {
                println!("    ✗ 增强失败（已记 warn 日志）");
                fail += 1;
            }
        }
    }
    println!("\n=== 汇总：成功 {ok} / 失败 {fail} ===");

    // 不需要 end；ctx 离开作用域自动 drop 连接
    drop(ctx);
}
