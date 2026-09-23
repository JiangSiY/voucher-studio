//! 界面显示口径的附件过滤。
//!
//! 背景（2026-09-17 缺陷）：备份把「增强版」当可重建缓存排除在外——`backup/local.rs` 只打包
//! DB 记录的原图（附件包里 120 个 `.jpg`、0 个 `.enhanced.jpg`），而恢复会整体替换数据库，
//! 于是恢复后 DB 的 `enhanced_path` 指向磁盘上并不存在的文件。
//! 界面按 `enhancedPath ?? filePath` 取图且不做存在性判断 → 「凭证信息正常、图片空白」；
//! 而 PDF 导出与 OCR 各自有 `.exists()` 回退（见 `services::export::pdf_image_abs`、
//! `services::ocr`），所以症状表现成「信息与导出都在，就是界面看不到图」。
//!
//! 这里把「增强版是否真的可显示」的判定收敛到一处，与导出/OCR 同口径：
//! **文件不存在或路径非法 → 下发给界面时按 `None` 处理**，前端自然回退原图。
//! 注意**不改 DB 存储**：`enhanced_path` 原值保留，「重新增强」仍覆盖同一文件。

use crate::models::Attachment;
use crate::CoreContext;

/// 该增强版相对路径是否指向真实存在的文件（空串 / 非法路径 / 文件缺失 → false）
pub fn enhanced_available(ctx: &CoreContext, rel: &str) -> bool {
    if rel.trim().is_empty() {
        return false;
    }
    match ctx.paths.attachment_abs_checked(rel) {
        Ok(abs) => {
            if abs.is_file() {
                true
            } else {
                log::warn!("增强版文件缺失，界面回退原图: {}", abs.display());
                false
            }
        }
        Err(_) => {
            log::warn!("增强版相对路径非法，界面回退原图: {rel:?}");
            false
        }
    }
}

/// 下发界面前的增强版路径：不可显示时返回 `None`
pub fn visible_enhanced(ctx: &CoreContext, rel: Option<&str>) -> Option<String> {
    let rel = rel?;
    enhanced_available(ctx, rel).then(|| rel.to_string())
}

/// 就地剥掉不可显示的增强版路径，并填好下发给界面的**绝对路径**
/// （编辑页附件列表用；`get_detail` 等场景统一走这里）。
///
/// 绝对路径由 Rust 侧用 `PathBuf` 拼（平台原生分隔符），前端只把它交给
/// `convertFileSrc`——前端自行拼路径曾在 macOS 上把 `\` 拼进路径，导致
/// asset protocol 拒绝全部图片请求（2026-09-23 缺陷）。
/// `file_abs` 只要相对路径合法就填：**不判存在性**，文件确实缺失时交给浏览器
/// 的「增强→原图→占位」回退阶梯处理，这样「原图在、增强缺失」仍能显示原图。
pub fn hide_missing_enhanced(ctx: &CoreContext, a: &mut Attachment) {
    a.file_abs = ctx.paths.abs_string_checked(&a.file_path);
    match a.enhanced_path.clone() {
        Some(rel) if enhanced_available(ctx, &rel) => {
            a.enhanced_abs = ctx.paths.abs_string_checked(&rel);
        }
        _ => {
            a.enhanced_path = None;
            a.enhanced_abs = None;
        }
    }
}

/// 批量版本（`voucher_mgr::get_detail` 等场景）
pub fn hide_missing_enhanced_all(ctx: &CoreContext, attachments: &mut [Attachment]) {
    for a in attachments.iter_mut() {
        hide_missing_enhanced(ctx, a);
    }
}
