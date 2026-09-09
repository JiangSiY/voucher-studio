//! 像素增强：去噪 + 锐化 + 灰度化 + 自适应二值化（5.5 方案步骤 4-7）
//!
//! B 步实装：imageproc 0.25 没有 Sauvola，二值化改走 OpenCV 自适应阈值（与灰度化协同）。
//! v0.6.1 实装：去噪（中值 3×3）与锐化（Unsharp Mask）插在裁边后、灰度化前。
//!
//! 错误处理约定（体检 #9）：OpenCV 错误经 `CoreError::Cv(#[from])` 自动 `?` 转换；
//! 空输入 / 尺寸过小等业务校验仍返回 `Invalid`。

use opencv::{
    core::Mat,
    imgproc::{
        adaptive_threshold, cvt_color, gaussian_blur, median_blur, AdaptiveThresholdTypes,
        ColorConversionCodes, ThresholdTypes,
    },
    prelude::*,
};

use crate::{CoreError, Result};

/// 彩色 Mat (BGR) → 灰度 Mat (8UC1)。
///
/// 步骤 4 的核心：与 enhance_geom 走 OpenCV 路径，行为统一。
pub fn to_grayscale(src: &Mat) -> Result<Mat> {
    if src.empty() {
        return Err(CoreError::Invalid("grayscale: 输入图像为空".into()));
    }
    let mut gray = Mat::default();
    if src.channels() == 1 {
        // 已是灰度图，克隆返回
        return Ok(src.try_clone()?);
    }
    cvt_color(
        src,
        &mut gray,
        ColorConversionCodes::COLOR_BGR2GRAY as i32,
        0,
    )?;
    Ok(gray)
}

/// 自适应二值化（高斯均值法）。
///
/// 经典参数：block_size=31, C=10。比 Sauvola 略粗糙但胜在无需额外依赖。
/// 失败兜底：图太小（< 32px）直接返错（adaptive_threshold 不接受过小窗口）。
pub fn binarize_sauvola(src: &Mat) -> Result<Mat> {
    if src.empty() {
        return Err(CoreError::Invalid("binarize: 输入图像为空".into()));
    }
    if src.cols() < 32 || src.rows() < 32 {
        return Err(CoreError::Invalid("binarize: 图太小（< 32px）".into()));
    }
    // 若输入是彩色图，先灰度化
    let gray = if src.channels() == 1 {
        src.try_clone()?
    } else {
        to_grayscale(src)?
    };
    let mut dst = Mat::default();
    adaptive_threshold(
        &gray,
        &mut dst,
        255.0,
        AdaptiveThresholdTypes::ADAPTIVE_THRESH_GAUSSIAN_C as i32,
        ThresholdTypes::THRESH_BINARY as i32,
        31,
        10.0,
    )?;
    Ok(dst)
}

/// 中值去噪（3×3）。
///
/// v0.6.1 步骤 4：保边平滑，适合扫描件 / 手机拍摄的椒盐噪点与传感器噪声；
/// 中值对边缘的破坏远小于高斯，且在二值化前去除孤立噪点可显著减少二值图的麻点。
pub fn denoise_median(src: &Mat) -> Result<Mat> {
    if src.empty() {
        return Err(CoreError::Invalid("denoise: 输入图像为空".into()));
    }
    let mut dst = Mat::default();
    median_blur(src, &mut dst, 3)?;
    Ok(dst)
}

/// Unsharp Mask 锐化（amount=1.5, σ=1.5）。
///
/// v0.6.1 步骤 5：`out = src * 1.5 - blurred * 0.5`，强化文字边缘对比，
/// 对 OCR 小字号与 PDF 可读性均有收益；过度锐化会放大噪声，与步骤 4 去噪搭配使用。
pub fn unsharp_mask(src: &Mat) -> Result<Mat> {
    if src.empty() {
        return Err(CoreError::Invalid("sharpen: 输入图像为空".into()));
    }
    let mut blurred = Mat::default();
    // ksize=(0,0) + sigma 让 OpenCV 按 σ 推导窗口大小
    gaussian_blur(
        src,
        &mut blurred,
        opencv::core::Size::new(0, 0),
        1.5,
        0.0,
        opencv::core::BORDER_DEFAULT,
    )?;
    let mut dst = Mat::default();
    opencv::core::add_weighted(src, 1.5, &blurred, -0.5, 0.0, &mut dst, -1)?;
    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencv::core::{Mat, Size};

    fn tiny_mat(w: i32, h: i32, channels: i32) -> Mat {
        // OpenCV 0.100 Mat::new_size 是 unsafe（共享数据指针）
        unsafe {
            Mat::new_size(Size::new(w, h), opencv::core::CV_8UC(channels)).unwrap_or_default()
        }
    }

    #[test]
    fn grayscale_empty_errors() {
        let m = Mat::default();
        assert!(to_grayscale(&m).is_err());
    }

    #[test]
    fn binarize_small_errors() {
        let m = tiny_mat(10, 10, 1);
        assert!(binarize_sauvola(&m).is_err());
    }

    #[test]
    fn denoise_empty_errors() {
        let m = Mat::default();
        assert!(denoise_median(&m).is_err());
    }

    #[test]
    fn sharpen_empty_errors() {
        let m = Mat::default();
        assert!(unsharp_mask(&m).is_err());
    }

    #[test]
    fn denoise_and_sharpen_preserve_geometry() {
        // 400x300 彩色图过一遍去噪 + 锐化，尺寸/通道不变
        let m = tiny_mat(400, 300, 3);
        let d = denoise_median(&m).expect("denoise");
        assert_eq!((d.cols(), d.rows(), d.channels()), (400, 300, 3));
        let s = unsharp_mask(&d).expect("sharpen");
        assert_eq!((s.cols(), s.rows(), s.channels()), (400, 300, 3));
    }
}
