//! 凭证快照模型（对应 v4.3 的 `models/snapshot.py`）
//!
//! 数据是生命线：凭证每次被修改前保存一份完整快照，可追溯、可回看。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherSnapshot {
    pub id: i64,
    pub voucher_id: i64,
    /// 修改前的完整凭证 JSON
    pub payload: serde_json::Value,
    /// 产生原因（update / delete / export 等）
    pub reason: String,
    pub created_at: String,
}

/// 新建快照入参
#[derive(Debug, Clone)]
pub struct NewSnapshot {
    pub voucher_id: i64,
    pub payload: serde_json::Value,
    pub reason: String,
}
