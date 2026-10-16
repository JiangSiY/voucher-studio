//! 凭证关联 / 事项组命令（v0.22 关联批）：薄壳，只做参数整理与错误映射。
//!
//! 越层约束：命令层只经 `services::links`，不直连仓储或数据库（`guards.sh layering` 固化）。
//! 命名与前端的 `invoke("...")` 一一对应（`invoke-contract.test.ts` 双向断言）。

use super::*;
use voucher_core::models::enums::VoucherLinkKind;
use voucher_core::models::{LinkedVoucher, VoucherGroup, VoucherLink, VoucherLinkEvent};
use voucher_core::services::links;

/// 关联类型字符串 → 枚举（错误文案面向用户，列出全部合法值）
fn parse_kind(kind: &str) -> std::result::Result<VoucherLinkKind, ApiError> {
    VoucherLinkKind::parse(kind.trim()).ok_or_else(|| {
        ApiError::from_core(CoreError::Invalid(format!(
            "未知的关联类型「{kind}」（只支持 explains 附件说明 / detail 明细）"
        )))
    })
}

#[tauri::command]
pub fn link_vouchers(
    ctx: Ctx,
    a_id: i64,
    b_id: i64,
    kind: String,
    note: Option<String>,
) -> std::result::Result<VoucherLink, ApiError> {
    let k = parse_kind(&kind)?;
    err_api(links::link(
        &ctx,
        a_id,
        b_id,
        k,
        note.as_deref().unwrap_or(""),
    ))
}

#[tauri::command]
pub fn unlink_vouchers(
    ctx: Ctx,
    a_id: i64,
    b_id: i64,
    kind: String,
) -> std::result::Result<bool, ApiError> {
    let k = parse_kind(&kind)?;
    err_api(links::unlink(&ctx, a_id, b_id, k))
}

#[tauri::command]
pub fn list_voucher_links(
    ctx: Ctx,
    voucher_id: i64,
) -> std::result::Result<Vec<LinkedVoucher>, ApiError> {
    err_api(links::links_of(&ctx, voucher_id))
}

#[tauri::command]
pub fn list_voucher_link_events(
    ctx: Ctx,
    voucher_id: i64,
) -> std::result::Result<Vec<VoucherLinkEvent>, ApiError> {
    err_api(links::events_of(&ctx, voucher_id))
}

#[tauri::command]
pub fn create_voucher_group(
    ctx: Ctx,
    title: String,
    note: Option<String>,
) -> std::result::Result<VoucherGroup, ApiError> {
    err_api(links::create_group(
        &ctx,
        &title,
        note.as_deref().unwrap_or(""),
    ))
}

#[tauri::command]
pub fn add_vouchers_to_group(
    ctx: Ctx,
    group_id: i64,
    voucher_ids: Vec<i64>,
) -> std::result::Result<VoucherGroup, ApiError> {
    err_api(links::add_to_group(&ctx, group_id, &voucher_ids))
}

#[tauri::command]
pub fn remove_voucher_from_group(
    ctx: Ctx,
    group_id: i64,
    voucher_id: i64,
) -> std::result::Result<VoucherGroup, ApiError> {
    err_api(links::remove_from_group(&ctx, group_id, voucher_id))
}

#[tauri::command]
pub fn voucher_groups_of(
    ctx: Ctx,
    voucher_id: i64,
) -> std::result::Result<Vec<VoucherGroup>, ApiError> {
    err_api(links::groups_of_voucher(&ctx, voucher_id))
}

#[tauri::command]
pub fn list_voucher_groups(ctx: Ctx) -> std::result::Result<Vec<VoucherGroup>, ApiError> {
    err_api(links::list_groups(&ctx))
}

#[tauri::command]
pub fn delete_voucher_group(ctx: Ctx, group_id: i64) -> std::result::Result<bool, ApiError> {
    err_api(links::delete_group(&ctx, group_id))
}
