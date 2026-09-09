//! 几何矫正：倾斜矫正 + 透视矫正 + 自动裁边（5.5 方案步骤 1-3）
//!
//! B 步实装：基于 OpenCV 4.8（opencv-rust 0.100 API）。
//! 任何步骤异常由 `enhance::run_pipeline` 单步失败跳过兜底。
//!
//! 错误处理约定（体检 #9）：OpenCV 返回的错误通过 `CoreError::Cv(#[from])` 自动 `?` 转换，
//! 不再逐处 `map_err(|e| CoreError::Invalid(...))` 样板；空输入等业务校验仍返回 `Invalid`。

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
        )?;
    } else {
        gray = src.clone();
    }

    let mut edges = Mat::default();
    opencv::imgproc::canny(&gray, &mut edges, 80.0, 200.0, 3, false)?;

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
    )?;

    if lines.is_empty() {
        return Ok(src.try_clone()?);
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
        return Ok(src.try_clone()?);
    }
    angles.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_angle = angles[angles.len() / 2].to_degrees();

    if median_angle.abs() < 0.5 {
        return Ok(src.try_clone()?);
    }

    let (w, h) = (src.cols(), src.rows());
    let center = Point2f::new(w as f32 / 2.0, h as f32 / 2.0);
    let rot = get_rotation_matrix_2d(center, median_angle, 1.0)?;
    let mut dst = Mat::default();
    warp_affine(
        src,
        &mut dst,
        &rot,
        Size::new(w, h),
        InterpolationFlags::INTER_LINEAR as i32,
        opencv::core::BorderTypes::BORDER_REPLICATE as i32,
        Scalar::default(),
    )?;
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
        )?;
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
    )?;

    let mut contours: Vector<opencv::core::Vector<Point>> = Vector::new();
    find_contours(
        &binary,
        &mut contours,
        RetrievalModes::RETR_EXTERNAL as i32,
        ContourApproximationModes::CHAIN_APPROX_SIMPLE as i32,
        Point::new(0, 0),
    )?;

    if contours.is_empty() {
        return Ok(src.try_clone()?);
    }

    let mut best_quad: Option<[Point; 4]> = None;
    let mut best_area = 0.0_f64;
    for contour in contours.iter() {
        let perim = opencv::imgproc::contour_area(&contour, false)?;
        if perim < 1000.0 {
            continue;
        }
        let mut approx = opencv::core::Vector::<Point>::new();
        approx_poly_dp(&contour, &mut approx, 0.02 * perim, true)?;
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
        None => return Ok(src.try_clone()?),
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

    let m = get_perspective_transform(&src_pts, &dst_pts, opencv::core::DECOMP_LU)?;
    let mut dst = Mat::default();
    warp_perspective(
        src,
        &mut dst,
        &m,
        Size::new(max_w, max_h),
        InterpolationFlags::INTER_LINEAR as i32,
        opencv::core::BorderTypes::BORDER_CONSTANT as i32,
        Scalar::default(),
    )?;
    Ok(dst)
}

/// 自动裁边（v0.8 重写：修复「内缩 5% 切掉票据边缘内容」的方向性 bug）。
///
/// 原实现四个缺陷与本版修复：
/// 1. bbox 后内缩 5% —— 把印章/税率栏/二维码等票据边缘直接裁掉 → 改为**外扩 2% 安全边**
/// 2. 固定阈值 240 —— 拍照阴影/暗角 < 240 被误判为内容（bbox 全图，裁不动）→ 改 **Otsu 自适应**
/// 3. 全轮廓并集 —— 角落脏点扩大边界 → 改取**最大面积轮廓**的 bbox
/// 4. 极端结果照单全收 → 裁后面积 < 原图 10% 或 > 97% 视为检测失败，**回退原图不裁**
///    （宁可不裁，不可裁错）
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
        )?;
    } else {
        gray = src.try_clone()?;
    }

    // 高斯模糊抑制纸纹/噪点（否则细小噪声会连成大轮廓或碎轮廓）
    let mut blurred = Mat::default();
    opencv::imgproc::gaussian_blur(
        &gray,
        &mut blurred,
        Size::new(5, 5),
        0.0,
        0.0,
        opencv::core::BORDER_DEFAULT,
    )?;

    // Otsu 自动阈值：亮背景暗内容的自适应分界（替代固定 240，抗阴影/曝光差异）
    let mut binary = Mat::default();
    let otsu_inv =
        (ThresholdTypes::THRESH_BINARY_INV as i32) | (ThresholdTypes::THRESH_OTSU as i32);
    opencv::imgproc::threshold(&blurred, &mut binary, 0.0, 255.0, otsu_inv)?;

    let mut contours: Vector<opencv::core::Vector<Point>> = Vector::new();
    find_contours(
        &binary,
        &mut contours,
        RetrievalModes::RETR_EXTERNAL as i32,
        ContourApproximationModes::CHAIN_APPROX_SIMPLE as i32,
        Point::new(0, 0),
    )?;

    if contours.is_empty() {
        return Ok(src.try_clone()?);
    }

    // 最大面积轮廓的 bbox（脏点/杂物不参与边界，原实现取全轮廓并集会被噪声撑大）
    let mut best: Option<opencv::core::Rect> = None;
    let mut best_area: f64 = 0.0;
    for c in contours.iter() {
        let r = bounding_rect(&c)?;
        let area = (r.width as f64) * (r.height as f64);
        if area > best_area {
            best_area = area;
            best = Some(r);
        }
    }
    let Some(r) = best else {
        return Ok(src.try_clone()?);
    };

    // 合理性校验：裁剪结果面积占比（太小=误检碎块；接近全图=背景脏/阴影导致 bbox 全开）
    let src_area = (src.cols() as f64) * (src.rows() as f64);
    let ratio = best_area / src_area;
    if !(0.10..=0.97).contains(&ratio) {
        return Ok(src.try_clone()?);
    }

    // 外扩 2% 安全边（方向与原实现相反：向内容外侧留白，不切内容），clamp 到图像边界
    let pad_x = ((r.width as f64) * 0.02) as i32;
    let pad_y = ((r.height as f64) * 0.02) as i32;
    let x = (r.x - pad_x).max(0);
    let y = (r.y - pad_y).max(0);
    let x2 = (r.x + r.width + pad_x).min(src.cols());
    let y2 = (r.y + r.height + pad_y).min(src.rows());
    let w = (x2 - x).max(1);
    let h = (y2 - y).max(1);
    if w >= src.cols() && h >= src.rows() {
        return Ok(src.try_clone()?);
    }
    let roi = opencv::core::Rect::new(x, y, w, h);
    let cropped = Mat::roi(src, roi)?;
    let mut dst = Mat::default();
    cropped.copy_to(&mut dst)?;
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

    /// 合成图：白底 + 深色矩形块（模拟亮背景暗内容票据）
    fn synth(src_w: i32, src_h: i32, x: i32, y: i32, w: i32, h: i32) -> Mat {
        let mut m = Mat::new_rows_cols_with_default(
            src_h,
            src_w,
            opencv::core::CV_8UC3,
            Scalar::new(255.0, 255.0, 255.0, 0.0),
        )
        .expect("new mat");
        fill_rect(&mut m, x, y, w, h);
        m
    }

    /// 矩形填充深色（BGR 30,30,30）
    fn fill_rect(m: &mut Mat, x: i32, y: i32, w: i32, h: i32) {
        for yy in y..(y + h) {
            for xx in x..(x + w) {
                if let Ok(px) = m.at_2d_mut::<opencv::core::Vec3b>(yy, xx) {
                    *px = opencv::core::Vec3b::from([30, 30, 30]);
                }
            }
        }
    }

    /// 核心回归：裁剪后票据边缘内容必须全保留（原实现内缩 5% 会切掉边缘）
    #[test]
    fn autocrop_keeps_full_content_with_margin() {
        // 400×300 图，票据位于 (100, 80) 尺寸 200×140
        let m = synth(400, 300, 100, 80, 200, 140);
        let out = autocrop(&m).expect("crop ok");
        // 裁后尺寸 ≥ 票据尺寸（外扩安全边），且 < 原图（真的裁了白边）
        assert!(out.cols() >= 200, "宽度切到内容: {}", out.cols());
        assert!(out.rows() >= 140, "高度切到内容: {}", out.rows());
        assert!(out.cols() < 400 && out.rows() < 300, "没有裁剪白边");
        // 内容完整：输出中深色像素的覆盖范围 ≥ 票据本体 200×140
        // （角上白边是外扩安全边的预期行为，与原实现内缩切内容相反）
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for yy in 0..out.rows() {
            for xx in 0..out.cols() {
                let p = *out.at_2d::<opencv::core::Vec3b>(yy, xx).expect("pixel");
                if p[0] < 128 {
                    min_x = min_x.min(xx);
                    max_x = max_x.max(xx);
                    min_y = min_y.min(yy);
                    max_y = max_y.max(yy);
                }
            }
        }
        let (dw, dh) = (max_x - min_x + 1, max_y - min_y + 1);
        assert!(dw >= 200, "深色内容宽度被裁: {dw} < 200");
        assert!(dh >= 140, "深色内容高度被裁: {dh} < 140");
    }

    /// 噪声/脏点不扩大 bbox（原实现全轮廓并集会被角落脏点撑大）
    #[test]
    fn autocrop_ignores_small_noise() {
        let mut m = synth(400, 300, 100, 80, 200, 140);
        // 右下角加一个 8×8 脏点（远离票据）
        fill_rect(&mut m, 380, 280, 8, 8);
        let out = autocrop(&m).expect("crop ok");
        // 脏点被忽略：裁剪结果只含票据 + 安全边（脏点在 380+，票据右界 300+pad≈306）
        assert!(out.cols() < 340, "脏点被算进 bbox: {}", out.cols());
    }

    /// 全图皆内容（无白边可裁，bbox ≈ 全图）→ 回退原图
    #[test]
    fn autocrop_full_content_falls_back() {
        let m = Mat::new_rows_cols_with_default(
            200,
            200,
            opencv::core::CV_8UC3,
            Scalar::new(30.0, 30.0, 30.0, 0.0),
        )
        .expect("new mat");
        let out = autocrop(&m).expect("ok");
        assert_eq!((out.cols(), out.rows()), (200, 200), "应回退原图");
    }

    /// 内容占比过小（< 10%，碎块误检）→ 回退原图不裁
    #[test]
    fn autocrop_tiny_content_falls_back() {
        let m = synth(400, 300, 190, 145, 20, 14); // 占比 0.23%
        let out = autocrop(&m).expect("ok");
        assert_eq!((out.cols(), out.rows()), (400, 300), "碎块应回退原图");
    }
}
