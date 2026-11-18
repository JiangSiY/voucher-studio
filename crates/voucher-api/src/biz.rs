//! 台账字段域命令：读/写必填字段、按台账口径改名。
//!
//! 三者围绕同一个 `BizFields`：编辑 → 校验 → 改名。顺序在前端约束
//! （先补必填再改名），服务端只保证每一步自己是对的。

use voucher_core::models::BizFields;
use voucher_core::services::biz;
pub use voucher_core::services::biz::{ArchiveApplied, BizDetail};

use crate::{core, ApiResult, CoreContext};

pub fn get_biz_fields(ctx: &CoreContext, voucher_id: i64) -> ApiResult<BizDetail> {
    core(biz::detail(ctx, voucher_id))
}

pub fn save_biz_fields(
    ctx: &CoreContext,
    voucher_id: i64,
    fields: BizFields,
) -> ApiResult<BizDetail> {
    ctx.db
        .with_conn(|conn| biz::save(conn, voucher_id, &fields))?;
    core(biz::detail(ctx, voucher_id))
}

pub fn apply_archive_name(ctx: &CoreContext, voucher_id: i64) -> ApiResult<ArchiveApplied> {
    core(biz::apply_archive_name(ctx, voucher_id))
}
