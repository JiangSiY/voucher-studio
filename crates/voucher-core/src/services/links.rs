//! 凭证关联 / 事项组服务（v0.22 关联批）。
//!
//! 口径（`docs/方案-v2-打印日报与凭证关联（定稿）.md`）：
//! - 关系**不带金额**：只记录"谁解释谁 / 谁是谁的明细"，**不改变任何金额合计**（关联 ≠ 抵消）；
//! - 建/解关联都走**同一个 `with_tx`**：先写关联、再追加 append-only 审计（照抄
//!   `voucher_mgr::delete_vouchers_in_tx` 的"留痕与变更同事务"范式）；
//! - 关联不受封账限制（不改金额与日期，且可解除）——若将来要拦，需显式查两端
//!   `voucher_date` 再调 `period::ensure_dates_open`；
//! - 组的金额合计只是**派生展示值**（`VoucherGroup::members_amount_cents`），不参与导出/统计口径。

use std::collections::HashMap;

use crate::db::repos::{voucher_link_repo, voucher_repo};
use crate::models::enums::{VoucherGroupAction, VoucherLinkAction, VoucherLinkKind};
use crate::models::{
    LinkedVoucher, NewVoucherGroupEvent, NewVoucherLink, NewVoucherLinkEvent, VoucherGroup,
    VoucherGroupEvent, VoucherLink, VoucherLinkCount,
};
use crate::{CoreContext, CoreError, Result};

/// 建立（或覆盖备注）一条关联；两端凭证必须存在，且不能自己关联自己
pub fn link(
    ctx: &CoreContext,
    a_id: i64,
    b_id: i64,
    kind: VoucherLinkKind,
    note: &str,
) -> Result<VoucherLink> {
    if a_id == b_id {
        return Err(CoreError::Invalid("不能把一张凭证关联到它自己".into()));
    }
    let note = note.trim().to_string();
    ctx.db.with_tx(|tx| {
        for id in [a_id, b_id] {
            if voucher_repo::get(tx, id)?.is_none() {
                return Err(CoreError::NotFound(format!("凭证 #{id} 不存在")));
            }
        }
        let link_id = voucher_link_repo::upsert(
            tx,
            &NewVoucherLink {
                a_id,
                b_id,
                kind,
                note: note.clone(),
            },
        )?;
        voucher_link_repo::insert_event(
            tx,
            &NewVoucherLinkEvent {
                link_id,
                a_id,
                b_id,
                kind,
                action: VoucherLinkAction::Linked,
                note: note.clone(),
            },
        )?;
        voucher_link_repo::get_pair(tx, a_id, b_id, kind)?
            .ok_or_else(|| CoreError::Invalid("关联写入后读回失败".into()))
    })
}

/// 解除一条关联；返回是否真的删掉了（幂等：不存在返回 false）
pub fn unlink(ctx: &CoreContext, a_id: i64, b_id: i64, kind: VoucherLinkKind) -> Result<bool> {
    ctx.db.with_tx(|tx| {
        let Some(removed) = voucher_link_repo::delete(tx, a_id, b_id, kind)? else {
            return Ok(false);
        };
        voucher_link_repo::insert_event(
            tx,
            &NewVoucherLinkEvent {
                link_id: removed.id,
                a_id: removed.a_id,
                b_id: removed.b_id,
                kind,
                action: VoucherLinkAction::Unlinked,
                note: removed.note.clone(),
            },
        )?;
        Ok(true)
    })
}

/// 某张凭证的全部关联（两个方向），带"对方凭证"摘要供右栏直接渲染
pub fn links_of(ctx: &CoreContext, voucher_id: i64) -> Result<Vec<LinkedVoucher>> {
    let links = ctx
        .db
        .with_conn(|conn| voucher_link_repo::list_of(conn, voucher_id))?;
    if links.is_empty() {
        return Ok(Vec::new());
    }
    let others: Vec<i64> = links.iter().map(|l| l.other_id(voucher_id)).collect();
    let vouchers: HashMap<i64, _> = ctx
        .db
        .with_conn(|conn| voucher_repo::get_many(conn, &others))?
        .into_iter()
        .map(|v| (v.id, v))
        .collect();
    Ok(links
        .into_iter()
        .filter_map(|l| {
            let other = vouchers.get(&l.other_id(voucher_id))?.clone();
            Some(LinkedVoucher {
                link_id: l.id,
                kind: l.kind,
                note: l.note,
                created_at: l.created_at,
                other,
                is_a: l.a_id == voucher_id,
            })
        })
        .collect())
}

/// 与某张凭证相关的审计事件（新→旧）
pub fn events_of(
    ctx: &CoreContext,
    voucher_id: i64,
) -> Result<Vec<crate::models::VoucherLinkEvent>> {
    ctx.db
        .with_conn(|conn| voucher_link_repo::events_of(conn, voucher_id))
}

// ---------- 事项组（「明细」的载体） ----------

/// 新建事项组（标题必填）
pub fn create_group(ctx: &CoreContext, title: &str, note: &str) -> Result<VoucherGroup> {
    let title = title.trim();
    if title.is_empty() {
        return Err(CoreError::Invalid("组的标题不能为空".into()));
    }
    ctx.db.with_tx(|tx| {
        let id = voucher_link_repo::create_group(tx, title, note.trim())?;
        // 审计与变更同事务（照抄 link/unlink 的范式）：组的建/删同样要留痕
        voucher_link_repo::insert_group_event(
            tx,
            &NewVoucherGroupEvent {
                group_id: id,
                action: VoucherGroupAction::Created,
                voucher_id: None,
                note: title.to_string(),
            },
        )?;
        group_view(tx, id)
    })
}

/// 把凭证加入组（幂等：已在组内则跳过），返回更新后的组
pub fn add_to_group(ctx: &CoreContext, group_id: i64, voucher_ids: &[i64]) -> Result<VoucherGroup> {
    if voucher_ids.is_empty() {
        return Err(CoreError::Invalid("没有选择要加入的凭证".into()));
    }
    ctx.db.with_tx(|tx| {
        if !voucher_link_repo::group_exists(tx, group_id)? {
            return Err(CoreError::NotFound(format!("事项组 #{group_id} 不存在")));
        }
        for id in voucher_ids {
            if voucher_repo::get(tx, *id)?.is_none() {
                return Err(CoreError::NotFound(format!("凭证 #{id} 不存在")));
            }
        }
        let added = voucher_link_repo::add_members(tx, group_id, voucher_ids)?;
        // 只给"真的加进去的"留痕：幂等重复调用不会重复记事件
        for vid in &added {
            voucher_link_repo::insert_group_event(
                tx,
                &NewVoucherGroupEvent {
                    group_id,
                    action: VoucherGroupAction::MemberAdded,
                    voucher_id: Some(*vid),
                    note: String::new(),
                },
            )?;
        }
        group_view(tx, group_id)
    })
}

/// 把凭证移出组，返回更新后的组
pub fn remove_from_group(
    ctx: &CoreContext,
    group_id: i64,
    voucher_id: i64,
) -> Result<VoucherGroup> {
    ctx.db.with_tx(|tx| {
        if !voucher_link_repo::group_exists(tx, group_id)? {
            return Err(CoreError::NotFound(format!("事项组 #{group_id} 不存在")));
        }
        let removed = voucher_link_repo::remove_member(tx, group_id, voucher_id)?;
        if removed > 0 {
            voucher_link_repo::insert_group_event(
                tx,
                &NewVoucherGroupEvent {
                    group_id,
                    action: VoucherGroupAction::MemberRemoved,
                    voucher_id: Some(voucher_id),
                    note: String::new(),
                },
            )?;
        }
        group_view(tx, group_id)
    })
}

/// 某张凭证所属的组（每个组带完整成员，供右栏「所属组」区块）
pub fn groups_of_voucher(ctx: &CoreContext, voucher_id: i64) -> Result<Vec<VoucherGroup>> {
    let ids = ctx
        .db
        .with_conn(|conn| voucher_link_repo::group_ids_of_voucher(conn, voucher_id))?;
    ctx.db.with_conn(|conn| {
        // 成员行随组删除级联清理，故这里不需要 group_exists 兜底：找不到就是真异常
        ids.iter().map(|id| group_view(conn, *id)).collect()
    })
}

/// 全部事项组（列表页/后续按组筛选用）
pub fn list_groups(ctx: &CoreContext) -> Result<Vec<VoucherGroup>> {
    let heads = ctx.db.with_conn(voucher_link_repo::list_group_heads)?;
    ctx.db.with_conn(|conn| {
        heads
            .iter()
            .map(|(id, _, _, _)| group_view(conn, *id))
            .collect()
    })
}

/// 删除事项组（成员行级联删除）
///
/// 删组前先取标题与成员数写进审计：删掉之后这些信息就再也查不到了，
/// 而"某月建过一个 12 张的组、后来又删了"恰恰是审计要能回答的问题。
pub fn delete_group(ctx: &CoreContext, group_id: i64) -> Result<bool> {
    ctx.db.with_tx(|tx| {
        let Some((_, title, _, _)) = voucher_link_repo::group_head(tx, group_id)? else {
            return Ok(false);
        };
        let members = voucher_link_repo::member_ids(tx, group_id)?.len();
        let n = voucher_link_repo::delete_group(tx, group_id)?;
        if n > 0 {
            voucher_link_repo::insert_group_event(
                tx,
                &NewVoucherGroupEvent {
                    group_id,
                    action: VoucherGroupAction::Deleted,
                    voucher_id: None,
                    note: format!("{title}（{members} 张）"),
                },
            )?;
        }
        Ok(n > 0)
    })
}

/// 某个组的事件（新→旧）
pub fn group_events_of(ctx: &CoreContext, group_id: i64) -> Result<Vec<VoucherGroupEvent>> {
    ctx.db
        .with_conn(|conn| voucher_link_repo::group_events_of(conn, group_id))
}

/// 列表徽标：一批凭证各自的关系计数（explains / detail / 所属组数）
pub fn link_counts(ctx: &CoreContext, voucher_ids: &[i64]) -> Result<Vec<VoucherLinkCount>> {
    if voucher_ids.is_empty() {
        return Ok(Vec::new());
    }
    let (links, groups) = ctx.db.with_conn(|conn| {
        Ok((
            voucher_link_repo::link_counts_in(conn, voucher_ids)?,
            voucher_link_repo::group_counts_in(conn, voucher_ids)?,
        ))
    })?;
    Ok(voucher_ids
        .iter()
        .map(|id| {
            let (explains, detail) = links.get(id).copied().unwrap_or((0, 0));
            VoucherLinkCount {
                voucher_id: *id,
                explains,
                detail,
                groups: groups.get(id).copied().unwrap_or(0),
            }
        })
        .collect())
}

/// 组视图：组头 + 成员（按 sort_order；成员缺席时跳过）
fn group_view(conn: &rusqlite::Connection, group_id: i64) -> Result<VoucherGroup> {
    let (id, title, note, created_at) = voucher_link_repo::group_head(conn, group_id)?
        .ok_or_else(|| CoreError::NotFound(format!("事项组 #{group_id} 不存在")))?;
    let ids = voucher_link_repo::member_ids(conn, group_id)?;
    let map: HashMap<i64, _> = voucher_repo::get_many(conn, &ids)?
        .into_iter()
        .map(|v| (v.id, v))
        .collect();
    let members = ids
        .iter()
        .filter_map(|vid| map.get(vid).cloned())
        .collect::<Vec<_>>();
    Ok(VoucherGroup {
        id,
        title,
        note,
        created_at,
        members,
    })
}

/// 相关凭证清单行（v0.22 批次③）：为一批凭证（＝某一天的日报）汇出它们的关联清单。
///
/// 只读；金额取「相关凭证自己的金额」作为**参考值**，页面上显式标注"不并入本日报合计"。
/// 该函数**不写 `export_items`**——批次张数与导出覆盖度口径完全不受影响。
pub fn appendix_rows(
    ctx: &CoreContext,
    voucher_ids: &[i64],
) -> Result<Vec<crate::services::pdf::PdfAppendixRow>> {
    if voucher_ids.is_empty() {
        return Ok(Vec::new());
    }
    // 去重键是**关联行 id**：同一天两张凭证都在这批里时，`list_of` 会从两端各读回
    // 同一条关联（A→B 与 B→A），不去重就会在附页上列两遍、小计重复计金额。
    // 保留首次出现的顺序，主凭证取先扫到的那一端（＝列表顺序里靠前的那张）。
    let pairs: Vec<(i64, VoucherLink)> = ctx.db.with_conn(|conn| {
        let mut out: Vec<(i64, VoucherLink)> = Vec::new();
        let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for id in voucher_ids {
            for l in voucher_link_repo::list_of(conn, *id)? {
                if seen.insert(l.id) {
                    out.push((*id, l));
                }
            }
        }
        Ok(out)
    })?;
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let mains: Vec<i64> = pairs.iter().map(|(id, _)| *id).collect();
    let others: Vec<i64> = pairs.iter().map(|(id, l)| l.other_id(*id)).collect();
    let (main_map, other_map) = ctx.db.with_conn(|conn| {
        let m: HashMap<i64, _> = voucher_repo::get_many(conn, &mains)?
            .into_iter()
            .map(|v| (v.id, v))
            .collect();
        let o: HashMap<i64, _> = voucher_repo::get_many(conn, &others)?
            .into_iter()
            .map(|v| (v.id, v))
            .collect();
        Ok((m, o))
    })?;
    Ok(pairs
        .into_iter()
        .filter_map(|(main_id, l)| {
            let main = main_map.get(&main_id)?;
            let other = other_map.get(&l.other_id(main_id))?;
            let summary = if l.note.trim().is_empty() {
                other.description.clone()
            } else {
                format!("{} · {}", other.description, l.note.trim())
            };
            Some(crate::services::pdf::PdfAppendixRow {
                main_no: main.voucher_no.clone(),
                kind_key: l.kind.as_str().to_string(),
                kind_label: l.kind.label_cn().to_string(),
                other_no: other.voucher_no.clone(),
                summary,
                amount_cents: other.amount_cents,
            })
        })
        .collect())
}
