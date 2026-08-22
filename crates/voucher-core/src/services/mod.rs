//! 服务层（对应 v4.3 的 `services/` + ocr/pdf/webdav_backup/scheduler/wechat 等顶层模块）

pub mod backup;
pub mod dedup;
pub mod export;
pub mod imaging;
pub mod ingestion;
pub mod naming;
pub mod ocr;
pub mod ocr_fields;
pub mod ocr_local;
pub mod pdf;
pub mod recovery;
pub mod reports;
pub mod scheduler;
pub mod search;
pub mod voucher_mgr;
pub mod wechat;
// v0.6 文档扫描矫正与增强（5.5 方案，A 步：依赖接入 + 占位）
pub mod enhance;
pub mod enhance_cache;
pub mod enhance_geom;
pub mod enhance_pixel;
