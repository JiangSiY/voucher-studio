//! 图片导入流水线（对应 v4.3 的 drop 拖放 / wechat 监听共用的入库逻辑）
//!
//! 流程：扩展名校验 → MD5 去重 → 复制进附件目录（yyyy/mm/哈希.扩展名）→ 落库 → 可选 OCR

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::db::repos::attachment_repo;
use crate::models::{Attachment, NewAttachment, NewVoucher};
use crate::services::{dedup, imaging, ocr, voucher_mgr};
use crate::{CoreContext, CoreError, Result};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    /// 导入后所属凭证（新草稿或指定凭证）
    pub voucher_id: i64,
    pub voucher_no: String,
    pub imported: Vec<Attachment>,
    /// 内容重复被跳过的文件名
    pub skipped_duplicates: Vec<String>,
    pub failed: Vec<ImportFailure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFailure {
    pub file_name: String,
    pub reason: String,
}

/// 导入图片。`voucher_id` 为 None 时自动新建一张草稿凭证。
/// `run_ocr`：导入后立即做 OCR（失败不影响入库）。
pub fn import_images(
    ctx: &CoreContext,
    paths: &[PathBuf],
    voucher_id: Option<i64>,
    run_ocr: bool,
) -> Result<ImportSummary> {
    if paths.is_empty() {
        return Err(CoreError::Invalid("未选择任何文件".into()));
    }

    // 确定目标凭证
    let voucher = match voucher_id {
        Some(id) => {
            let detail = voucher_mgr::get_detail(ctx, id)?;
            detail.voucher
        }
        None => {
            let first_name = paths[0].file_stem().and_then(|s| s.to_str()).unwrap_or("");
            voucher_mgr::create_voucher(
                ctx,
                NewVoucher {
                    description: first_name.to_string(),
                    ..Default::default()
                },
            )?
        }
    };

    let year_month = crate::utils::paths::current_year_month();
    let dir = ctx.paths.attachment_dir_for(&year_month);
    std::fs::create_dir_all(&dir)?;

    let mut summary = ImportSummary {
        voucher_id: voucher.id,
        voucher_no: voucher.voucher_no.clone(),
        ..Default::default()
    };

    // 逐文件：去重 + 复制 + 建行
    let mut rows: Vec<NewAttachment> = Vec::new();
    for path in paths {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        if !imaging::is_supported(path) {
            summary.failed.push(ImportFailure {
                reason: "不支持的文件类型（仅图片）".into(),
                file_name,
            });
            continue;
        }
        if !path.is_file() {
            summary.failed.push(ImportFailure {
                reason: "文件不存在".into(),
                file_name,
            });
            continue;
        }

        let hash = match dedup::md5_file(path) {
            Ok(h) => h,
            Err(e) => {
                summary.failed.push(ImportFailure {
                    reason: e.to_string(),
                    file_name,
                });
                continue;
            }
        };

        let dup = ctx
            .db
            .with_conn(|conn| dedup::find_duplicate(conn, &hash))?;
        if dup.is_some() {
            summary.skipped_duplicates.push(file_name);
            continue;
        }

        let ext = imaging::ext_of(path);
        let rel = ctx.paths.attachment_rel_path(&hash, &ext, &year_month);
        let dest = ctx.paths.attachment_abs(&rel);
        if let Err(e) = std::fs::copy(path, &dest) {
            summary.failed.push(ImportFailure {
                reason: format!("复制失败: {e}"),
                file_name,
            });
            continue;
        }
        let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);

        rows.push(NewAttachment {
            voucher_id: voucher.id,
            file_name,
            file_path: rel,
            file_hash: hash,
            file_size: size,
            mime_type: imaging::mime_from_ext(&ext).to_string(),
        });
    }

    // 附件行入库（单事务）
    if !rows.is_empty() {
        ctx.db.with_tx(|tx| {
            for r in &rows {
                summary.imported.push(attachment_repo::insert(tx, r)?);
            }
            Ok(())
        })?;
    }

    // 可选 OCR（尽力而为）
    if run_ocr {
        for a in &summary.imported {
            if let Err(e) = ocr::run_for_attachment(ctx, a.id) {
                log::warn!("导入后 OCR 失败（附件 {}）: {e}", a.id);
            }
        }
    }

    Ok(summary)
}

/// 删除附件（行 + 磁盘文件）
pub fn delete_attachment(ctx: &CoreContext, attachment_id: i64) -> Result<()> {
    let attachment = ctx.db.with_conn(|conn| {
        attachment_repo::get(conn, attachment_id)?
            .ok_or_else(|| CoreError::NotFound(format!("附件 {attachment_id} 不存在")))
    })?;

    ctx.db
        .with_conn(|conn| attachment_repo::delete(conn, attachment_id))?;

    let abs = ctx.paths.attachment_abs(&attachment.file_path);
    if ctx.paths.is_inside_attachments(&abs) {
        let _ = std::fs::remove_file(&abs);
        // 清理可能空掉的月份目录
        if let Some(parent) = abs.parent() {
            if parent.starts_with(&ctx.paths.attachments) {
                let _ = try_remove_empty_dir(parent);
            }
        }
    }
    Ok(())
}

fn try_remove_empty_dir(dir: &Path) -> std::io::Result<()> {
    let is_empty = dir.read_dir()?.next().is_none();
    if is_empty {
        std::fs::remove_dir(dir)?;
    }
    Ok(())
}
