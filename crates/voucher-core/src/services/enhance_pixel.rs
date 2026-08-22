//! 像素增强：灰度化 + 自适应二值化（5.5 方案步骤 4-5）
//!
//! B 步实装：imageproc 0.25 没有 Sauvola，二值化改走 OpenCV 自适应阈值（与灰度化协同）。

use opencv::{
    core::Mat,
    imgproc::{adaptive_threshold, cvt_color, ColorConversionCodes, AdaptiveThresholdTypes, ThresholdTypes},
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
        return src.try_clone().map_err(|e| CoreError::Invalid(e.to_string()));
    }
    cvt_color(src, &mut gray, ColorConversionCodes::COLOR_BGR2GRAY as i32, 0)
        .map_err(|e| CoreError::Invalid(format!("grayscale: cvt_color 失败: {e}")))?;
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
        src.try_clone().map_err(|e| CoreError::Invalid(e.to_string()))?
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
    )
    .map_err(|e| CoreError::Invalid(format!("binarize: adaptive_threshold 失败: {e}")))?;
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
}
