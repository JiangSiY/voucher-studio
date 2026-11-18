//! 凭证关联 / 事项组命令（与 `src-tauri/src/commands/links.rs` 对应）。

use voucher_core::models::enums::VoucherLinkKind;
use voucher_core::models::{
    LinkedVoucher, VoucherGroup, VoucherGroupEvent, VoucherLink, VoucherLinkCount, VoucherLinkEvent,
};
use voucher_core::services::links;

use crate::{core, ApiError, ApiResult, CoreContext, CoreError};

/// 关联类型字符串 → 枚举（错误文案面向用户，列出全部合法值）
fn parse_kind(kind: &str) -> ApiResult<VoucherLinkKind> {
    VoucherLinkKind::parse(kind.trim()).ok_or_else(|| {
        ApiError::from_core(CoreError::Invalid(format!(
            "未知的关联类型「{kind}」（只支持 explains 附件说明 / detail 明细）"
        )))
    })
}

pub fn link_vouchers(
    ctx: &CoreContext,
    a_id: i64,
    b_id: i64,
    kind: String,
    note: Option<String>,
) -> ApiResult<VoucherLink> {
    let k = parse_kind(&kind)?;
    core(links::link(
        ctx,
        a_id,
        b_id,
        k,
        note.as_deref().unwrap_or(""),
    ))
}

pub fn unlink_vouchers(ctx: &CoreContext, a_id: i64, b_id: i64, kind: String) -> ApiResult<bool> {
    let k = parse_kind(&kind)?;
    core(links::unlink(ctx, a_id, b_id, k))
}

pub fn list_voucher_links(ctx: &CoreContext, voucher_id: i64) -> ApiResult<Vec<LinkedVoucher>> {
    core(links::links_of(ctx, voucher_id))
}

pub fn list_voucher_link_events(
    ctx: &CoreContext,
    voucher_id: i64,
) -> ApiResult<Vec<VoucherLinkEvent>> {
    core(links::events_of(ctx, voucher_id))
}

pub fn create_voucher_group(
    ctx: &CoreContext,
    title: String,
    note: Option<String>,
) -> ApiResult<VoucherGroup> {
    core(links::create_group(
        ctx,
        &title,
        note.as_deref().unwrap_or(""),
    ))
}

pub fn add_vouchers_to_group(
    ctx: &CoreContext,
    group_id: i64,
    voucher_ids: Vec<i64>,
) -> ApiResult<VoucherGroup> {
    core(links::add_to_group(ctx, group_id, &voucher_ids))
}

pub fn remove_voucher_from_group(
    ctx: &CoreContext,
    group_id: i64,
    voucher_id: i64,
) -> ApiResult<VoucherGroup> {
    core(links::remove_from_group(ctx, group_id, voucher_id))
}

pub fn voucher_groups_of(ctx: &CoreContext, voucher_id: i64) -> ApiResult<Vec<VoucherGroup>> {
    core(links::groups_of_voucher(ctx, voucher_id))
}

pub fn list_voucher_groups(ctx: &CoreContext) -> ApiResult<Vec<VoucherGroup>> {
    core(links::list_groups(ctx))
}

pub fn delete_voucher_group(ctx: &CoreContext, group_id: i64) -> ApiResult<bool> {
    core(links::delete_group(ctx, group_id))
}

pub fn voucher_link_counts(
    ctx: &CoreContext,
    voucher_ids: Vec<i64>,
) -> ApiResult<Vec<VoucherLinkCount>> {
    core(links::link_counts(ctx, &voucher_ids))
}

pub fn list_voucher_group_events(
    ctx: &CoreContext,
    group_id: i64,
) -> ApiResult<Vec<VoucherGroupEvent>> {
    core(links::group_events_of(ctx, group_id))
}
