//! 图片处理基础能力（对应 v4.3 的 `imaging.py` 的基础部分；
//! 透视矫正等 OpenCV 能力待后续阶段按需引入 opencv crate）

use std::path::Path;

use crate::{CoreError, Result};

pub const SUPPORTED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "bmp", "webp"];

pub fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn is_supported(path: &Path) -> bool {
    SUPPORTED_EXTENSIONS.contains(&ext_of(path).as_str())
}

pub fn mime_from_ext(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

/// 读取图片尺寸（用于校验文件确实是可用图片）
pub fn probe_dimensions(path: &Path) -> Result<(u32, u32)> {
    image::image_dimensions(path)
        .map_err(|e| CoreError::Invalid(format!("无法解析图片 {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_and_mime() {
        let p = Path::new("/tmp/IMG_20260819.JPEG");
        assert_eq!(ext_of(p), "jpeg");
        assert!(is_supported(p));
        assert!(!is_supported(Path::new("/tmp/a.pdf")));
        assert_eq!(mime_from_ext("png"), "image/png");
    }
}
