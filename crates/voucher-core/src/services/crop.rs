//! v0.12：图片手动裁剪 / 旋转（自动矫正失败时的人工补救）。
//!
//! 语义：以**原图**为基准 → 旋转（90° 整数倍精确旋转；任意角度重采样、白底）→
//! 按框选裁剪 → 结果**覆盖写** `attachments/<id>.enhanced.jpg`。
//! 与自动增强共用「工作版本」语义（enhanced = 自动增强或手动调整后的版本；原图永不修改），
//! OCR / 查看器 / PDF 导出链路零改动（已按 enhanced_path 读取）。
//! 期间锁定检查由命令层守卫（换图口径——裁剪改变该月凭证的图片工作版本）。

use image::{DynamicImage, GenericImageView};

use crate::db::repos::attachment_repo;
use crate::models::Attachment;
use crate::{CoreContext, CoreError, Result};

/// 裁剪框（旋转后图像的像素坐标；左上为原点）
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// 手动裁剪附件：旋转 → 裁剪 → 覆盖写增强版 JPEG（质量 92），返回更新后的附件。
/// `rotation_deg` 任意整数（0 = 不旋转；±90/180/270 精确旋转；其余角度白底重采样）。
/// 裁剪框会被 clamp 到旋转后图像边界内；w/h 为 0 报错。
pub fn crop_attachment(
    ctx: &CoreContext,
    attachment_id: i64,
    rect: CropRect,
    rotation_deg: i32,
) -> Result<Attachment> {
    if rect.w == 0 || rect.h == 0 {
        return Err(CoreError::Invalid("裁剪区域不能为空".into()));
    }
    let attachment = ctx.db.with_conn(|conn| {
        attachment_repo::get(conn, attachment_id)?
            .ok_or_else(|| CoreError::NotFound(format!("附件 {attachment_id} 不存在")))
    })?;

    // v0.12：期间锁同换图口径（裁剪改的是锁定月凭证的工作图）
    let voucher_date = ctx.db.with_conn(|conn| {
        let v = crate::db::repos::voucher_repo::get(conn, attachment.voucher_id)?
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {} 不存在", attachment.voucher_id)))?;
        Ok(v.voucher_date)
    })?;
    ctx.db.with_conn(|conn| {
        crate::services::period::ensure_dates_open(conn, &[voucher_date.as_str()])
    })?;

    // 基准 = 原图（手动裁剪是对自动增强结果的替代，不是叠加）
    let src_abs = ctx.paths.attachment_abs(&attachment.file_path);
    let img =
        image::open(&src_abs).map_err(|e| CoreError::Invalid(format!("读取图片失败: {e}")))?;

    // 旋转（90° 倍数精确；其余角度白底重采样）
    let mut rotated = rotate_image(img, rotation_deg);

    // 裁剪（clamp 到边界；至少 8px 保证 JPEG 可编码）
    let (iw, ih) = rotated.dimensions();
    let x = rect.x.min(iw.saturating_sub(1));
    let y = rect.y.min(ih.saturating_sub(1));
    let w = rect.w.min(iw - x).max(8.min(iw - x));
    let h = rect.h.min(ih - y).max(8.min(ih - y));
    let cropped = rotated.crop(x, y, w, h);

    // 编码 JPEG（与自动增强同目录、同名覆盖；质量 92 略高于增强版 88——人工操作更怕质量损失）
    let mut jpeg = std::io::Cursor::new(Vec::new());
    cropped
        .to_rgb8()
        .write_to(&mut jpeg, image::ImageFormat::Jpeg)
        .map_err(|e| CoreError::Invalid(format!("JPEG 编码失败: {e}")))?;

    // 目标路径：<原图所在月>/<hash>.enhanced.jpg（与 write_enhanced 同约定）
    let rel = enhance_enhanced_rel(&attachment.file_path);
    let dest_abs = ctx.paths.attachment_abs(&rel);
    std::fs::write(&dest_abs, jpeg.into_inner())?;

    // 更新附件行（enhanced_path 指向手动版；来源分类保持不变——它描述的是图片性质）
    ctx.db
        .with_conn(|conn| attachment_repo::set_enhanced(conn, attachment_id, &rel))?;
    let mut updated = attachment;
    updated.enhanced_path = Some(rel);
    Ok(updated)
}

/// 原图 rel → 增强版 rel（同目录、同名 .enhanced.jpg）
fn enhance_enhanced_rel(orig_rel: &str) -> String {
    let p = std::path::Path::new(orig_rel);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("img");
    match p.parent().and_then(|d| d.to_str()) {
        Some(dir) if !dir.is_empty() => format!("{dir}/{stem}.enhanced.jpg"),
        _ => format!("{stem}.enhanced.jpg"),
    }
}

/// 旋转：90° 整数倍走精确旋转（无损尺寸互换）；其余角度白底重采样（最近邻，逆映射）。
/// 约定：正角度 = 顺时针（与 CSS / Canvas transform 一致，前端预览用同一约定即可对齐）。
fn rotate_image(img: DynamicImage, deg: i32) -> DynamicImage {
    let norm = deg.rem_euclid(360);
    match norm {
        0 => img,
        90 => img.rotate90(),
        180 => img.rotate180(),
        270 => img.rotate270(),
        _ => {
            let theta = (norm as f32).to_radians();
            let (c, s) = (theta.cos(), theta.sin());
            let (w, h) = img.dimensions();
            let out_w = ((w as f32 * c.abs() + h as f32 * s.abs()).ceil() as u32).max(1);
            let out_h = ((w as f32 * s.abs() + h as f32 * c.abs()).ceil() as u32).max(1);
            let rgba = img.to_rgba8();
            let mut out =
                image::RgbaImage::from_pixel(out_w, out_h, image::Rgba([255, 255, 255, 255]));
            let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
            let (ox, oy) = (out_w as f32 / 2.0, out_h as f32 / 2.0);
            for y in 0..out_h {
                for x in 0..out_w {
                    // 目标像素 → 源坐标（顺时针旋转的逆映射：src = M^T * dest）
                    let dx = x as f32 - ox + 0.5;
                    let dy = y as f32 - oy + 0.5;
                    let sx = c * dx + s * dy + cx;
                    let sy = -s * dx + c * dy + cy;
                    if sx >= 0.0 && sy >= 0.0 && sx < w as f32 && sy < h as f32 {
                        if let Some(px) = rgba.get_pixel_checked(sx as u32, sy as u32) {
                            out.put_pixel(x, y, *px);
                        }
                    }
                }
            }
            DynamicImage::ImageRgba8(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhanced_rel_convention() {
        assert_eq!(
            enhance_enhanced_rel("2026/08/abcd1234.jpg"),
            "2026/08/abcd1234.enhanced.jpg"
        );
        assert_eq!(enhance_enhanced_rel("a.png"), "a.enhanced.jpg");
    }
}
