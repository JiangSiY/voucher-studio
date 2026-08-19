//! 导出批次模型（对应 v4.3 的 `models/export_batch.py`，export_batches / export_items 两张表）

use serde::{Deserialize, Serialize};

use super::enums::ExportStatus;

/// 导出批次：一次「选中若干凭证 → 生成一份 PDF」的记录
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportBatch {
    pub id: i64,
    /// 批次号，如 EXP-20260819-143205
    pub batch_no: String,
    pub voucher_count: i64,
    /// 生成的 PDF 路径（绝对路径）
    pub pdf_path: String,
    pub status: ExportStatus,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct NewExportBatch {
    pub batch_no: String,
    pub voucher_count: i64,
    pub pdf_path: String,
    pub status: ExportStatus,
}

/// 导出明细：批次包含哪些凭证（含打印顺序）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportItem {
    pub id: i64,
    pub batch_id: i64,
    pub voucher_id: i64,
    pub sort_order: i64,
}
