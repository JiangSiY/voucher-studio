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
