//! 疑似重复的**处置**（v0.16）
//!
//! 背景（2026-09-23 用户报"很鸡肋"）：语义层疑似重复自 v0.10 起只检测不处理——
//! 结果每次调用都重算、无任何持久化，于是仪表盘面板与保存提示永远重现，用户无处处置。
//!
//! 本模块补齐行业通行的处置闭环：**待核查队列 → 旁列对比（证据/置信度）→ 二元判定
//! （不是重复 / 确认重复并删掉多余那份）→ 非破坏性记忆 → 留痕**。
//! 判定口径**不变**（同日期 + 同金额，金额非空；对方账户只作证据，不做硬条件），
//! 因此"宁多报不漏报"的既有取舍继续成立，靠"不是重复"的记忆压制误报。
//!
//! 术语（全仓"重复"至少三义，勿混）：
//! - **内容重复**：同一张图（导入期 MD5，阻断式，`dedup::find_duplicate`）
//! - **疑似重复**：同日期同金额（本模块，提示式）
//! - **编号重复**：`voucher_no` UNIQUE 冲突（表单校验）
//!
//! 另注意对账里的"同日期同金额"含义**相反**（那是"匹配成功"，`reconcile.rs`）。

use std::collections::{HashMap, HashSet};

use rusqlite::params;
use serde::Serialize;

use crate::db::repos::{attachment_repo, dup_decision_repo, voucher_repo};
use crate::models::dup_decision::NewDupDecision;
use crate::models::enums::DupDecisionKind;
use crate::models::{DupDecision, Voucher};
use crate::services::{dedup, voucher_mgr};
use crate::{CoreContext, CoreError, Result};

/// 置信度分档（**只用于排序与提示强弱，不做过滤**）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceLevel {
    /// 同图（两张凭证的附件内容哈希相同——几乎可以确定是重录），或"同对方 ∧ 同摘要"
    High,
    /// 同对方或同摘要之一
    Medium,
    /// 仅"同日期 + 同金额"（可能只是同一天两笔相同金额的正常业务）
    Low,
}

impl EvidenceLevel {
    fn rank(&self) -> u8 {
        match self {
            EvidenceLevel::High => 0,
            EvidenceLevel::Medium => 1,
            EvidenceLevel::Low => 2,
        }
    }

    pub fn label_cn(&self) -> &'static str {
        match self {
            EvidenceLevel::High => "高",
            EvidenceLevel::Medium => "中",
            EvidenceLevel::Low => "低",
        }
    }
}

/// 一对疑似重复的**证据**（给人工核对用，不参与判定）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairEvidence {
    /// 两张凭证的首附件内容哈希相同（跨层打通的唯一强证据）
    pub same_image: bool,
    /// 对方账户相同（都非空）
    pub same_counterparty: bool,
    /// 摘要相同（都非空）
    pub same_description: bool,
    pub level: EvidenceLevel,
}

/// 待核查的一对
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPair {
    pub a: Voucher,
    pub b: Voucher,
    pub evidence: PairEvidence,
    /// 已判定则带上结论（队列默认只给未判定的，此字段供"已判定但仍想看"的场景）
    pub decided: Option<DupDecisionKind>,
}

/// 一个待核查组（同日期同金额）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewGroup {
    pub voucher_date: String,
    pub amount_cents: i64,
    /// 组内全部凭证
    pub vouchers: Vec<Voucher>,
    /// 未判定的对（按置信度排序：高 → 低）
    pub pending: Vec<ReviewPair>,
    /// 已判定的对数（供"3 对中已判定 1 对"的进度显示）
    pub decided_pairs: usize,
}

/// 轻量统计（侧栏徽标/仪表盘面板用，不必构造整个队列）
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingSummary {
    pub groups: u32,
    pub pairs: u32,
}

/// 每个凭证的首附件内容哈希（一图一凭证下即唯一附件；批量取，避免 N+1）
fn first_attachment_hashes(
    conn: &rusqlite::Connection,
    voucher_ids: &[i64],
) -> Result<HashMap<i64, String>> {
    if voucher_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; voucher_ids.len()].join(",");
    let sql = format!(
        "SELECT a.voucher_id, a.file_hash
         FROM attachments a
         INNER JOIN (
             SELECT voucher_id, MIN(id) AS min_id
             FROM attachments
             WHERE voucher_id IN ({placeholders})
             GROUP BY voucher_id
         ) m ON m.voucher_id = a.voucher_id AND m.min_id = a.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(voucher_ids.iter()))?;
    let mut out = HashMap::new();
    while let Some(r) = rows.next()? {
        out.insert(r.get(0)?, r.get(1)?);
    }
    Ok(out)
}

/// 组装一对的证据（纯计算）
fn evidence_of(a: &Voucher, b: &Voucher, hashes: &HashMap<i64, String>) -> PairEvidence {
    let same_image = match (hashes.get(&a.id), hashes.get(&b.id)) {
        (Some(x), Some(y)) => !x.is_empty() && x == y,
        _ => false,
    };
    let (ca, cb) = (a.counterparty.trim(), b.counterparty.trim());
    let same_counterparty = !ca.is_empty() && ca == cb;
    let (da, db) = (a.description.trim(), b.description.trim());
    let same_description = !da.is_empty() && da == db;
    let level = if same_image || (same_counterparty && same_description) {
        EvidenceLevel::High
    } else if same_counterparty || same_description {
        EvidenceLevel::Medium
    } else {
        EvidenceLevel::Low
    };
    PairEvidence {
        same_image,
        same_counterparty,
        same_description,
        level,
    }
}

/// 待核查队列：只返回**还有未判定对**的组；组内未判定的对按置信度排序。
pub fn review_queue(ctx: &CoreContext) -> Result<Vec<ReviewGroup>> {
    let (groups, hashes, decided) = ctx.db.with_conn(|conn| {
        let groups = dedup::scan_groups(conn)?;
        let ids: Vec<i64> = groups
            .iter()
            .flat_map(|g| g.vouchers.iter().map(|v| v.id))
            .collect();
        let hashes = first_attachment_hashes(conn, &ids)?;
        let decided = dup_decision_repo::decided_pairs(conn)?;
        Ok((groups, hashes, decided))
    })?;

    let mut out = Vec::new();
    for g in groups {
        let mut pending: Vec<ReviewPair> = Vec::new();
        let mut decided_pairs = 0usize;
        for i in 0..g.vouchers.len() {
            for j in (i + 1)..g.vouchers.len() {
                let a = g.vouchers[i].clone();
                let b = g.vouchers[j].clone();
                let (na, nb) = dup_decision_repo::normalize_pair(a.id, b.id);
                if decided.contains(&(na, nb)) {
                    decided_pairs += 1;
                    continue;
                }
                pending.push(ReviewPair {
                    evidence: evidence_of(&a, &b, &hashes),
                    a,
                    b,
                    decided: None,
                });
            }
        }
        if pending.is_empty() {
            continue; // 全判定的组不进队列
        }
        pending.sort_by_key(|p| p.evidence.level.rank());
        out.push(ReviewGroup {
            voucher_date: g.voucher_date,
            amount_cents: g.amount_cents,
            vouchers: g.vouchers,
            pending,
            decided_pairs,
        });
    }
    Ok(out)
}

/// 待处理统计（侧栏徽标/仪表盘面板）
///
/// **注意**：本函数会构造整个核查队列（含逐对证据与首附件哈希），成本较高——
/// 徽标/待办卡这类只需数字的场景请用 [`pending_count`]。
pub fn pending_summary(ctx: &CoreContext) -> Result<PendingSummary> {
    let queue = review_queue(ctx)?;
    Ok(PendingSummary {
        groups: queue.len() as u32,
        pairs: queue.iter().map(|g| g.pending.len() as u32).sum(),
    })
}

/// 轻量待处理计数（v0.18）：只做 SQL 计数，**不构造队列、不查证据**。
///
/// 口径与 [`review_queue`] 严格一致，并由用例锁定（`pending_count_matches_queue`）：
/// - `groups`：仍有未判定对的组数；
/// - `pairs`：组内 `C(n,2)` 之和减去已判定对；
/// - `high_pairs`：其中高置信度（同图 **或** 同对方∧同摘要）的对数——**精确值**，
///   同图部分靠 `attachments.file_hash` 等值自连接（走 `idx_attachments_hash`），
///   已判定的对由 `dup_decisions` 唯一索引排除。
///
/// 判定口径本身不变（同日期 + 同金额）；这里只是把"还剩多少、其中多少证据较强"用一条
/// SQL 算出来，供侧栏徽标与工作台待办卡使用——旧实现是跑完整个 `review_queue` 再取长度。
pub fn pending_count(ctx: &CoreContext) -> Result<PendingCount> {
    ctx.db.with_conn(|conn| {
        // 1) 高置信度对数：同图 ∨ (同对方 ∧ 同摘要)，且排除已判定的对
        let high_pairs: u32 = conn.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT v1.id AS a, v2.id AS b
                 FROM vouchers v1
                 JOIN vouchers v2
                   ON v1.id < v2.id
                  AND v1.voucher_date = v2.voucher_date
                  AND v1.amount_cents IS NOT NULL
                  AND v1.amount_cents = v2.amount_cents
                 WHERE (
                        (TRIM(COALESCE(v1.counterparty,'')) <> '' AND TRIM(v1.counterparty) = TRIM(v2.counterparty)
                         AND TRIM(COALESCE(v1.description,'')) <> '' AND TRIM(v1.description) = TRIM(v2.description))
                     OR EXISTS (
                         SELECT 1 FROM attachments a1 JOIN attachments a2 ON a1.file_hash = a2.file_hash
                         WHERE a1.voucher_id = v1.id AND a2.voucher_id = v2.id AND a1.file_hash <> ''
                     )
                 )
             ) pair
             WHERE NOT EXISTS (
                 SELECT 1 FROM dup_decisions d WHERE d.a_id = pair.a AND d.b_id = pair.b
             )",
            [],
            |r| r.get::<_, i64>(0),
        )? as u32;

        // 2) 组与对：沿用与队列相同的分组（同日期同金额、金额非空），逐组扣减已判定对
        let mut stmt = conn.prepare(
            "SELECT voucher_date, amount_cents FROM vouchers WHERE amount_cents IS NOT NULL
             GROUP BY voucher_date, amount_cents HAVING COUNT(*) >= 2",
        )?;
        let mut keys: Vec<(String, i64)> = Vec::new();
        {
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                keys.push((r.get(0)?, r.get(1)?));
            }
        }
        let decided = dup_decision_repo::decided_pairs(conn)?;

        let mut groups = 0u32;
        let mut pairs = 0u32;
        for (date, amount) in &keys {
            let mut stmt = conn
                .prepare("SELECT id FROM vouchers WHERE voucher_date = ?1 AND amount_cents = ?2")?;
            let mut ids: Vec<i64> = Vec::new();
            {
                let mut rows = stmt.query(params![date, amount])?;
                while let Some(r) = rows.next()? {
                    ids.push(r.get(0)?);
                }
            }
            let n = ids.len() as u32;
            if n < 2 {
                continue;
            }
            let in_group: std::collections::HashSet<i64> = ids.iter().copied().collect();
            let decided_in_group = decided
                .iter()
                .filter(|(a, b)| in_group.contains(a) && in_group.contains(b))
                .count() as u32;
            let pending = (n * (n - 1) / 2).saturating_sub(decided_in_group);
            if pending == 0 {
                continue;
            }
            groups += 1;
            pairs += pending;
        }

        Ok(PendingCount {
            groups,
            pairs,
            high_pairs: clamp_high_pairs(high_pairs, pairs),
        })
    })
}

/// 高置信度对的防御性收口（审计 A-8，2026-09-27）。
///
/// 不变式：高置信度对必然落在"同日期同金额"的某个组内，而该组只要还有未判定对就会计入
/// [`PendingCount::pairs`]，故 `high_pairs <= pairs` 应恒成立（由 `dashboard_flow.rs` 的
/// 判定矩阵用例锁定：初始态 / 判定一对 / 删除一张 / 整组判完 / 只有弱证据）。
///
/// 旧实现是 `.min(pairs)` + 一句"一定不会超过"的注释——注释与代码互斥：不变式成立时
/// 夹紧是死代码，不成立时它把偏小的数字**静默**喂给侧栏徽标。现在越界会留痕：
/// 测试环境由 `debug_assert!` 直接炸出来（矩阵用例可当场发现漂移），release 下告警并夹紧
/// （不让界面出现"高置信度对 > 待处理对"这种自相矛盾的数）。
fn clamp_high_pairs(high_pairs: u32, pairs: u32) -> u32 {
    debug_assert!(
        high_pairs <= pairs,
        "两条计数口径已漂移：high_pairs({high_pairs}) > pairs({pairs})"
    );
    if high_pairs > pairs {
        log::warn!(
            "高置信度对数（{high_pairs}）超过待处理对数（{pairs}）——dup_review 的两条 SQL 口径漂移，已夹紧；请对照 dashboard_flow.rs 的判定矩阵用例排查"
        );
        return pairs;
    }
    high_pairs
}

/// 轻量待处理计数结果
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCount {
    pub groups: u32,
    pub pairs: u32,
    /// 高置信度对数（同图 ∨ 同对方∧同摘要），已排除已判定的对
    pub high_pairs: u32,
}

/// 处置：判定"不是重复"（**不碰凭证，故不受封账限制**）。
/// 同一对重复判定为幂等（覆盖旧结论与备注）。
pub fn resolve_pair(ctx: &CoreContext, x: i64, y: i64, note: &str) -> Result<i64> {
    if x == y {
        return Err(CoreError::Invalid("同一张凭证无法与自身判重".into()));
    }
    ctx.db.with_conn(|conn| {
        for id in [x, y] {
            if voucher_repo::get(conn, id)?.is_none() {
                return Err(CoreError::NotFound(format!("凭证 {id} 不存在")));
            }
        }
        dup_decision_repo::upsert(
            conn,
            &NewDupDecision {
                a_id: x,
                b_id: y,
                decision: DupDecisionKind::NotDuplicate,
                keep_id: None,
                removed_id: None,
                removed_payload: None,
                note: note.to_string(),
            },
        )
    })
}

/// 处置：确认重复，**保留 `keep_id`、删除 `remove_id`**（含留痕）。
///
/// 单事务：取被删凭证的完整 JSON + 附件清单 → 写处置记录 → 删除凭证（复用
/// `voucher_mgr::delete_vouchers_in_tx`，含期间锁校验）→ 提交后清理物理文件。
/// **留痕与删除同成同败**：不会出现"删了却没留痕"，也不会"留痕说删了却还在"。
///
/// 返回 (处置记录 id, 删除的凭证数)。
pub fn remove_duplicate(
    ctx: &CoreContext,
    keep_id: i64,
    remove_id: i64,
    note: &str,
) -> Result<(i64, u64)> {
    if keep_id == remove_id {
        return Err(CoreError::Invalid("保留与删除不能是同一张凭证".into()));
    }
    let (decision_id, deleted, rels) = ctx.db.with_tx(|tx| {
        let keep = voucher_repo::get(tx, keep_id)?
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {keep_id} 不存在")))?;
        let remove = voucher_repo::get(tx, remove_id)?
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {remove_id} 不存在")))?;
        // 守卫：只允许删除"确实是同一组疑似重复"的凭证，避免从核查流入误删无关记录
        if keep.voucher_date != remove.voucher_date
            || keep.amount_cents.is_none()
            || keep.amount_cents != remove.amount_cents
        {
            return Err(CoreError::Invalid(format!(
                "凭证 {} 与 {} 不是同一组疑似重复（日期或金额不同），已拒绝删除",
                keep.voucher_no, remove.voucher_no
            )));
        }
        // 留痕：被删凭证的完整 JSON + 附件清单（凭证行删除后仍可追溯）。
        // 同时记下**保留侧**的 id 与编号：被删的凭证已不在库里，只留 removed_id 的话
        // 「已处理」列表就显示不出"留的是哪一张"（编号要再查一次库，还可能已被删）。
        let attachments = attachment_repo::list_by_voucher(tx, remove_id)?;
        let payload = serde_json::json!({
            "voucher": serde_json::to_value(&remove)?,
            "attachments": serde_json::to_value(&attachments)?,
            "keptVoucherId": keep_id,
            "keptVoucherNo": keep.voucher_no,
        });
        let decision_id = dup_decision_repo::upsert(
            tx,
            &NewDupDecision {
                a_id: keep_id,
                b_id: remove_id,
                decision: DupDecisionKind::DuplicateRemoved,
                keep_id: Some(keep_id),
                removed_id: Some(remove_id),
                removed_payload: Some(payload),
                note: note.to_string(),
            },
        )?;
        let (deleted, rels) = voucher_mgr::delete_vouchers_in_tx(tx, &[remove_id])?;
        Ok((decision_id, deleted, rels))
    })?;
    voucher_mgr::cleanup_attachment_files(ctx, &rels);
    Ok((decision_id, deleted))
}

/// 处置历史（新 → 旧）
pub fn history(ctx: &CoreContext, limit: u32) -> Result<Vec<DupDecision>> {
    ctx.db
        .with_conn(|conn| dup_decision_repo::list_recent(conn, limit))
}

/// 某张凭证**尚未判定**的疑似重复（保存后提示用：已判定的对不再提示，
/// 从根上消除"每次保存都重弹"）。返回 (对端凭证, 证据) 按置信度排序。
pub fn undecided_suspects(
    ctx: &CoreContext,
    voucher_id: i64,
) -> Result<Vec<(Voucher, PairEvidence)>> {
    let (me, suspects, hashes, decided) = ctx.db.with_conn(|conn| {
        let Some(me) = voucher_repo::get(conn, voucher_id)? else {
            return Ok((None, Vec::new(), HashMap::new(), HashSet::new()));
        };
        let suspects = dedup::find_suspected(conn, &me)?;
        let mut ids: Vec<i64> = suspects.iter().map(|v| v.id).collect();
        ids.push(me.id);
        let hashes = first_attachment_hashes(conn, &ids)?;
        let decided = dup_decision_repo::decided_pairs(conn)?;
        Ok((Some(me), suspects, hashes, decided))
    })?;
    let Some(me) = me else { return Ok(Vec::new()) };
    let mut out: Vec<(Voucher, PairEvidence)> = suspects
        .into_iter()
        .filter(|other| {
            let (a, b) = dup_decision_repo::normalize_pair(me.id, other.id);
            !decided.contains(&(a, b))
        })
        .map(|other| {
            let ev = evidence_of(&me, &other, &hashes);
            (other, ev)
        })
        .collect();
    out.sort_by_key(|(_, ev)| ev.level.rank());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::clamp_high_pairs;

    /// 不变式成立时原样返回（夹紧不得成为"静默改数"的暗箱）
    #[test]
    fn clamp_high_pairs_passes_through_within_invariant() {
        assert_eq!(clamp_high_pairs(0, 0), 0);
        assert_eq!(clamp_high_pairs(2, 5), 2);
        assert_eq!(
            clamp_high_pairs(5, 5),
            5,
            "相等是合法边界（高置信度对可以占满）"
        );
    }

    /// 越界即炸（审计 A-8）：这条路径若被走到，说明两条 SQL 口径已漂移——
    /// 测试环境必须立刻发现，而不是把偏小的数字喂给侧栏徽标
    #[test]
    #[should_panic(expected = "两条计数口径已漂移")]
    fn clamp_high_pairs_panics_on_drift() {
        let _ = clamp_high_pairs(6, 5);
    }
}
