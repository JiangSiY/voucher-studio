//! 附件模型（对应 v4.3 的 `models/attachment.py`）

use serde::{Deserialize, Serialize};

use super::enums::OcrStatus;

/// 附件（凭证对应的图片/PDF 原件，按内容哈希存储实现去重）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: i64,
    pub voucher_id: i64,
    /// 原始文件名（导入时的名字）
    pub file_name: String,
    /// 存储路径（相对附件根目录，如 2026/08/ab12cd34.jpg）
    pub file_path: String,
    /// 文件内容 MD5（去重键）
    pub file_hash: String,
    pub file_size: u64,
    pub mime_type: String,
    pub ocr_status: OcrStatus,
    pub ocr_text: Option<String>,
    pub created_at: String,
}

/// 新建附件入参
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAttachment {
    pub voucher_id: i64,
    pub file_name: String,
    pub file_path: String,
    pub file_hash: String,
    pub file_size: u64,
    pub mime_type: String,
}

impl Attachment {
    /// 是否为可在 PDF/预览中嵌入的图片
    pub fn is_image(&self) -> bool {
        self.mime_type.starts_with("image/")
    }
}
