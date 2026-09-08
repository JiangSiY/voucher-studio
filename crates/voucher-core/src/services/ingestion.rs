//! 图片导入流水线（拖放 / 微信监听共用的入库逻辑）
//!
//! 一图一凭证：每张成功入库的图片各建一张凭证（建卡延迟到文件入库成功之后，
//! 重复/失败文件不产生空凭证——根治 G3-3 空卡缺陷）。
//! 流程（每文件独立）：扩展名校验 → MD5 去重 → 复制进附件目录 → 凭证+附件单事务落库 → 可选 OCR 自动填单。

use std::collections::HashSet;
use std::path::PathBuf;

use serde::Serialize;

use crate::db::repos::attachment_repo;
use crate::models::{Attachment, EnhanceSource, NewAttachment, NewVoucher};
use crate::services::{dedup, enhance, imaging, naming, ocr, ocr_fields, voucher_mgr};
use crate::{CoreContext, CoreError, Result};

/// 一次导入产生的一张凭证（一张图片）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedVoucher {
    pub voucher_id: i64,
    pub voucher_no: String,
    pub attachment: Attachment,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    /// 本次导入创建的凭证（每张图片一张）
    pub vouchers: Vec<ImportedVoucher>,
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

/// 导入图片。`voucher_id` 为 None 时每张图片各建一张草稿凭证（一图一凭证）；
/// 为 Some 时附加到指定凭证——仅当该凭证当前无图片（一图上限）。
/// `run_ocr`：导入后立即 OCR 并自动填单（失败不影响入库）。
pub fn import_images(
    ctx: &CoreContext,
    paths: &[PathBuf],
    voucher_id: Option<i64>,
    run_ocr: bool,
) -> Result<ImportSummary> {
    if paths.is_empty() {
        return Err(CoreError::Invalid("未选择任何文件".into()));
    }

    // 附加模式前置校验：目标凭证必须无图（一图上限）
    if let Some(id) = voucher_id {
        let detail = voucher_mgr::get_detail(ctx, id)?;
        if !detail.attachments.is_empty() {
            return Err(CoreError::Invalid(
                "该凭证已有图片，请使用「更换图片」".into(),
            ));
        }
        // v0.11 期间锁：已封账月份的凭证不可附加图片
        ctx.db.with_conn(|conn| {
            crate::services::period::ensure_dates_open(
                conn,
                &[detail.voucher.voucher_date.as_str()],
            )
        })?;
    }

    let year_month = crate::utils::paths::current_year_month();
    std::fs::create_dir_all(ctx.paths.attachment_dir_for(&year_month))?;

    let cfg = ctx.config();
    let engine = naming::NamingEngine::new(&cfg.naming.template);

    let mut summary = ImportSummary::default();
    // 批内已见内容哈希：DB 查重看不到本批尚未入库的行，需内存去重
    let mut seen_hashes: HashSet<String> = HashSet::new();
    // 附加模式下首图入库后凭证已满，后续文件全部失败（不会转去新建凭证）
    let mut attach_done = false;

    for path in paths {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        if voucher_id.is_some() && attach_done {
            summary.failed.push(ImportFailure {
                reason: "该凭证已有图片（一图一凭证），仅第一张被导入".into(),
                file_name,
            });
            continue;
        }
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

        let dup = seen_hashes.contains(&hash)
            || ctx
                .db
                .with_conn(|conn| dedup::find_duplicate(conn, &hash))?
                .is_some();
        if dup {
            summary.skipped_duplicates.push(file_name);
            continue;
        }
        seen_hashes.insert(hash.clone());

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

        // v0.6 C 步：增强版入库（仅在「扫描件」来源时跑；总开关 / only_scans 控制）
        // 失败兜底：返回 None → 走原图，不影响主流程
        let cfg = ctx.config();
        let enhance_cfg = &cfg.enhance;
        let source = enhance::classify_source(path);
        let only_scans = enhance_cfg.only_scans;
        let enhanced_rel =
            if enhance_cfg.enabled && !(only_scans && source == enhance::SourceKind::Screenshot) {
                // v0.6.1：逐项步骤开关映射进流水线（关闭的步骤记 off 并透传）
                let opts = enhance::PipelineOptions {
                    deskew: enhance_cfg.step_deskew,
                    dewarp: enhance_cfg.step_dewarp,
                    autocrop: enhance_cfg.step_autocrop,
                    denoise: enhance_cfg.step_denoise,
                    sharpen: enhance_cfg.step_sharpen,
                    grayscale: enhance_cfg.step_grayscale,
                    binarize: enhance_cfg.step_binarize,
                };
                enhance::write_enhanced(&dest, &rel, &opts)
            } else {
                None
            };

        // 凭证 + 附件行同一事务（文件已复制成功才建卡——无空卡）
        let created = import_one(
            ctx,
            &engine,
            voucher_id,
            file_name.clone(),
            rel.clone(),
            hash,
            size,
            &ext,
        );
        match created {
            Ok((created_voucher_id, voucher_no, attachment)) => {
                if voucher_id.is_some() {
                    attach_done = true; // 附加模式：该凭证已满
                }
                // v0.6 C 步：写入增强版路径 + 来源分类（事务外；失败仅告警）
                if enhanced_rel.is_some() {
                    if let Err(e) = ctx.db.with_conn(|conn| {
                        attachment_repo::set_enhanced(
                            conn,
                            attachment.id,
                            enhanced_rel.as_ref().unwrap(),
                        )
                    }) {
                        log::warn!("写入增强版路径失败（附件 {}）: {e}", attachment.id);
                    }
                }
                let db_source = match source {
                    enhance::SourceKind::Screenshot => EnhanceSource::Screenshot,
                    enhance::SourceKind::PhotoOrScan => EnhanceSource::Scan,
                };
                if let Err(e) = ctx.db.with_conn(|conn| {
                    attachment_repo::set_enhanced_source(conn, attachment.id, db_source)
                }) {
                    log::warn!("写入增强来源失败（附件 {}）: {e}", attachment.id);
                }

                // 反射回 attachment：把增强字段也带出来
                let mut attachment_out = attachment;
                attachment_out.enhanced_path = enhanced_rel.clone();
                attachment_out.enhanced_source = db_source;

                summary.vouchers.push(ImportedVoucher {
                    voucher_id: created_voucher_id,
                    voucher_no,
                    attachment: attachment_out,
                });
            }
            Err(e) => {
                // 事务失败：清理刚复制的文件 + 增强版文件，不留孤儿
                let _ = std::fs::remove_file(&dest);
                if let Some(er) = &enhanced_rel {
                    let _ = std::fs::remove_file(ctx.paths.attachment_abs(er));
                }
                summary.failed.push(ImportFailure {
                    reason: e.to_string(),
                    file_name,
                });
            }
        }
    }

    // 可选 OCR + 自动填单 + 重命名（尽力而为，失败仅告警）
    // v0.6 C 步：OCR 服务内部已读 attachments.enhanced_path，存在则用增强图、否则回退原图
    if run_ocr {
        for v in &summary.vouchers {
            match ocr::run_for_attachment(ctx, v.attachment.id) {
                Ok(att) => {
                    if let Err(e) = ocr_fields::apply_for_attachment(
                        ctx,
                        v.voucher_id,
                        att.id,
                        ocr_fields::FillMode::Import,
                    ) {
                        log::warn!("OCR 自动填单失败（凭证 {}）: {e}", v.voucher_id);
                    }
                }
                Err(e) => log::warn!("导入后 OCR 失败（附件 {}）: {e}", v.attachment.id),
            }
        }
    }

    Ok(summary)
}

/// 单文件落库：新建凭证（命名引擎取号）或挂到既有凭证，返回 (凭证 id, 编号, 附件)
#[allow(clippy::too_many_arguments)]
fn import_one(
    ctx: &CoreContext,
    engine: &naming::NamingEngine,
    attach_to: Option<i64>,
    file_name: String,
    rel: String,
    hash: String,
    size: u64,
    ext: &str,
) -> Result<(i64, String, Attachment)> {
    let today = crate::utils::paths::today();
    let file_stem = file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_else(|| file_name.clone());

    ctx.db.with_tx(|tx| {
        let (voucher_id, voucher_no) = match attach_to {
            Some(id) => {
                let v = crate::db::repos::voucher_repo::get(tx, id)?
                    .ok_or_else(|| CoreError::NotFound(format!("凭证 {id} 不存在")))?;
                (id, v.voucher_no)
            }
            None => {
                let voucher_no = engine.next_no(tx, &today)?;
                let v = crate::db::repos::voucher_repo::insert(
                    tx,
                    &NewVoucher {
                        voucher_no,
                        voucher_date: today.clone(),
                        description: file_stem.clone(),
                        counterparty: String::new(),
                        amount_cents: None,
                        status: None,
                        tags: Vec::new(),
                        note: String::new(),
                    },
                )?;
                (v.id, v.voucher_no)
            }
        };
        let attachment = attachment_repo::insert(
            tx,
            &NewAttachment {
                voucher_id,
                file_name,
                file_path: rel,
                file_hash: hash,
                file_size: size,
                mime_type: imaging::mime_from_ext(ext).to_string(),
            },
        )?;
        Ok((voucher_id, voucher_no, attachment))
    })
}

/// 更换凭证图片：删旧图挂新图（单事务 + 快照），旧物理文件随后清理。
/// 新图内容与库中其他凭证图片重复时拒绝；与当前图片内容相同视为幂等成功（不换）。
/// 返回新附件行。
pub fn replace_attachment(
    ctx: &CoreContext,
    voucher_id: i64,
    path: &PathBuf,
) -> Result<Attachment> {
    if !imaging::is_supported(path) {
        return Err(CoreError::Invalid("不支持的文件类型（仅图片）".into()));
    }
    if !path.is_file() {
        return Err(CoreError::Invalid("文件不存在".into()));
    }

    let old = ctx.db.with_conn(|conn| {
        attachment_repo::list_by_voucher(conn, voucher_id)?
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {voucher_id} 无图片可更换")))
    })?;

    let hash = dedup::md5_file(path)?;
    if hash == old.file_hash {
        // 同内容：幂等，不换
        return Ok(old);
    }
    let dup_other = ctx.db.with_conn(|conn| {
        let hit = dedup::find_duplicate(conn, &hash)?;
        Ok(hit.filter(|a| a.id != old.id))
    })?;
    if dup_other.is_some() {
        return Err(CoreError::Invalid("这张图片已存在于其他凭证".into()));
    }

    let year_month = crate::utils::paths::current_year_month();
    std::fs::create_dir_all(ctx.paths.attachment_dir_for(&year_month))?;
    let ext = imaging::ext_of(path);
    let rel = ctx.paths.attachment_rel_path(&hash, &ext, &year_month);
    let dest = ctx.paths.attachment_abs(&rel);
    std::fs::copy(path, &dest)?;
    let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    // 事务：快照 → 删旧附件行 → 插新附件行
    let new_attachment = ctx.db.with_tx(|tx| {
        let current = crate::db::repos::voucher_repo::get(tx, voucher_id)?
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {voucher_id} 不存在")))?;
        // v0.11 期间锁：已封账月份的凭证不可换图
        crate::services::period::ensure_dates_open(tx, &[current.voucher_date.as_str()])?;
        crate::db::repos::snapshot_repo::insert(
            tx,
            &crate::models::snapshot::NewSnapshot {
                voucher_id,
                payload: serde_json::to_value(&current)?,
                reason: "replace_image".into(),
            },
        )?;
        attachment_repo::delete(tx, old.id)?;
        attachment_repo::insert(
            tx,
            &NewAttachment {
                voucher_id,
                file_name,
                file_path: rel,
                file_hash: hash,
                file_size: size,
                mime_type: imaging::mime_from_ext(&ext).to_string(),
            },
        )
    })?;

    // 事务外：清理旧物理文件（含增强版，尽力而为）
    let old_abs = ctx.paths.attachment_abs(&old.file_path);
    if ctx.paths.is_inside_attachments(&old_abs) {
        let _ = std::fs::remove_file(&old_abs);
    }
    if let Some(old_enh) = &old.enhanced_path {
        let enh_abs = ctx.paths.attachment_abs(old_enh);
        if ctx.paths.is_inside_attachments(&enh_abs) {
            let _ = std::fs::remove_file(&enh_abs);
        }
    }
    Ok(new_attachment)
}
