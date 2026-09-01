//! 业务模型（对应 v4.3 的 `voucher/models/`）

pub mod attachment;
pub mod enums;
pub mod export_batch;
pub mod ocr_line;
pub mod snapshot;
pub mod voucher;

pub use attachment::{Attachment, EnhanceSource, NewAttachment};
pub use enums::{ExportStatus, OcrStatus, VoucherStatus};
pub use export_batch::{ExportBatch, ExportItem, NewExportBatch};
pub use ocr_line::OcrLine;
pub use snapshot::VoucherSnapshot;
pub use voucher::{NewVoucher, Voucher, VoucherUpdate};
