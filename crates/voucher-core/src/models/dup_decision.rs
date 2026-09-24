//! 疑似重复的处置记录（v0.16）
//!
//! 逐**对**记录：同一对只存一行（`a_id < b_id` 归一化 + 唯一索引），
//! 见 `db/schema.rs::SCHEMA_V7` 的注释（为什么刻意不加外键）。

use serde::{Deserialize, Serialize};

use super::enums::DupDecisionKind;

/// 一条处置记录
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DupDecision {
    pub id: i64,
    /// 归一化后的较小 id
    pub a_id: i64,
    /// 归一化后的较大 id
    pub b_id: i64,
    pub decision: DupDecisionKind,
    /// 仅 `DuplicateRemoved`：保留哪张
    pub keep_id: Option<i64>,
    /// 仅 `DuplicateRemoved`：删掉哪张
    pub removed_id: Option<i64>,
    /// 仅 `DuplicateRemoved`：被删凭证的**完整留痕**（凭证 JSON + 附件清单），
    /// 凭证本身删除后仍可追溯
    pub removed_payload: Option<serde_json::Value>,
    pub note: String,
    pub created_at: String,
}

impl DupDecision {
    /// 该记录涉及的两个 id 是否包含给定 id
    pub fn involves(&self, id: i64) -> bool {
        self.a_id == id || self.b_id == id
    }
}

/// 新建处置记录（`a_id`/`b_id` 由仓储归一化为 `a < b`）
#[derive(Debug, Clone)]
pub struct NewDupDecision {
    pub a_id: i64,
    pub b_id: i64,
    pub decision: DupDecisionKind,
    pub keep_id: Option<i64>,
    pub removed_id: Option<i64>,
    pub removed_payload: Option<serde_json::Value>,
    pub note: String,
}
