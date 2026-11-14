//! 台账导入批次（v13）：一次导入 = 一条批次，撤销按批进行。
//!
//! 与 `ExportBatch` 对称：导出批次记"这次导出了什么"，导入批次记"这次导入了什么"。
//! 差异在于导入批次还要支撑**撤销**，所以带 `undone_at`——撤销只写时间戳，
//! 不删行、不删凭证历史（删掉的只是那批凭证本身，见 `services::ledger::undo_import_batch`）。

use serde::{Deserialize, Serialize};

/// 导入批次（读模型，直接下发给前端）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportBatch {
    pub id: i64,
    pub created_at: String,
    /// `shoukuan` / `fukuan` / `jiti`
    pub kind: String,
    pub file_name: String,
    pub file_path: String,
    /// CSV 数据行数（不含表头与空行）
    pub rows_total: i64,
    pub created: i64,
    pub matched: i64,
    pub skipped: i64,
    pub error_rows: i64,
    /// 撤销时间；None = 批次仍有效
    pub undone_at: Option<String>,
}

/// 新建批次入参
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NewImportBatch {
    pub kind: String,
    pub file_name: String,
    pub file_path: String,
    pub rows_total: i64,
    pub created: i64,
    pub matched: i64,
    pub skipped: i64,
    pub error_rows: i64,
}
