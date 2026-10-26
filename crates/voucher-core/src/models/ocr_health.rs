//! OCR 运行状态（进程内，不落盘）
//!
//! 设置页要回答两个用户最关心的问题：「刚才这张图到底走的云端还是本地？」
//! 和「云端为什么没用上（没 Key？断网？返回异常？）」。
//! 这里只保留**最近一次**结果，够用且不引入持久化与迁移负担。

use serde::Serialize;

/// 最近一次 OCR 的实际引擎与云端失败原因
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrHealth {
    /// 最近一次实际产出结果的引擎：`local` / `deepseek` / `cloud`
    pub last_engine: Option<String>,
    /// 最近一次云端失败原因（云端成功时清空）
    pub last_cloud_error: Option<String>,
    /// 最近一次识别完成时间（RFC3339）
    pub last_at: Option<String>,
}
