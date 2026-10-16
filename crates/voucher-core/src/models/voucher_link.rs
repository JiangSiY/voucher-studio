//! 凭证之间的关系（v0.22 关联批）与「事项组」。
//!
//! 设计要点（见 `docs/方案-v2-打印日报与凭证关联（定稿）.md`）：
//! - 只做**不带金额**的语义关系：`explains`（附件说明）与 `detail`（明细）；
//! - 两两关联存 `voucher_links`（归一化 `a_id < b_id`，双向可见）；
//! - 「明细」由**组**承载（`voucher_groups` + `voucher_group_items`），组的合计只作派生展示，
//!   **不改变任何金额合计口径**（关联 ≠ 抵消）；
//! - 建立/解除关联写 append-only 审计表 `voucher_link_events`（**刻意不加外键**，
//!   与 `dup_decisions` 同理：留痕必须独立于凭证生命周期）。

use serde::{Deserialize, Serialize};

use super::enums::{VoucherLinkAction, VoucherLinkKind};
use super::Voucher;

/// 一条两两关联
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherLink {
    pub id: i64,
    /// 归一化后的较小 id
    pub a_id: i64,
    /// 归一化后的较大 id
    pub b_id: i64,
    pub kind: VoucherLinkKind,
    pub note: String,
    pub created_at: String,
}

/// 新建关联（仓储负责归一化 `a < b`）
#[derive(Debug, Clone)]
pub struct NewVoucherLink {
    pub a_id: i64,
    pub b_id: i64,
    pub kind: VoucherLinkKind,
    pub note: String,
}

impl VoucherLink {
    /// 给定 id 在关系里的"对方"id
    pub fn other_id(&self, id: i64) -> i64 {
        if self.a_id == id {
            self.b_id
        } else {
            self.a_id
        }
    }
}

/// 右栏「相关凭证」的一行：关系 + **对方凭证**（含编号/日期/摘要/对方账户/金额/状态）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedVoucher {
    pub link_id: i64,
    pub kind: VoucherLinkKind,
    pub note: String,
    pub created_at: String,
    /// 关系另一端（用于展示与跳转）
    pub other: Voucher,
    /// 查询的凭证是不是这条关系的 a 端
    pub is_a: bool,
}

/// 事项组（组内成员＝「明细」）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherGroup {
    pub id: i64,
    pub title: String,
    pub note: String,
    pub created_at: String,
    /// 成员（按 `sort_order`）
    pub members: Vec<Voucher>,
}

impl VoucherGroup {
    /// 组内金额合计（**派生值**，仅供展示；不参与任何导出/统计口径）
    pub fn members_amount_cents(&self) -> i64 {
        self.members
            .iter()
            .filter_map(|v| v.amount_cents)
            .sum::<i64>()
    }
}

/// 新建审计事件（append-only）
#[derive(Debug, Clone)]
pub struct NewVoucherLinkEvent {
    pub link_id: i64,
    pub a_id: i64,
    pub b_id: i64,
    pub kind: VoucherLinkKind,
    pub action: VoucherLinkAction,
    pub note: String,
}

/// 审计事件（读侧：测试与排查用）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherLinkEvent {
    pub id: i64,
    pub link_id: i64,
    pub a_id: i64,
    pub b_id: i64,
    pub kind: String,
    pub action: String,
    pub note: String,
    pub at: String,
}
