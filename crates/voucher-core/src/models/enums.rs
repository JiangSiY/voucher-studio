//! 枚举（对应 v4.3 的 `models/enums.py`）

use serde::{Deserialize, Serialize};

/// 凭证状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VoucherStatus {
    /// 草稿（刚导入/录入）
    Draft,
    /// 已复核
    Reviewed,
    /// 已导出（生成过 PDF）
    Exported,
    /// 已归档
    Archived,
}

impl VoucherStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            VoucherStatus::Draft => "draft",
            VoucherStatus::Reviewed => "reviewed",
            VoucherStatus::Exported => "exported",
            VoucherStatus::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(VoucherStatus::Draft),
            "reviewed" => Some(VoucherStatus::Reviewed),
            "exported" => Some(VoucherStatus::Exported),
            "archived" => Some(VoucherStatus::Archived),
            _ => None,
        }
    }

    /// 中文标签（前端展示用）
    pub fn label_cn(&self) -> &'static str {
        match self {
            VoucherStatus::Draft => "草稿",
            VoucherStatus::Reviewed => "已复核",
            VoucherStatus::Exported => "已导出",
            VoucherStatus::Archived => "已归档",
        }
    }

    pub fn all() -> [VoucherStatus; 4] {
        [
            VoucherStatus::Draft,
            VoucherStatus::Reviewed,
            VoucherStatus::Exported,
            VoucherStatus::Archived,
        ]
    }
}

impl std::fmt::Display for VoucherStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 附件 OCR 状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OcrStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl OcrStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            OcrStatus::Pending => "pending",
            OcrStatus::Running => "running",
            OcrStatus::Done => "done",
            OcrStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(OcrStatus::Pending),
            "running" => Some(OcrStatus::Running),
            "done" => Some(OcrStatus::Done),
            "failed" => Some(OcrStatus::Failed),
            _ => None,
        }
    }

    pub fn label_cn(&self) -> &'static str {
        match self {
            OcrStatus::Pending => "待识别",
            OcrStatus::Running => "识别中",
            OcrStatus::Done => "已识别",
            OcrStatus::Failed => "识别失败",
        }
    }
}

/// 疑似重复的处置结论（v0.16）
///
/// - `NotDuplicate`：人工核对后判定"不是重复"——**不碰凭证**，只记下这一对已被判定，
///   从待核查队列与保存提示里消失（对应行业里的 "Mark as non-duplicate"）
/// - `DuplicateRemoved`：确认重复并删掉多余的那一份——留痕（`removed_payload`）后删除
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DupDecisionKind {
    NotDuplicate,
    DuplicateRemoved,
}

impl DupDecisionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DupDecisionKind::NotDuplicate => "not_duplicate",
            DupDecisionKind::DuplicateRemoved => "duplicate_removed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "not_duplicate" => Some(DupDecisionKind::NotDuplicate),
            "duplicate_removed" => Some(DupDecisionKind::DuplicateRemoved),
            _ => None,
        }
    }

    pub fn label_cn(&self) -> &'static str {
        match self {
            DupDecisionKind::NotDuplicate => "已判定不是重复",
            DupDecisionKind::DuplicateRemoved => "已删除多余的一份",
        }
    }
}

impl std::fmt::Display for DupDecisionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 导出批次状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportStatus {
    Done,
    Failed,
}

impl ExportStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExportStatus::Done => "done",
            ExportStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "done" => Some(ExportStatus::Done),
            "failed" => Some(ExportStatus::Failed),
            _ => None,
        }
    }
}

/// 凭证之间的关系类型（v0.22 关联批）：**只做不带金额的语义关系**。
///
/// 与「疑似重复」（`DupDecisionKind`）是两回事：这里记录"谁解释谁 / 谁是谁的明细"，
/// 不改变任何金额合计口径（关联 ≠ 抵消，见方案 v2 §5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoucherLinkKind {
    /// 附件说明：一张说明/备忘/分割单凭证解释另一张凭证
    Explains,
    /// 明细：一张凭证的明细构成（由「事项组」承载，见 `voucher_group_items`）
    Detail,
}

impl VoucherLinkKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            VoucherLinkKind::Explains => "explains",
            VoucherLinkKind::Detail => "detail",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "explains" => Some(VoucherLinkKind::Explains),
            "detail" => Some(VoucherLinkKind::Detail),
            _ => None,
        }
    }

    pub fn label_cn(&self) -> &'static str {
        match self {
            VoucherLinkKind::Explains => "附件说明",
            VoucherLinkKind::Detail => "明细",
        }
    }
}

impl std::fmt::Display for VoucherLinkKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 关联审计动作（append-only 的 `voucher_link_events.action`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoucherLinkAction {
    Linked,
    Unlinked,
}

impl VoucherLinkAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            VoucherLinkAction::Linked => "linked",
            VoucherLinkAction::Unlinked => "unlinked",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "linked" => Some(VoucherLinkAction::Linked),
            "unlinked" => Some(VoucherLinkAction::Unlinked),
            _ => None,
        }
    }
}

impl std::fmt::Display for VoucherLinkAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 事项组审计动作（append-only 的 `voucher_group_events.action`）。
///
/// 与 `VoucherLinkAction` 分表存放：两两关联是"两条边"，事项组是"一个容器 + 成员行"，
/// 混在一张审计表里会让 `link_id` / `a_id` / `b_id` 三列失去意义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoucherGroupAction {
    /// 建组
    Created,
    /// 加入成员（每张凭证一条事件，voucher_id 有值）
    MemberAdded,
    /// 移出成员（每张凭证一条事件，voucher_id 有值）
    MemberRemoved,
    /// 删组（成员行级联消失，事件记下标题与人数）
    Deleted,
}

impl VoucherGroupAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            VoucherGroupAction::Created => "created",
            VoucherGroupAction::MemberAdded => "member_added",
            VoucherGroupAction::MemberRemoved => "member_removed",
            VoucherGroupAction::Deleted => "deleted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "created" => Some(VoucherGroupAction::Created),
            "member_added" => Some(VoucherGroupAction::MemberAdded),
            "member_removed" => Some(VoucherGroupAction::MemberRemoved),
            "deleted" => Some(VoucherGroupAction::Deleted),
            _ => None,
        }
    }
}

impl std::fmt::Display for VoucherGroupAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
