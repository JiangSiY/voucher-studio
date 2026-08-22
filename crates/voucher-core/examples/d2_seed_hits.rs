//! D-2 真机验收：往增强版注入命中数据（让设置页的命中率有可视内容）
//!
//! 用法：cargo run -p voucher-core --example d2_seed_hits
//!
//! 行为：
//! - 列出所有 enhanced 附件
//! - 给前 4 张写 5 次 OCR 命中 + 1 次 viewer 命中
//! - 给第 5、6 张写 1 次 OCR 命中
//! - 不动最后 0 张（如果存在）
//!
//! 用于真机走查时让「累计命中」「近 30 天命中」「活跃占比」三个指标都有数。

use std::path::PathBuf;

use voucher_core::db::repos::attachment_repo;
use voucher_core::db::repos::enhance_hit_repo;
use voucher_core::CoreContext;

fn main() {
    let data_dir = dirs_data().unwrap_or_else(|| {
        PathBuf::from(std::env::var("APPDATA").expect("APPDATA"))
            .join("VoucherStudio")
            .join("data")
    });
    let ctx = CoreContext::init(Some(data_dir.clone())).expect("init");
    println!("✓ 数据目录: {}", data_dir.display());

    let atts = ctx
        .db
        .with_conn(attachment_repo::list_enhanced)
        .expect("list_enhanced");
    println!("待命中: {} 张增强版", atts.len());

    let plan: Vec<(usize, &[&str])> = vec![
        (0, &["ocr", "ocr", "ocr", "ocr", "ocr", "viewer"]), // id 1: 6
        (1, &["ocr", "ocr", "ocr", "ocr", "ocr"]),           // id 2: 5
        (2, &["ocr", "ocr", "ocr", "ocr", "ocr"]),           // id 3: 5
        (3, &["ocr", "ocr", "ocr", "ocr", "ocr"]),           // id 4: 5
        (4, &["ocr"]),                                       // id 5: 1
        (5, &["ocr"]),                                       // id 6: 1
    ];

    for (i, sources) in &plan {
        if *i >= atts.len() {
            break;
        }
        let att = &atts[*i];
        for s in sources.iter() {
            let src = match *s {
                "ocr" => enhance_hit_repo::HitSource::Ocr,
                "export" => enhance_hit_repo::HitSource::Export,
                "viewer" => enhance_hit_repo::HitSource::Viewer,
                _ => panic!("unknown source: {s}"),
            };
            ctx.db
                .with_conn(|c| enhance_hit_repo::record(c, att.id, src))
                .expect("record");
        }
        println!(
            "  - id={}  voucher_id={}  +{} hits",
            att.id,
            att.voucher_id,
            sources.len()
        );
    }

    let total = ctx
        .db
        .with_conn(enhance_hit_repo::count_total)
        .expect("count_total");
    println!("=== 累计命中: {} ===", total);
}

fn dirs_data() -> Option<PathBuf> {
    Some(
        PathBuf::from(std::env::var("APPDATA").ok()?)
            .join("VoucherStudio")
            .join("data"),
    )
}
