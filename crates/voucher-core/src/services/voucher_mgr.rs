//! 凭证管理核心服务（对应 v4.3 的 `services/voucher_mgr.py`，应用心脏）
//!
//! 所有写操作都走显式事务，并在修改/删除前落快照——数据是生命线。

use rusqlite::Connection;

use crate::db::repos::voucher_repo::{VoucherNeighbor, VoucherQuery, VoucherStats};
use crate::db::repos::{attachment_repo, snapshot_repo, voucher_repo};
use crate::models::{
    Attachment, NewVoucher, Voucher, VoucherSnapshot, VoucherStatus, VoucherUpdate,
};
use crate::services::naming::NamingEngine;
use crate::{CoreContext, CoreError, Result};

/// 凭证详情（编辑页/详情页一次取全）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherDetail {
    pub voucher: Voucher,
    pub attachments: Vec<Attachment>,
    pub snapshots: Vec<VoucherSnapshot>,
}

pub fn create_voucher(ctx: &CoreContext, new: NewVoucher) -> Result<Voucher> {
    let cfg = ctx.config();
    let engine = NamingEngine::new(&cfg.naming.template);

    ctx.db.with_tx(|tx| {
        let date = if new.voucher_date.trim().is_empty() {
            crate::utils::paths::today()
        } else {
            validate_date(&new.voucher_date)?
        };

        let voucher_no = if new.voucher_no.trim().is_empty() {
            engine.next_no(tx, &date)?
        } else {
            let no = new.voucher_no.trim().to_string();
            if voucher_repo::get_by_no(tx, &no)?.is_some() {
                return Err(CoreError::Invalid(format!("凭证编号 {no} 已存在")));
            }
            no
        };

        let mut v = new.clone();
        v.voucher_no = voucher_no;
        v.voucher_date = date;
        voucher_repo::insert(tx, &v)
    })
}

pub fn update_voucher(ctx: &CoreContext, id: i64, update: VoucherUpdate) -> Result<Voucher> {
    let cfg = ctx.config();
    let engine = NamingEngine::new(&cfg.naming.template);

    ctx.db.with_tx(|tx| {
        let current = voucher_repo::get(tx, id)?
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {id} 不存在")))?;
        if let Some(d) = &update.voucher_date {
            validate_date(d)?;
        }
        // v0.11 期间锁：改出月 / 改入月都必须未封账
        let new_date = update.voucher_date.as_deref();
        crate::services::period::ensure_dates_open(
            tx,
            &[current.voucher_date.as_str(), new_date.unwrap_or("")],
        )?;
        // 快照：修改前的完整状态
        snapshot_repo::insert(
            tx,
            &crate::models::snapshot::NewSnapshot {
                voucher_id: id,
                payload: serde_json::to_value(&current)?,
                reason: "update".into(),
            },
        )?;
        if !voucher_repo::update(tx, id, &update)? {
            return Err(CoreError::NotFound(format!("凭证 {id} 不存在")));
        }
        // 档号日期以凭证日期为准：日期变更时按新日期重排编号。
        // 用户显式指定编号则尊重之；已导出/已归档编号锁定（已印在 PDF 中不能变）。
        let new_date = update
            .voucher_date
            .clone()
            .unwrap_or_else(|| current.voucher_date.clone());
        if update.voucher_no.is_none() && new_date != current.voucher_date {
            renumber_for_date(tx, &engine, id, &new_date)?;
        }
        voucher_repo::get(tx, id)?.ok_or_else(|| CoreError::NotFound(format!("凭证 {id} 不存在")))
    })
}

/// 日期变更后的档号重排：按新日期取下一个编号，落 renumber 快照后更新。
/// 仅草稿/已复核状态重排；已导出/已归档返回 None（编号锁定）。
/// 需在事务内调用（修改前状态应由调用方先落快照；本函数再落一份重排前快照）。
pub fn renumber_for_date(
    tx: &Connection,
    engine: &NamingEngine,
    id: i64,
    new_date: &str,
) -> Result<Option<String>> {
    let current = voucher_repo::get(tx, id)?
        .ok_or_else(|| CoreError::NotFound(format!("凭证 {id} 不存在")))?;
    if matches!(
        current.status,
        VoucherStatus::Exported | VoucherStatus::Archived
    ) {
        log::info!(
            "凭证 {id}（{}）已导出/归档，日期改为 {new_date} 时编号保持锁定",
            current.voucher_no
        );
        return Ok(None);
    }
    let new_no = engine.next_no(tx, new_date)?;
    if new_no == current.voucher_no {
        return Ok(None); // 同日重排无需变更
    }
    snapshot_repo::insert(
        tx,
        &crate::models::snapshot::NewSnapshot {
            voucher_id: id,
            payload: serde_json::to_value(&current)?,
            reason: "renumber".into(),
        },
    )?;
    voucher_repo::update(
        tx,
        id,
        &VoucherUpdate {
            voucher_no: Some(new_no.clone()),
            ..Default::default()
        },
    )?;
    log::info!(
        "档号重排：凭证 {id} {} → {new_no}（日期 {new_date}）",
        current.voucher_no
    );
    Ok(Some(new_no))
}

pub fn delete_voucher(ctx: &CoreContext, id: i64) -> Result<()> {
    delete_vouchers(ctx, &[id]).map(|_| ())
}

/// 批量删除凭证（单事务）：附件行随外键级联删除，事务提交后清理磁盘附件文件。
/// 注意：快照表同样级联——历史快照随凭证一并移除，删除不可恢复。
/// 返回实际删除的凭证数（不存在的 id 跳过）。
pub fn delete_vouchers(ctx: &CoreContext, ids: &[i64]) -> Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    // 事务内：逐个删除凭证（附件行由 ON DELETE CASCADE 移除），先收集附件相对路径（含增强版）
    let (deleted, attachment_rels) = ctx.db.with_tx(|tx| {
        let mut deleted = 0u64;
        let mut rels: Vec<String> = Vec::new();
        for id in ids {
            let Some(v) = voucher_repo::get(tx, *id)? else {
                continue; // 不存在的 id 跳过（幂等）
            };
            // v0.11 期间锁：已封账月份的凭证不可删除
            crate::services::period::ensure_dates_open(tx, &[v.voucher_date.as_str()])?;
            for a in attachment_repo::list_by_voucher(tx, *id)? {
                rels.push(a.file_path);
                if let Some(er) = a.enhanced_path {
                    rels.push(er);
                }
            }
            voucher_repo::delete(tx, *id)?;
            deleted += 1;
        }
        Ok((deleted, rels))
    })?;

    // 事务外：清理物理文件（尽力而为；失败仅告警，不影响数据一致性）
    for rel in &attachment_rels {
        let abs = ctx.paths.attachment_abs(rel);
        if ctx.paths.is_inside_attachments(&abs) {
            if let Err(e) = std::fs::remove_file(&abs) {
                log::warn!("删除附件文件失败 {}: {e}", abs.display());
            }
        }
    }
    // 清理可能空掉的月份目录
    let mut parents: Vec<std::path::PathBuf> = attachment_rels
        .iter()
        .filter_map(|rel| {
            ctx.paths
                .attachment_abs(rel)
                .parent()
                .map(|p| p.to_path_buf())
        })
        .collect();
    parents.sort();
    parents.dedup();
    for dir in parents {
        if dir.starts_with(&ctx.paths.attachments)
            && dir
                .read_dir()
                .map(|mut d| d.next().is_none())
                .unwrap_or(false)
        {
            let _ = std::fs::remove_dir(&dir);
        }
    }
    Ok(deleted)
}

pub fn get_detail(ctx: &CoreContext, id: i64) -> Result<VoucherDetail> {
    ctx.db.with_conn(|conn| {
        let voucher = voucher_repo::get(conn, id)?
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {id} 不存在")))?;
        let attachments = attachment_repo::list_by_voucher(conn, id)?;
        let snapshots = snapshot_repo::list_by_voucher(conn, id, 20)?;
        Ok(VoucherDetail {
            voucher,
            attachments,
            snapshots,
        })
    })
}

pub fn list(ctx: &CoreContext, query: &VoucherQuery) -> Result<(Vec<Voucher>, u64)> {
    ctx.db.with_conn(|conn| voucher_repo::list(conn, query))
}

/// 编辑页上下文导航：同筛选/同排序下的相邻凭证与位置（「保存并下一张」用）
pub fn neighbor(ctx: &CoreContext, id: i64, query: &VoucherQuery) -> Result<VoucherNeighbor> {
    ctx.db
        .with_conn(|conn| voucher_repo::neighbor(conn, id, query))
}

pub fn stats(ctx: &CoreContext) -> Result<VoucherStats> {
    ctx.db.with_conn(voucher_repo::stats)
}

/// 变更状态（事务 + 变更前快照，与其他写操作口径一致）
pub fn set_status(ctx: &CoreContext, id: i64, status: VoucherStatus) -> Result<()> {
    ctx.db.with_tx(|tx| {
        let current = voucher_repo::get(tx, id)?
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {id} 不存在")))?;
        // v0.11 期间锁
        crate::services::period::ensure_dates_open(tx, &[current.voucher_date.as_str()])?;
        snapshot_repo::insert(
            tx,
            &crate::models::snapshot::NewSnapshot {
                voucher_id: id,
                payload: serde_json::to_value(&current)?,
                reason: "status".into(),
            },
        )?;
        voucher_repo::set_status(tx, id, status)
    })
}

/// 批量变更状态（单事务逐张快照；不存在的跳过不计入）。
/// 整批一个事务：要么全部生效要么全部回滚，与单张口径一致。
pub fn batch_set_status(ctx: &CoreContext, ids: &[i64], status: VoucherStatus) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    ctx.db.with_tx(|tx| {
        let mut changed = 0usize;
        for id in ids {
            let current = match voucher_repo::get(tx, *id)? {
                Some(v) => v,
                None => continue, // 不存在/已删：跳过
            };
            if current.status == status {
                continue; // 已是目标状态：幂等跳过，不产生无意义快照
            }
            // v0.11 期间锁：已封账月份的凭证不可变更状态
            crate::services::period::ensure_dates_open(tx, &[current.voucher_date.as_str()])?;
            snapshot_repo::insert(
                tx,
                &crate::models::snapshot::NewSnapshot {
                    voucher_id: *id,
                    payload: serde_json::to_value(&current)?,
                    reason: "batch_status".into(),
                },
            )?;
            voucher_repo::set_status(tx, *id, status)?;
            changed += 1;
        }
        Ok(changed)
    })
}

fn validate_date(d: &str) -> Result<String> {
    if chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").is_ok() {
        Ok(d.trim().to_string())
    } else {
        Err(CoreError::Invalid(format!("日期 {d} 格式应为 YYYY-MM-DD")))
    }
}

/// 批量编辑补丁：None = 不修改该字段（仅开放批量安全的字段；金额/对方账户等强业务字段不开放）
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BatchEditPatch {
    /// 统一日期（变更后按新日期重排档号；已导出/归档的编号锁定）
    pub voucher_date: Option<String>,
    /// 备注（整体替换）
    pub note: Option<String>,
    /// 标签（整体替换）
    pub tags: Option<Vec<String>>,
}

/// 批量编辑（单事务逐张快照，reason=batch_edit）：日期 / 备注 / 标签。
/// 幂等跳过：凭证当前值已与补丁一致（或凭证不存在）时不产生快照、不计入变更数。
/// 返回实际变更数。空补丁报错（避免误触产生一串无意义快照）。
pub fn batch_edit(ctx: &CoreContext, ids: &[i64], patch: &BatchEditPatch) -> Result<usize> {
    if patch == &BatchEditPatch::default() {
        return Err(CoreError::Invalid("批量编辑内容为空：至少填写一项".into()));
    }
    if let Some(d) = &patch.voucher_date {
        validate_date(d)?;
    }
    if ids.is_empty() {
        return Ok(0);
    }
    let cfg = ctx.config();
    let engine = NamingEngine::new(&cfg.naming.template);

    ctx.db.with_tx(|tx| {
        let mut changed = 0usize;
        for id in ids {
            let Some(current) = voucher_repo::get(tx, *id)? else {
                continue; // 不存在/已删：跳过
            };
            // v0.11 期间锁：当前月 / 补丁目标月（改日期）都必须未封账
            let target_date = patch.voucher_date.as_deref().unwrap_or("");
            crate::services::period::ensure_dates_open(
                tx,
                &[current.voucher_date.as_str(), target_date],
            )?;
            let mut update = VoucherUpdate::default();
            let mut date_changed = false;
            if let Some(d) = &patch.voucher_date {
                if *d != current.voucher_date {
                    update.voucher_date = Some(d.clone());
                    date_changed = true;
                }
            }
            if let Some(n) = &patch.note {
                if *n != current.note {
                    update.note = Some(n.clone());
                }
            }
            if let Some(t) = &patch.tags {
                if *t != current.tags {
                    update.tags = Some(t.clone());
                }
            }
            if update == VoucherUpdate::default() {
                continue; // 全部字段已是目标值：幂等跳过，不产生快照
            }
            snapshot_repo::insert(
                tx,
                &crate::models::snapshot::NewSnapshot {
                    voucher_id: *id,
                    payload: serde_json::to_value(&current)?,
                    reason: "batch_edit".into(),
                },
            )?;
            voucher_repo::update(tx, *id, &update)?;
            // 日期变更 → 档号重排（规则同 update_voucher：导出/归档锁定）
            if date_changed {
                let new_date = patch.voucher_date.as_deref().unwrap_or_default();
                if let Some(new_no) = renumber_for_date(tx, &engine, *id, new_date)? {
                    log::info!("批量改日期触发档号重排：凭证 {id} → {new_no}");
                }
            }
            changed += 1;
        }
        Ok(changed)
    })
}

/// 给命令行/测试用：空连接上生成下一个编号
pub fn next_voucher_no(ctx: &CoreContext, date: &str) -> Result<String> {
    let cfg = ctx.config();
    let engine = NamingEngine::new(&cfg.naming.template);
    ctx.db
        .with_conn(|conn: &Connection| engine.next_no(conn, date))
}
