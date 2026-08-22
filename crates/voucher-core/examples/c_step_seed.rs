//! C 步真机验收：往指定 base_dir 注入 N 张测试凭证（含增强版）
//!
//! 用法：cargo run -p voucher-core --example c_step_seed -- --target <base_dir> [--count 6]
//!
//! 该 binary 与 e2e 测试共用 voucher-core，构造 800×600 模拟文档图，调用 ingestion
//! 完整触发 5 步增强流水线，落到真实数据目录，启动 Voucher Studio 即可看到 UI。

use std::path::PathBuf;

use image::{ImageBuffer, Rgb};
use voucher_core::services::ingestion;
use voucher_core::CoreContext;

fn make_doc(path: &std::path::Path, ext: &str, seed: u8) {
    let mut img = ImageBuffer::<Rgb<u8>, _>::from_pixel(800, 600, Rgb([255u8, 255, 255]));
    for y in 60..540 {
        for x in 80..720 {
            if !(65..=535).contains(&y) || !(85..=715).contains(&x) {
                img.put_pixel(x, y, Rgb([0, 0, 0]));
            }
        }
    }
    for i in 0..5 {
        let y = 200 + i * 60;
        for x in 120..700 {
            img.put_pixel(x, y, Rgb([0, 0, 0]));
        }
    }
    // seed 不同 → 不同位置 / 颜色的填充块，避免被 MD5 去重
    let r = ((seed as u16 * 73) % 256) as u8;
    let g = ((seed as u16 * 137 + 11) % 256) as u8;
    let b = ((seed as u16 * 191 + 23) % 256) as u8;
    let off = (seed as i32 * 17) % 200;
    for y in 100..140 {
        for x in (120 + off)..(260 + off) {
            if x >= 0 && x < 800 {
                img.put_pixel(x as u32, y, Rgb([r, g, b]));
            }
        }
    }
    match ext {
        "jpg" | "jpeg" => img.save(path).expect("save jpg"),
        _ => img.save(path).expect("save png"),
    }
}

fn main() {
    let mut target: Option<PathBuf> = None;
    let mut count: usize = 6;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--target" => target = args.next().map(PathBuf::from),
            "--count" => {
                count = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .expect("--count <num>")
            }
            other => panic!("未知参数: {other}"),
        }
    }
    let target = target.expect("--target <base_dir> 必填");

    let src_dir = target.join("__seed_src__");
    std::fs::create_dir_all(&src_dir).expect("create src dir");
    let mut sources: Vec<PathBuf> = Vec::new();
    for i in 0..count {
        let ext = if i % 2 == 0 { "jpg" } else { "png" };
        let p = src_dir.join(format!("scan_{:02}.{}", i + 1, ext));
        make_doc(&p, ext, i as u8);
        sources.push(p);
    }

    let ctx = CoreContext::init(Some(target.clone())).expect("init target");
    println!("✓ 已初始化数据目录：{}", target.display());

    let summary = ingestion::import_images(&ctx, &sources, None, false).expect("import");
    println!(
        "✓ 导入完成：vouchers={} skipped_dup={}",
        summary.vouchers.len(),
        summary.skipped_duplicates.len()
    );
    for v in &summary.vouchers {
        println!(
            "  - voucher_id={}  no={}  enhanced={:?}  source={:?}",
            v.voucher_id,
            v.voucher_no,
            v.attachment.enhanced_path,
            v.attachment.enhanced_source
        );
    }

    let _ = std::fs::remove_dir_all(&src_dir);
    println!("✓ 已清理临时源目录");

    let lock = target.join(".lock");
    if lock.exists() {
        let _ = std::fs::remove_file(&lock);
        println!("✓ 已删除 .lock");
    }

    println!("✓ Seeding 完成");
}
