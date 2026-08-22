//! 文档扫描矫正与增强：流水线编排（5.5 方案）
//!
//! B 步实装：5 步串行 + 单步失败跳过 + 最终回退原图。
//!
//! 处理流程（决策 ④：单步失败跳过，不抛错到调用方）：
//!   1. 倾斜矫正（enhance_geom::deskew）
//!   2. 透视矫正（enhance_geom::dewarp）
//!   3. 自动裁边（enhance_geom::autocrop）
//!   4. 灰度化（enhance_pixel::to_grayscale）
//!   5. 自适应二值化（enhance_pixel::binarize_sauvola）
//!
//! 步骤 1-3 维持彩色图 → mat_color（PDF 导出/查看器用）
//! 步骤 4-5 走灰度/二值化 → mat_ocr（OCR 用）

use std::path::Path;

use opencv::{
    core::{Mat, Vector},
    imgcodecs::{imencode, imread, ImreadModes},
    prelude::*,
};

use crate::{CoreError, Result};

use super::{enhance_geom, enhance_pixel};
/// 流水线输出。
#[derive(Debug, Clone)]
pub struct PipelineOutput {
    /// 第 1-3 步后的彩色图：用于查看器、PDF 导出。
    pub mat_color: Mat,
    /// 第 1-5 步后的二值图：用于 OCR（更易识别）。
    pub mat_ocr: Mat,
    /// 实际走过的步骤描述（调试/日志用）。
    pub trace: Vec<String>,
}

/// 流水线入口：读图 → 走 5 步 → 返回双输出。
///
/// 单步失败绝不抛错（决策 ④），而是用上一可用步的输出作为该步的输入继续。
pub fn run_pipeline(src_path: &Path) -> Result<PipelineOutput> {
    if !src_path.exists() {
        return Err(CoreError::Invalid(format!(
            "enhance: 文件不存在 {}",
            src_path.display()
        )));
    }

    let src = imread(src_path.to_string_lossy().as_ref(), ImreadModes::IMREAD_COLOR as i32)
        .map_err(|e| CoreError::Invalid(format!("enhance: 读图失败: {e}")))?;
    if src.empty() {
        return Err(CoreError::Invalid("enhance: 读图结果为空".into()));
    }

    let mut trace: Vec<String> = Vec::new();
    let mut current = src;

    current = step(current, "deskew", &mut trace, |m| enhance_geom::deskew(m))?;
    current = step(current, "dewarp", &mut trace, |m| enhance_geom::dewarp(m))?;
    current = step(current, "autocrop", &mut trace, |m| enhance_geom::autocrop(m))?;

    // 步骤 1-3 后：保存彩色输出
    let mat_color = current
        .try_clone()
        .map_err(|e| CoreError::Invalid(e.to_string()))?;

    // 步骤 4：灰度化
    let mat_gray = step(current, "grayscale", &mut trace, |m| enhance_pixel::to_grayscale(m))?;
    // 步骤 5：自适应二值化
    let mat_ocr = step(mat_gray, "binarize_sauvola", &mut trace, enhance_pixel::binarize_sauvola)?;

    Ok(PipelineOutput {
        mat_color,
        mat_ocr,
        trace,
    })
}

/// 步骤包装：单步失败跳过。
fn step<F>(mut current: Mat, name: &str, trace: &mut Vec<String>, f: F) -> Result<Mat>
where
    F: FnOnce(&Mat) -> Result<Mat>,
{
    match f(&current) {
        Ok(next) => {
            trace.push(format!("{name}:ok"));
            current = next;
        }
        Err(e) => {
            trace.push(format!("{name}:skip({})", e));
        }
    }
    Ok(current)
}

/// 把彩色 Mat 编码为 JPEG 字节流（用于存储/传输）。
pub fn encode_color_jpeg(mat: &Mat, quality: i32) -> Result<Vec<u8>> {
    let mut buf = Vector::<u8>::new();
    let params = Vector::<i32>::from_iter(vec![
        opencv::imgcodecs::IMWRITE_JPEG_QUALITY,
        quality,
    ]);
    imencode(".jpg", mat, &mut buf, &params)
        .map_err(|e| CoreError::Invalid(format!("imencode jpeg 失败: {e}")))?;
    Ok(buf.to_vec())
}

/// 把灰度 Mat 编码为 PNG 字节流（无损，OCR 用）。
pub fn encode_gray_png(mat: &Mat) -> Result<Vec<u8>> {
    let mut buf = Vector::<u8>::new();
    let params = Vector::<i32>::new();
    imencode(".png", mat, &mut buf, &params)
        .map_err(|e| CoreError::Invalid(format!("imencode png 失败: {e}")))?;
    Ok(buf.to_vec())
}

/// 检测图片来源（截图 vs 手机拍照/扫描），决定增强版用途。
pub fn classify_source(path: &Path) -> SourceKind {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    if ext != "png" {
        return SourceKind::PhotoOrScan;
    }

    let size_ok = std::fs::metadata(path)
        .map(|m| m.len() < 500 * 1024)
        .unwrap_or(false);

    if size_ok {
        if let Ok((w, h)) = crate::services::imaging::probe_dimensions(path) {
            let ratio = w as f32 / h.max(1) as f32;
            let screen_ratios = [4.0 / 3.0, 16.0 / 9.0, 9.0 / 16.0];
            if screen_ratios.iter().any(|r| (ratio - r).abs() / r < 0.15) {
                return SourceKind::Screenshot;
            }
        }
    }

    SourceKind::PhotoOrScan
}

/// 图片来源分类（决定增强版是否用于 PDF 导出）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// 屏幕截图：增强版仅给 OCR，不用于 PDF 导出
    Screenshot,
    /// 手机拍照/扫描：增强版用于 OCR + PDF 导出
    PhotoOrScan,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Screenshot => "screenshot",
            SourceKind::PhotoOrScan => "photo_or_scan",
        }
    }
}

/// C 步：把增强版落地为磁盘 JPEG，写入 `attachments/<yyyy>/<mm>/<hash>.enhanced.jpg`。
///
/// 返回增强版相对路径（与原图同月分目录，命名加 `.enhanced` 后缀）。
///
/// 任何失败（读图失败、流水线异常、编码失败、写盘失败）一律不抛错给调用方——
/// 返回 `None`，由 ingestion 继续走原图入库（决策 ④ 失败兜底）。
pub fn write_enhanced(
    src_abs: &Path,
    rel_base: &str,
) -> Option<String> {
    // 1. 跑流水线（任一步失败内部已 skip，不抛错）
    let out = match run_pipeline(src_abs) {
        Ok(o) => o,
        Err(e) => {
            log::warn!("增强流水线启动失败: {e}");
            return None;
        }
    };

    // 2. 编码 mat_color 为 JPEG 字节流
    let jpeg = match encode_color_jpeg(&out.mat_color, 88) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("增强 JPEG 编码失败: {e}");
            return None;
        }
    };

    // 3. 算目标路径：<原图所在月>/<原图 hash>.enhanced.jpg
    //    rel_base 形如 "2026/08/abcd1234.jpg"，取出 yyyy/mm + hash + .enhanced.jpg
    let (year_month, hash_stem) = parse_rel(rel_base)?;
    let new_name = format!("{}.enhanced.jpg", hash_stem);
    let rel = format!("{}/{}", year_month, new_name);

    // 4. 写盘（父目录已存在；复制阶段已 create_dir_all）
    if let Some(parent) = src_abs.parent() {
        let dest_abs = parent.join(&new_name);
        if let Err(e) = std::fs::write(&dest_abs, &jpeg) {
            log::warn!("增强版写盘失败 {}: {e}", dest_abs.display());
            return None;
        }
        return Some(rel);
    }
    None
}

/// 解析 rel 路径为 (yyyy/mm, hash_stem)  —— 仅依赖 `Path` 工具
fn parse_rel(rel: &str) -> Option<(String, String)> {
    let p = std::path::Path::new(rel);
    let year_month = p.parent()?.to_str()?.to_string();
    let stem = p.file_stem()?.to_str()?.to_string();
    Some((year_month, stem))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn classify_jpeg_is_photo() {
        let p = PathBuf::from("test.jpg");
        assert_eq!(classify_source(&p), SourceKind::PhotoOrScan);
    }

    #[test]
    fn classify_bmp_is_photo() {
        let p = PathBuf::from("scan.bmp");
        assert_eq!(classify_source(&p), SourceKind::PhotoOrScan);
    }

    #[test]
    fn pipeline_missing_file_errors() {
        let p = PathBuf::from("nonexistent_enhance_test_zzz.jpg");
        assert!(run_pipeline(&p).is_err());
    }

    /// B 步实装验证：5 步流水线跑一张合成图
    /// - 用 OpenCV 画一张 800x600 的"含文字"的合成图（白底黑矩形模拟文档）
    /// - 跑 5 步流水线
    /// - 断言：mat_color/mat_ocr 都有非零尺寸，trace 至少包含 5 步
    #[test]
    fn pipeline_runs_synthetic_document() {
        use opencv::{
            core::{Mat, Scalar, Size},
            imgproc::{put_text, rectangle, line, FONT_HERSHEY_SIMPLEX, LineTypes},
        };
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let img_path = dir.path().join("synth_doc.png");

        // 构造 800x600 BGR 合成图：白底 + 黑色矩形框 + 文字 + 若干水平线
        let mut src = unsafe {
            Mat::new_size(Size::new(800, 600), opencv::core::CV_8UC3)
        }.expect("new_size");
        // 全白填充
        src.set_to_def(&Scalar::new(255.0, 255.0, 255.0, 0.0))
            .expect("set_to");

        // 画一个文档矩形
        rectangle(
            &mut src,
            opencv::core::Rect::new(80, 60, 640, 480),
            Scalar::new(0.0, 0.0, 0.0, 0.0),
            3,
            LineTypes::LINE_8 as i32,
            0,
        )
        .expect("rect");

        // 写"凭证 NO.001"文字（作为 Hough 检测的强信号）
        put_text(
            &mut src,
            "VOUCHER NO 001",
            opencv::core::Point::new(120, 120),
            FONT_HERSHEY_SIMPLEX as i32,
            1.0,
            Scalar::new(0.0, 0.0, 0.0, 0.0),
            2,
            LineTypes::LINE_8 as i32,
            false,
        )
        .expect("put_text");

        // 画 5 条水平线（让 deskew 能找到角度）
        for i in 0..5 {
            let y = 200 + i * 60;
            line(
                &mut src,
                opencv::core::Point::new(120, y),
                opencv::core::Point::new(700, y),
                Scalar::new(0.0, 0.0, 0.0, 0.0),
                2,
                LineTypes::LINE_8 as i32,
                0,
            )
            .expect("line");
        }

        // 编码写入磁盘
        let mut buf = opencv::core::Vector::<u8>::new();
        opencv::imgcodecs::imencode(".png", &src, &mut buf, &opencv::core::Vector::<i32>::new())
            .expect("imencode");
        std::fs::write(&img_path, buf.to_vec()).expect("write png");

        // 跑 5 步流水线
        let out = run_pipeline(&img_path).expect("pipeline ok");

        // 断言：彩色图、二值图都有非零尺寸
        assert!(!out.mat_color.empty(), "mat_color 不应为空");
        assert!(!out.mat_ocr.empty(), "mat_ocr 不应为空");
        assert_eq!(out.mat_color.channels(), 3, "mat_color 应保持 3 通道");
        assert_eq!(out.mat_ocr.channels(), 1, "mat_ocr 应为单通道二值图");

        // 断言：trace 至少记录 5 步（允许部分 skip）
        assert!(out.trace.len() >= 5, "trace 应至少 5 条，实际 {} 条", out.trace.len());
        eprintln!("5 步流水线 trace = {:?}", out.trace);
    }
}
