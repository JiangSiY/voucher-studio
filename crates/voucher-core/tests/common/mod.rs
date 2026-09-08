//! 集成测试共享辅助：临时 CoreContext、测试图片生成器、计数器。
//! （各测试文件按需取用，不要求全部使用）

#![allow(dead_code)]

use voucher_core::{CoreContext, CoreError, Result};

pub fn test_ctx() -> (tempfile::TempDir, CoreContext) {
    let dir = tempfile::tempdir().expect("tempdir");
    let ctx = CoreContext::init(Some(dir.path().to_path_buf())).expect("init");
    (dir, ctx)
}

pub fn make_png(path: &std::path::Path) {
    let img = image::RgbImage::from_pixel(64, 48, image::Rgb([180u8, 60, 60]));
    img.save(path).expect("save png");
}

/// 不同内容的图片生成器（同批每张内容必须不同，否则会被 MD5 去重）
pub fn make_png_variant(path: &std::path::Path, shade: u8) {
    let img =
        image::RgbImage::from_pixel(64, 48, image::Rgb([shade, 60, 180u8.saturating_sub(shade)]));
    img.save(path).expect("save png");
}

pub fn voucher_count(ctx: &CoreContext) -> Result<i64> {
    ctx.db.with_conn(|conn| {
        conn.query_row("SELECT COUNT(*) FROM vouchers", [], |r| r.get(0))
            .map_err(CoreError::from)
    })
}
