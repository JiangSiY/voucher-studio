//! 凭证管理核心服务（对应 v4.3 的 `services/voucher_mgr.py`，应用心脏）
//!
//! 所有写操作都走显式事务，并在修改/删除前落快照——数据是生命线。

use rusqlite::Connection;

use crate::db::repos::voucher_repo::{VoucherQuery, VoucherStats};
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
    ctx.db.with_tx(|tx| {
        let current = voucher_repo::get(tx, id)?
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {id} 不存在")))?;
        if let Some(d) = &update.voucher_date {
            validate_date(d)?;
        }
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
        voucher_repo::get(tx, id)?.ok_or_else(|| CoreError::NotFound(format!("凭证 {id} 不存在")))
    })
}

pub fn delete_voucher(ctx: &CoreContext, id: i64) -> Result<()> {
    ctx.db.with_tx(|tx| {
        let current = voucher_repo::get(tx, id)?
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {id} 不存在")))?;
        snapshot_repo::insert(
            tx,
            &crate::models::snapshot::NewSnapshot {
                voucher_id: id,
                payload: serde_json::to_value(&current)?,
                reason: "delete".into(),
            },
        )?;
        voucher_repo::delete(tx, id)?;
        Ok(())
    })
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

pub fn stats(ctx: &CoreContext) -> Result<VoucherStats> {
    ctx.db.with_conn(voucher_repo::stats)
}

pub fn set_status(ctx: &CoreContext, id: i64, status: VoucherStatus) -> Result<()> {
    ctx.db.with_conn(|conn| {
        if voucher_repo::get(conn, id)?.is_none() {
            return Err(CoreError::NotFound(format!("凭证 {id} 不存在")));
        }
        voucher_repo::set_status(conn, id, status)
    })
}

fn validate_date(d: &str) -> Result<String> {
    if chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").is_ok() {
        Ok(d.trim().to_string())
    } else {
        Err(CoreError::Invalid(format!("日期 {d} 格式应为 YYYY-MM-DD")))
    }
}

/// 给命令行/测试用：空连接上生成下一个编号
pub fn next_voucher_no(ctx: &CoreContext, date: &str) -> Result<String> {
    let cfg = ctx.config();
    let engine = NamingEngine::new(&cfg.naming.template);
    ctx.db
        .with_conn(|conn: &Connection| engine.next_no(conn, date))
}
