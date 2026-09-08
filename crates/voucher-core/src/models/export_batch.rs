//! 导出批次模型（对应 v4.3 的 `models/export_batch.py`，export_batches / export_items 两张表）

use serde::{Deserialize, Serialize};

use super::enums::ExportStatus;

/// 导出批次：一次「选中若干凭证 → 生成一份文件」的记录
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportBatch {
    pub id: i64,
    /// 批次号，如 EXP-20260819-143205
    pub batch_no: String,
    pub voucher_count: i64,
    /// 生成的文件绝对路径（PDF / XLSX / CSV）
    pub pdf_path: String,
    pub status: ExportStatus,
    pub created_at: String,
    /// v0.10：导出格式 pdf | xlsx | csv（旧批次默认 pdf）
    #[serde(default = "default_format")]
    pub format: ExportFormat,
}

fn default_format() -> ExportFormat {
    ExportFormat::Pdf
}

/// 导出格式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    #[default]
    Pdf,
    Xlsx,
    Csv,
}

impl ExportFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExportFormat::Pdf => "pdf",
            ExportFormat::Xlsx => "xlsx",
            ExportFormat::Csv => "csv",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "xlsx" => ExportFormat::Xlsx,
            "csv" => ExportFormat::Csv,
            _ => ExportFormat::Pdf,
        }
    }

    /// 文件扩展名（不含点）
    pub fn ext(&self) -> &'static str {
        self.as_str()
    }
}

#[derive(Debug, Clone)]
pub struct NewExportBatch {
    pub batch_no: String,
    pub voucher_count: i64,
    pub pdf_path: String,
    pub status: ExportStatus,
    /// v0.10：导出格式（默认 pdf）
    pub format: ExportFormat,
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
