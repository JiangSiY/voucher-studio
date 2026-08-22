//! 附件模型（对应 v4.3 的 `models/attachment.py`）

use serde::{Deserialize, Serialize};

use super::enums::OcrStatus;

/// 附件来源分类（v0.6 文档扫描矫正）
///
/// - `Screenshot`：截图（手机/电脑截图）；不参与增强（已接近二值图，增强无收益）
/// - `Scan`：扫描件/手机拍照；走 5 步增强
/// - `Unknown`：未识别（默认保守走增强）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnhanceSource {
    Screenshot,
    Scan,
    Unknown,
}

impl EnhanceSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            EnhanceSource::Screenshot => "screenshot",
            EnhanceSource::Scan => "scan",
            EnhanceSource::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "screenshot" => Some(EnhanceSource::Screenshot),
            "scan" => Some(EnhanceSource::Scan),
            "unknown" => Some(EnhanceSource::Unknown),
            _ => None,
        }
    }

    pub fn label_cn(&self) -> &'static str {
        match self {
            EnhanceSource::Screenshot => "截图",
            EnhanceSource::Scan => "扫描件",
            EnhanceSource::Unknown => "未识别",
        }
    }
}

impl std::fmt::Display for EnhanceSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

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
    /// v3：5 步增强后的彩色 JPEG 相对路径（NULL = 未增强，原图回退）
    pub enhanced_path: Option<String>,
    /// v3：来源分类（截图/扫描件/未识别）
    pub enhanced_source: EnhanceSource,
    pub created_at: String,
}

impl Attachment {
    /// 是否为可在 PDF/预览中嵌入的图片
    pub fn is_image(&self) -> bool {
        self.mime_type.starts_with("image/")
    }

    /// 是否有增强版
    pub fn is_enhanced(&self) -> bool {
        self.enhanced_path.is_some()
    }
}

/// 新建附件入参（v3 保留全部字段，enhanced_path/enhanced_source 落库后由 set_enhanced 单独写）
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
