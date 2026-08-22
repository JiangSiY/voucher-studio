//! OCR 服务（对应 v4.3 的 `ocr.py`）
//!
//! - 本地（默认）：ort + RapidOCR PP-OCRv4，完全离线（G1 Spike B 实测字符准确率 99.9%）
//! - 云端：通用 HTTP API（配置地址 + API Key 走系统凭据库）
//! - 结果以附件内容哈希为键落缓存（ocr_cache 表），同一张图不重复识别

use std::path::Path;
use std::time::Duration;

use base64::Engine;
use rusqlite::Connection;

use crate::db::repos::{attachment_repo, enhance_hit_repo, ocr_cache_repo};
use crate::models::{Attachment, OcrStatus};
use crate::utils::config::SECRET_OCR_API_KEY;
use crate::{CoreContext, CoreError, Result};

/// 识别单个附件；命中缓存直接返回
pub fn run_for_attachment(ctx: &CoreContext, attachment_id: i64) -> Result<Attachment> {
    let attachment = ctx.db.with_conn(|conn| {
        attachment_repo::get(conn, attachment_id)?
            .ok_or_else(|| CoreError::NotFound(format!("附件 {attachment_id} 不存在")))
    })?;

    if !attachment.is_image() {
        return Err(CoreError::Invalid("仅支持识别图片附件".into()));
    }

    let abs = ctx.paths.attachment_abs(&attachment.file_path);
    if !abs.exists() {
        return Err(CoreError::NotFound(format!(
            "附件文件缺失: {}",
            abs.display()
        )));
    }

    // v0.6 C 步：优先用增强版（5 步算法后的彩色 JPEG）做 OCR；
    // 增强版不存在（未增强 / 失败）时回退原图。
    // v0.6.5 D-2：使用增强版时记一条 enhance_hits 命中（OCR 来源），给设置页命中率统计。
    let ocr_target = if let Some(ep) = &attachment.enhanced_path {
        let enhanced_abs = ctx.paths.attachment_abs(ep);
        if enhanced_abs.exists() {
            if let Err(e) = ctx.db.with_conn(|conn| {
                enhance_hit_repo::record(conn, attachment.id, enhance_hit_repo::HitSource::Ocr)
            }) {
                log::warn!("记录 enhance_hits 失败（不影响 OCR）: {e}");
            }
            enhanced_abs
        } else {
            log::warn!("增强版文件缺失，回退原图: {}", enhanced_abs.display());
            abs.clone()
        }
    } else {
        abs.clone()
    };

    let text = ctx
        .db
        .with_conn(|conn| recognize_with_cache(conn, ctx, &attachment, &ocr_target))?;

    ctx.db.with_conn(|conn| {
        attachment_repo::set_ocr(conn, attachment.id, OcrStatus::Done, Some(&text))?;
        attachment_repo::get(conn, attachment.id)?
            .ok_or_else(|| CoreError::Invalid("附件更新后读取失败".into()))
    })
}

/// 批量识别待处理附件，返回成功数（失败的逐个记为 failed，不中断批次）。
/// 识别成功后自动填单（Manual 模式：只填空字段，不覆盖已录入内容）。
/// `on_progress` 每处理完一张回调 (已完成数, 总数)，供上层做进度反馈（传 `&|_, _| {}` 忽略）。
pub fn run_pending(
    ctx: &CoreContext,
    limit: u32,
    on_progress: &dyn Fn(usize, usize),
) -> Result<usize> {
    let pending = ctx
        .db
        .with_conn(|conn| attachment_repo::list_pending_ocr(conn, limit))?;
    let total = pending.len();
    let mut ok = 0;
    for (i, a) in pending.into_iter().enumerate() {
        match run_for_attachment(ctx, a.id) {
            Ok(att) => {
                if let Err(e) = crate::services::ocr_fields::apply_for_attachment(
                    ctx,
                    att.voucher_id,
                    att.id,
                    crate::services::ocr_fields::FillMode::Manual,
                ) {
                    log::warn!("OCR 自动填单失败（凭证 {}）: {e}", att.voucher_id);
                }
                ok += 1;
            }
            Err(e) => {
                log::warn!("OCR 失败（附件 {}）: {e}", a.id);
                let _ = ctx.db.with_conn(|conn| {
                    attachment_repo::set_ocr(conn, a.id, OcrStatus::Failed, None)
                });
            }
        }
        on_progress(i + 1, total);
    }
    Ok(ok)
}

fn recognize_with_cache(
    conn: &Connection,
    ctx: &CoreContext,
    attachment: &Attachment,
    abs: &Path,
) -> Result<String> {
    if let Some((engine, text)) = ocr_cache_repo::get(conn, &attachment.file_hash)? {
        let _ = engine;
        return Ok(text);
    }

    let cfg = ctx.config();
    if !cfg.ocr.enabled {
        return Err(CoreError::Invalid("OCR 未启用（设置 → OCR）".into()));
    }

    let text = match cfg.ocr.engine.as_str() {
        "cloud" => cloud_recognize(ctx, abs)?,
        "local" => local_recognize(ctx, abs)?,
        other => return Err(CoreError::Invalid(format!("未知 OCR 引擎: {other}"))),
    };

    ocr_cache_repo::upsert(conn, &attachment.file_hash, &cfg.ocr.engine, &text)?;
    Ok(text)
}

/// 本地识别（G1 Spike B 验证路线：ort + RapidOCR PP-OCRv4，完全离线）
fn local_recognize(ctx: &CoreContext, image_path: &Path) -> Result<String> {
    let engine = ctx.local_ocr()?;
    let decoded = image::io::Reader::open(image_path)
        .map_err(|e| e.to_string())
        .and_then(|r| r.decode().map_err(|e| e.to_string()));
    let img = decoded
        .map_err(|e| CoreError::LocalOcr(format!("读取图片失败 {}: {e}", image_path.display())))?;
    let lines = engine.lock().unwrap().recognize_image(&img)?;
    Ok(lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n"))
}

/// 云端识别：POST {image_base64, model} → {text}（兼容 data.text / result 字段）
fn cloud_recognize(ctx: &CoreContext, image_path: &Path) -> Result<String> {
    let cfg = ctx.config();
    let api_url = cfg.ocr.api_url.trim().to_string();
    if api_url.is_empty() {
        return Err(CoreError::Invalid(
            "云端 OCR API 地址未配置（设置 → OCR）".into(),
        ));
    }

    let bytes = std::fs::read(image_path)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;

    let mut req = client.post(&api_url).json(&serde_json::json!({
        "image_base64": b64,
        "model": cfg.ocr.model,
    }));
    if let Some(key) = ctx.secrets.get(SECRET_OCR_API_KEY)? {
        if !key.is_empty() {
            req = req.bearer_auth(key);
        }
    }

    let resp: serde_json::Value = req.send()?.error_for_status()?.json()?;
    extract_text(&resp)
        .ok_or_else(|| CoreError::Invalid("OCR 服务返回格式无法解析（缺少 text 字段）".into()))
}

/// 常见返回格式逐层取文本（某条路径不存在时继续尝试下一条，而非提前退出）
fn extract_text(v: &serde_json::Value) -> Option<String> {
    for path in ["text", "data.text", "result", "data.result", "result.text"] {
        if let Some(cur) = path.split('.').try_fold(v, |acc, k| acc.get(k)) {
            if let Some(s) = cur.as_str() {
                if !s.trim().is_empty() {
                    return Some(s.trim().to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_supports_common_shapes() {
        let a = serde_json::json!({"text": "发票 100 元"});
        assert_eq!(extract_text(&a).unwrap(), "发票 100 元");

        let b = serde_json::json!({"data": {"text": "收据"}});
        assert_eq!(extract_text(&b).unwrap(), "收据");

        let c = serde_json::json!({"result": "出租车票"});
        assert_eq!(extract_text(&c).unwrap(), "出租车票");

        let d = serde_json::json!({"foo": 1});
        assert!(extract_text(&d).is_none());
    }
}
