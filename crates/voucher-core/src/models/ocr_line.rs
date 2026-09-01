//! OCR 识别行（文本 + 置信度 + 坐标框）
//!
//! v5 起本地引擎的版面数据结构：识别时不丢坐标，落 ocr_layouts 表，
//! 供 ocr_layout 版面分析（几何键值配对）使用。

/// 一行识别结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrLine {
    pub text: String,
    /// 识别置信度（rec softmax 均值，0.0–1.0）
    pub score: f32,
    /// 文本框 [x0, y0, x1, y1]（原图像素坐标）
    pub box_xyxy: [f32; 4],
}

impl OcrLine {
    /// 行中心 y 坐标（行配对用）
    pub fn cy(&self) -> f32 {
        (self.box_xyxy[1] + self.box_xyxy[3]) / 2.0
    }

    /// 行高（像素）
    pub fn height(&self) -> f32 {
        self.box_xyxy[3] - self.box_xyxy[1]
    }

    /// 行左边缘 x
    pub fn x0(&self) -> f32 {
        self.box_xyxy[0]
    }

    /// 行右边缘 x
    pub fn x1(&self) -> f32 {
        self.box_xyxy[2]
    }
}
