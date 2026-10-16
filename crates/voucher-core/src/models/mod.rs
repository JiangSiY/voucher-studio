//! 业务模型（对应 v4.3 的 `voucher/models/`）

pub mod attachment;
pub mod dup_decision;
pub mod enums;
pub mod export_batch;
pub mod ocr_line;
pub mod snapshot;
pub mod voucher;
pub mod voucher_link;

pub use attachment::{Attachment, EnhanceSource, NewAttachment};
pub use dup_decision::{DupDecision, NewDupDecision};
pub use enums::{
    DupDecisionKind, ExportStatus, OcrStatus, VoucherLinkAction, VoucherLinkKind, VoucherStatus,
};
pub use export_batch::{ExportBatch, ExportFormat, ExportItem, NewExportBatch};
pub use ocr_line::OcrLine;
pub use snapshot::VoucherSnapshot;
pub use voucher::{NewVoucher, Voucher, VoucherUpdate};
pub use voucher_link::{
    LinkedVoucher, NewVoucherLink, NewVoucherLinkEvent, VoucherGroup, VoucherLink, VoucherLinkEvent,
};
