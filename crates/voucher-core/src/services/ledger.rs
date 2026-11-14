//! 台账侧服务（v13 台账页）：待补附件 / 待入账 / 导入批次 / 撤销。
//!
//! # 三段口径（用户 2026-11-14 定稿）
//!
//! 一张台账凭证的一生：**导入（批次）→ 补附件 → 人工勾「已入账」**。
//! - `unattached`（待补附件）：`ledger_ref` 有值、但一张附件都没有——票据还没贴上；
//! - `unbooked`（待入账）：已经有附件、但 `ledger_booked_at` 仍为空——还没在金蝶入账；
//! - 两段都走完，这张凭证才从台账页消失（但它仍在「全部凭证」里，按正常凭证流转）。
//!
//! # 撤销为什么放在服务层
//!
//! 撤销要同时动 `import_batches`（写 `undone_at`）与 `vouchers`（删行），跨两个聚合，
//! 所以必须在一个 `with_tx` 里完成——仓储层各自只管自己的表。
//!
//! **只撤「还没干活」的**：已补附件或已导出/已归档的凭证一律保留并在结果里逐张说明原因。
//! 撤销是"这批导错了，重来"，不是"把已经做过的活抹掉"。

use serde::Serialize;

use crate::db::repos::{import_batch_repo, voucher_repo};
use crate::models::{ImportBatch, NewImportBatch, VoucherStatus};
use crate::{CoreContext, CoreError, Result};

/// 台账阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerStage {
    /// 待补附件（有台账关联、无附件）
    Unattached,
    /// 待入账（已有附件、未勾已入账）
    Unbooked,
}

impl LedgerStage {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim() {
            "unattached" => Ok(Self::Unattached),
            "unbooked" => Ok(Self::Unbooked),
            other => Err(CoreError::Invalid(format!(
                "未知的台账阶段：{other}（应为 unattached / unbooked）"
            ))),
        }
    }
}

/// 台账凭证条目（前端两条列表共用同一形状）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerItem {
    pub voucher_id: i64,
    pub voucher_no: String,
    pub voucher_date: String,
    pub ledger_ref: String,
    /// 收款/付款/计提（取自 `kind:` 标签；历史数据可能为空）
    pub ledger_kind: String,
    pub amount_cents: Option<i64>,
    pub counterparty: String,
    pub description: String,
    pub has_attachment: bool,
    /// 已勾「已入账」
    pub booked: bool,
    pub import_batch_id: Option<i64>,
}

/// 台账页概览计数（与两条列表同口径，供页头与待处理页那张只读卡使用）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerCounts {
    pub unattached: u64,
    pub unbooked: u64,
    /// 未撤销的批次数
    pub active_batches: u64,
}

/// 按阶段列出台账凭证（新的在前）
pub fn list_items(ctx: &CoreContext, stage: LedgerStage) -> Result<Vec<LedgerItem>> {
    let tail = match stage {
        LedgerStage::Unattached => {
            "AND NOT EXISTS (SELECT 1 FROM attachments a WHERE a.voucher_id = v.id)"
        }
        LedgerStage::Unbooked => {
            "AND EXISTS (SELECT 1 FROM attachments a WHERE a.voucher_id = v.id)
             AND TRIM(COALESCE(v.ledger_booked_at,'')) = ''"
        }
    };
    let sql = format!(
        "SELECT v.id, v.voucher_no, v.voucher_date, COALESCE(v.ledger_ref,''), v.tags,
                v.amount_cents, v.counterparty, v.description, v.import_batch_id,
                COALESCE(v.ledger_booked_at,''),
                EXISTS(SELECT 1 FROM attachments a WHERE a.voucher_id = v.id)
         FROM vouchers v
         WHERE TRIM(COALESCE(v.ledger_ref,'')) <> '' {tail}
         ORDER BY v.voucher_date DESC, v.id DESC"
    );
    ctx.db.with_conn(|conn| {
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let tags_json: String = r.get(4)?;
            let kind = serde_json::from_str::<Vec<String>>(&tags_json)
                .unwrap_or_default()
                .into_iter()
                .find_map(|t| t.strip_prefix("kind:").map(str::to_string))
                .unwrap_or_default();
            out.push(LedgerItem {
                voucher_id: r.get(0)?,
                voucher_no: r.get(1)?,
                voucher_date: r.get(2)?,
                ledger_ref: r.get(3)?,
                ledger_kind: kind,
                amount_cents: r.get(5)?,
                counterparty: r.get(6)?,
                description: r.get(7)?,
                import_batch_id: r.get(8)?,
                booked: !r.get::<_, String>(9)?.trim().is_empty(),
                has_attachment: r.get::<_, i64>(10)? != 0,
            });
        }
        Ok(out)
    })
}

/// 台账页概览计数（三次 COUNT，走 `ledger_ref` / `import_batch_id` 索引）
pub fn counts(ctx: &CoreContext) -> Result<LedgerCounts> {
    ctx.db.with_conn(|conn| {
        let unattached: u64 = conn.query_row(
            "SELECT COUNT(*) FROM vouchers v
              WHERE TRIM(COALESCE(v.ledger_ref,'')) <> ''
                AND NOT EXISTS (SELECT 1 FROM attachments a WHERE a.voucher_id = v.id)",
            [],
            |r| r.get(0),
        )?;
        let unbooked: u64 = conn.query_row(
            "SELECT COUNT(*) FROM vouchers v
              WHERE TRIM(COALESCE(v.ledger_ref,'')) <> ''
                AND EXISTS (SELECT 1 FROM attachments a WHERE a.voucher_id = v.id)
                AND TRIM(COALESCE(v.ledger_booked_at,'')) = ''",
            [],
            |r| r.get(0),
        )?;
        let active_batches: u64 = conn.query_row(
            "SELECT COUNT(*) FROM import_batches WHERE undone_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(LedgerCounts {
            unattached,
            unbooked,
            active_batches,
        })
    })
}

/// 批次列表（新的在前）
pub fn list_batches(ctx: &CoreContext, limit: u32) -> Result<Vec<ImportBatch>> {
    ctx.db
        .with_conn(|conn| import_batch_repo::list(conn, limit))
}

/// 勾 / 取消「已入账」（人工确认在金蝶入过账；金蝶凭证号暂不入库，见开发日志第七十五笔）
pub fn set_booked(ctx: &CoreContext, voucher_id: i64, booked: bool) -> Result<()> {
    ctx.db.with_tx(|tx| {
        if voucher_repo::get(tx, voucher_id)?.is_none() {
            return Err(CoreError::NotFound(format!("凭证 {voucher_id} 不存在")));
        }
        let stamp = if booked {
            Some(crate::utils::paths::now_iso())
        } else {
            None
        };
        tx.execute(
            "UPDATE vouchers SET ledger_booked_at = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![stamp, crate::utils::paths::now_iso(), voucher_id],
        )?;
        Ok(())
    })
}

/// 撤销一批导入：删掉这批里「还没干活」的凭证，其余保留并说明原因
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptVoucher {
    pub voucher_no: String,
    pub ledger_ref: String,
    /// `有附件` / `已导出` / `已归档`
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoBatchResult {
    pub batch_id: i64,
    pub removed: u32,
    pub kept: Vec<KeptVoucher>,
}

pub fn undo_batch(ctx: &CoreContext, batch_id: i64) -> Result<UndoBatchResult> {
    ctx.db.with_tx(|tx| {
        let batch = import_batch_repo::get(tx, batch_id)?
            .ok_or_else(|| CoreError::NotFound(format!("批次 {batch_id} 不存在")))?;
        if batch.undone_at.is_some() {
            return Err(CoreError::Invalid("这批已经撤销过".into()));
        }

        // 一次性取这批的全部凭证与各自的"有没有附件"，再逐张判定
        let mut stmt = tx.prepare(
            "SELECT v.id, v.voucher_no, COALESCE(v.ledger_ref,''), v.status,
                    EXISTS(SELECT 1 FROM attachments a WHERE a.voucher_id = v.id)
             FROM vouchers v
             WHERE v.import_batch_id = ?1
             ORDER BY v.id",
        )?;
        let mut rows = stmt.query(rusqlite::params![batch_id])?;
        let mut doomed: Vec<i64> = Vec::new();
        let mut kept: Vec<KeptVoucher> = Vec::new();
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            let voucher_no: String = r.get(1)?;
            let ledger_ref: String = r.get(2)?;
            let status =
                VoucherStatus::parse(&r.get::<_, String>(3)?).unwrap_or(VoucherStatus::Draft);
            let has_attachment: bool = r.get::<_, i64>(4)? != 0;
            let reason = if has_attachment {
                Some("有附件")
            } else if matches!(status, VoucherStatus::Exported) {
                Some("已导出")
            } else if matches!(status, VoucherStatus::Archived) {
                Some("已归档")
            } else {
                None
            };
            match reason {
                Some(reason) => kept.push(KeptVoucher {
                    voucher_no,
                    ledger_ref,
                    reason: reason.to_string(),
                }),
                None => doomed.push(id),
            }
        }
        drop(rows);
        drop(stmt);

        for id in &doomed {
            voucher_repo::delete(tx, *id)?;
        }
        import_batch_repo::mark_undone(tx, batch_id)?;

        Ok(UndoBatchResult {
            batch_id,
            removed: u32::try_from(doomed.len()).unwrap_or(u32::MAX),
            kept,
        })
    })
}

/// 导入完成后落一条批次，并把新建的凭证挂到这批上（同一个事务）。
///
/// 命中（已存在的）凭证**不改归属**：它们属于当初创建它们的那个批次，
/// 否则撤销会牵连到别的批次。
pub(crate) fn record_batch(
    ctx: &CoreContext,
    batch: &NewImportBatch,
    created_ids: &[i64],
) -> Result<ImportBatch> {
    ctx.db.with_tx(|tx| {
        let row = import_batch_repo::insert(tx, batch)?;
        for id in created_ids {
            tx.execute(
                "UPDATE vouchers SET import_batch_id = ?1 WHERE id = ?2",
                rusqlite::params![row.id, id],
            )?;
        }
        Ok(row)
    })
}
