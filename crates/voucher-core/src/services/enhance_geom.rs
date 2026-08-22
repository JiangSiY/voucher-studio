//! 几何矫正：倾斜矫正 + 透视矫正 + 自动裁边（5.5 方案步骤 1-3）
//!
//! B 步实装：基于 OpenCV 4.8（opencv-rust 0.100 API）。
//! 任何步骤异常由 `enhance::run_pipeline` 单步失败跳过兜底。

use opencv::{
    core::{Mat, Point, Point2f, Scalar, Size, Vector},
    imgproc::{
        adaptive_threshold, approx_poly_dp, bounding_rect, cvt_color, find_contours,
        get_perspective_transform, get_rotation_matrix_2d, hough_lines, warp_affine,
        warp_perspective, AdaptiveThresholdTypes, ColorConversionCodes, ContourApproximationModes,
        InterpolationFlags, RetrievalModes, ThresholdTypes,
    },
    prelude::*,
};

use crate::{CoreError, Result};

/// 角度制转弧度。
#[inline]
fn deg2rad(d: f64) -> f64 {
    d * std::f64::consts::PI / 180.0
}

/// 倾斜矫正：Hough 直线检测 → 取最强水平线倾角 → 旋转校正。
pub fn deskew(src: &Mat) -> Result<Mat> {
    if src.empty() {
        return Err(CoreError::Invalid("deskew: 输入图像为空".into()));
    }

    let mut gray = Mat::default();
    if src.channels() == 3 || src.channels() == 4 {
        cvt_color(
            src,
            &mut gray,
            ColorConversionCodes::COLOR_BGR2GRAY as i32,
            0,
        )
        .map_err(|e| CoreError::Invalid(format!("deskew: 灰度化失败: {e}")))?;
    } else {
        gray = src.clone();
    }

    let mut edges = Mat::default();
    opencv::imgproc::canny(&gray, &mut edges, 80.0, 200.0, 3, false)
        .map_err(|e| CoreError::Invalid(format!("deskew: Canny 失败: {e}")))?;

    let mut lines = Vector::<opencv::core::Vec2f>::new();
    hough_lines(
        &edges,
        &mut lines,
        1.0,
        deg2rad(1.0),
        200,
        50.0,
        10.0,
        0.0,
        std::f64::consts::PI,
    )
    .map_err(|e| CoreError::Invalid(format!("deskew: Hough 失败: {e}")))?;

    if lines.is_empty() {
        return src
            .try_clone()
            .map_err(|e| CoreError::Invalid(e.to_string()));
    }

    // Vec2f[1] = theta，0.100 是 f32，统一转 f64
    let mut angles: Vec<f64> = Vec::new();
    for line in lines.iter().take(50) {
        let theta = line[1] as f64;
        let mut angle = theta - std::f64::consts::PI / 2.0;
        if angle > std::f64::consts::PI / 4.0 {
            angle -= std::f64::consts::PI / 2.0;
        } else if angle < -std::f64::consts::PI / 4.0 {
            angle += std::f64::consts::PI / 2.0;
        }
        angles.push(angle);
    }
    if angles.is_empty() {
        return src
            .try_clone()
            .map_err(|e| CoreError::Invalid(e.to_string()));
    }
    angles.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_angle = angles[angles.len() / 2].to_degrees();

    if median_angle.abs() < 0.5 {
        return src
            .try_clone()
            .map_err(|e| CoreError::Invalid(e.to_string()));
    }

    let (w, h) = (src.cols(), src.rows());
    let center = Point2f::new(w as f32 / 2.0, h as f32 / 2.0);
    let rot = get_rotation_matrix_2d(center, median_angle, 1.0)
        .map_err(|e| CoreError::Invalid(format!("deskew: 旋转矩阵失败: {e}")))?;
    let mut dst = Mat::default();
    warp_affine(
        src,
        &mut dst,
        &rot,
        Size::new(w, h),
        InterpolationFlags::INTER_LINEAR as i32,
        opencv::core::BorderTypes::BORDER_REPLICATE as i32,
        Scalar::default(),
    )
    .map_err(|e| CoreError::Invalid(format!("deskew: warp_affine 失败: {e}")))?;
    Ok(dst)
}

/// 透视矫正：检测最大四边形 → 透视变换。
pub fn dewarp(src: &Mat) -> Result<Mat> {
    if src.empty() {
        return Err(CoreError::Invalid("dewarp: 输入图像为空".into()));
    }

    let mut gray = Mat::default();
    if src.channels() == 3 || src.channels() == 4 {
        cvt_color(
            src,
            &mut gray,
            ColorConversionCodes::COLOR_BGR2GRAY as i32,
            0,
        )
        .map_err(|e| CoreError::Invalid(format!("dewarp: 灰度化失败: {e}")))?;
    } else {
        gray = src.clone();
    }

    let mut binary = Mat::default();
    adaptive_threshold(
        &gray,
        &mut binary,
        255.0,
        AdaptiveThresholdTypes::ADAPTIVE_THRESH_GAUSSIAN_C as i32,
        ThresholdTypes::THRESH_BINARY as i32,
        31,
        10.0,
    )
    .map_err(|e| CoreError::Invalid(format!("dewarp: 自适应阈值失败: {e}")))?;

    let mut contours: Vector<opencv::core::Vector<Point>> = Vector::new();
    find_contours(
        &binary,
        &mut contours,
        RetrievalModes::RETR_EXTERNAL as i32,
        ContourApproximationModes::CHAIN_APPROX_SIMPLE as i32,
        Point::new(0, 0),
    )
    .map_err(|e| CoreError::Invalid(format!("dewarp: find_contours 失败: {e}")))?;

    if contours.is_empty() {
        return src
            .try_clone()
            .map_err(|e| CoreError::Invalid(e.to_string()));
    }

    let mut best_quad: Option<[Point; 4]> = None;
    let mut best_area = 0.0_f64;
    for contour in contours.iter() {
        let perim = opencv::imgproc::contour_area(&contour, false)
            .map_err(|e| CoreError::Invalid(format!("dewarp: contour_area 失败: {e}")))?;
        if perim < 1000.0 {
            continue;
        }
        let mut approx = opencv::core::Vector::<Point>::new();
        approx_poly_dp(&contour, &mut approx, 0.02 * perim, true)
            .map_err(|e| CoreError::Invalid(format!("dewarp: approx_poly_dp 失败: {e}")))?;
        if approx.len() == 4 {
            let area = opencv::imgproc::contour_area(&approx, false).unwrap_or(0.0);
            if area > best_area {
                best_area = area;
                let pts: Vec<Point> = approx.iter().collect();
                if pts.len() == 4 {
                    best_quad = Some([pts[0], pts[1], pts[2], pts[3]]);
                }
            }
        }
    }

    let quad = match best_quad {
        Some(q) => q,
        None => {
            return src
                .try_clone()
                .map_err(|e| CoreError::Invalid(e.to_string()))
        }
    };

    let mut pts = quad.to_vec();
    pts.sort_by(|a, b| {
        (a.y, a.x)
            .partial_cmp(&(b.y, b.x))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let (tl, br) = (pts[0], pts[3]);
    let (mut tr, mut bl) = (pts[1], pts[2]);
    if tr.x < bl.x {
        std::mem::swap(&mut tr, &mut bl);
    }
    let src_pts: Vector<Point2f> = Vector::from_iter(vec![
        Point2f::new(tl.x as f32, tl.y as f32),
        Point2f::new(tr.x as f32, tr.y as f32),
        Point2f::new(br.x as f32, br.y as f32),
        Point2f::new(bl.x as f32, bl.y as f32),
    ]);
    let max_w = ((br.x - tl.x) as f32)
        .abs()
        .max((tr.x - bl.x) as f32)
        .max(1.0) as i32;
    let max_h = ((br.y - tl.y) as f32)
        .abs()
        .max((tr.y - bl.y) as f32)
        .max(1.0) as i32;
    let dst_pts: Vector<Point2f> = Vector::from_iter(vec![
        Point2f::new(0.0, 0.0),
        Point2f::new((max_w - 1) as f32, 0.0),
        Point2f::new((max_w - 1) as f32, (max_h - 1) as f32),
        Point2f::new(0.0, (max_h - 1) as f32),
    ]);

    let m = get_perspective_transform(&src_pts, &dst_pts, opencv::core::DECOMP_LU)
        .map_err(|e| CoreError::Invalid(format!("dewarp: 透视矩阵失败: {e}")))?;
    let mut dst = Mat::default();
    warp_perspective(
        src,
        &mut dst,
        &m,
        Size::new(max_w, max_h),
        InterpolationFlags::INTER_LINEAR as i32,
        opencv::core::BorderTypes::BORDER_CONSTANT as i32,
        Scalar::default(),
    )
    .map_err(|e| CoreError::Invalid(format!("dewarp: warp_perspective 失败: {e}")))?;
    Ok(dst)
}

/// 自动裁边：找非白边 → 内缩 5% 边距。
pub fn autocrop(src: &Mat) -> Result<Mat> {
    if src.empty() {
        return Err(CoreError::Invalid("autocrop: 输入图像为空".into()));
    }

    let mut gray = Mat::default();
    if src.channels() == 3 || src.channels() == 4 {
        cvt_color(
            src,
            &mut gray,
            ColorConversionCodes::COLOR_BGR2GRAY as i32,
            0,
        )
        .map_err(|e| CoreError::Invalid(format!("autocrop: 灰度化失败: {e}")))?;
    } else {
        gray = src.clone();
    }

    let mut binary = Mat::default();
    opencv::imgproc::threshold(
        &gray,
        &mut binary,
        240.0,
        255.0,
        ThresholdTypes::THRESH_BINARY_INV as i32,
    )
    .map_err(|e| CoreError::Invalid(format!("autocrop: threshold 失败: {e}")))?;

    let mut contours: Vector<opencv::core::Vector<Point>> = Vector::new();
    find_contours(
        &binary,
        &mut contours,
        RetrievalModes::RETR_EXTERNAL as i32,
        ContourApproximationModes::CHAIN_APPROX_SIMPLE as i32,
        Point::new(0, 0),
    )
    .map_err(|e| CoreError::Invalid(format!("autocrop: find_contours 失败: {e}")))?;

    if contours.is_empty() {
        return src
            .try_clone()
            .map_err(|e| CoreError::Invalid(e.to_string()));
    }

    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for c in contours.iter() {
        let r = bounding_rect(&c)
            .map_err(|e| CoreError::Invalid(format!("autocrop: bbox 失败: {e}")))?;
        min_x = min_x.min(r.x);
        min_y = min_y.min(r.y);
        max_x = max_x.max(r.x + r.width);
        max_y = max_y.max(r.y + r.height);
    }
    if min_x >= max_x || min_y >= max_y {
        return src
            .try_clone()
            .map_err(|e| CoreError::Invalid(e.to_string()));
    }
    let pad_x = ((max_x - min_x) as f32 * 0.05) as i32;
    let pad_y = ((max_y - min_y) as f32 * 0.05) as i32;
    let x = (min_x + pad_x).max(0);
    let y = (min_y + pad_y).max(0);
    let w = (max_x - min_x - 2 * pad_x).max(1);
    let h = (max_y - min_y - 2 * pad_y).max(1);
    let roi = opencv::core::Rect::new(x, y, w, h);
    let cropped =
        Mat::roi(src, roi).map_err(|e| CoreError::Invalid(format!("autocrop: roi 失败: {e}")))?;
    let mut dst = Mat::default();
    cropped
        .copy_to(&mut dst)
        .map_err(|e| CoreError::Invalid(e.to_string()))?;
    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deskew_empty_errors() {
        let m = Mat::default();
        assert!(deskew(&m).is_err());
    }

    #[test]
    fn dewarp_empty_errors() {
        let m = Mat::default();
        assert!(dewarp(&m).is_err());
    }

    #[test]
    fn autocrop_empty_errors() {
        let m = Mat::default();
        assert!(autocrop(&m).is_err());
    }
}
