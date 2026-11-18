//! v0.27 工作台重构 B2a：凭证必填（attachment_required）与建条目写路径。
//!
//! 覆盖：新条目默认 required、迁移豁免可复核但仍不可入账/导出、行级 meta、
//! 筛选维度、set_booked 闸门、create_with_attachment 的路径白名单与失败回滚。

mod common;

use common::*;
use voucher_core::db::repos::attachment_repo;
use voucher_core::db::repos::voucher_repo::{self, VoucherQuery};
use voucher_core::models::attachment::NewAttachment;
use voucher_core::models::{NewVoucher, VoucherStatus, VoucherUpdate};
use voucher_core::services::{ledger, voucher_mgr};
use voucher_core::{CoreContext, CoreError};

fn attach(ctx: &CoreContext, voucher_id: i64) {
    ctx.db
        .with_conn(|conn| {
            attachment_repo::insert(
                conn,
                &NewAttachment {
                    voucher_id,
                    file_name: "scan.png".into(),
                    file_path: "2026-11/a.png".into(),
                    file_hash: "hash-a".into(),
                    file_size: 10,
                    mime_type: "image/png".into(),
                    is_pdf: Some(false),
                    page_count: None,
                },
            )
        })
        .expect("插附件");
}

#[test]
fn new_record_is_required_and_meta_is_side_loaded() {
    let (_dir, ctx) = test_ctx();
    let v = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "新条目".into(),
            ..Default::default()
        },
    )
    .expect("建条目");

    let d = voucher_mgr::get_detail(&ctx, v.id).expect("详情");
    assert!(d.row_meta.attachment_required, "新条目必须要求凭证");
    assert!(!d.row_meta.booked);
    assert!(d.row_meta.batch_id.is_none());

    let meta = voucher_mgr::row_meta(&ctx, &[v.id]).expect("meta");
    assert_eq!(meta.get(&v.id), Some(&d.row_meta));
}

#[test]
fn migrated_optional_record_can_be_reviewed_but_not_booked_or_exported() {
    let (_dir, ctx) = test_ctx();
    let v = voucher_mgr::create_voucher(&ctx, NewVoucher::default()).expect("建条目");
    ctx.db
        .with_conn(|conn| voucher_repo::set_attachment_required(conn, v.id, false))
        .expect("迁移豁免");

    let reviewed = voucher_mgr::update_voucher(
        &ctx,
        v.id,
        VoucherUpdate {
            status: Some(VoucherStatus::Reviewed),
            ..Default::default()
        },
    )
    .expect("历史无图条目仍可复核");
    assert_eq!(reviewed.status, VoucherStatus::Reviewed);

    let err = voucher_mgr::update_voucher(
        &ctx,
        v.id,
        VoucherUpdate {
            status: Some(VoucherStatus::Exported),
            ..Default::default()
        },
    )
    .expect_err("无凭证不能导出");
    assert!(matches!(err, CoreError::Invalid(_)), "实际：{err:?}");

    let err = ledger::set_booked(&ctx, v.id, true).expect_err("无凭证不能入账");
    assert!(matches!(err, CoreError::Invalid(_)), "实际：{err:?}");
    // 取消入账不受闸门限制
    ledger::set_booked(&ctx, v.id, false).expect("取消入账");
}

#[test]
fn attachment_gate_opens_after_attach() {
    let (_dir, ctx) = test_ctx();
    let v = voucher_mgr::create_voucher(&ctx, NewVoucher::default()).expect("建条目");
    attach(&ctx, v.id);
    ledger::set_booked(&ctx, v.id, true).expect("有凭证可入账");
    let meta = voucher_mgr::row_meta(&ctx, &[v.id]).expect("meta");
    assert!(meta.get(&v.id).expect("条目").booked);
}

#[test]
fn query_filters_by_booked_and_required() {
    let (_dir, ctx) = test_ctx();
    let a = voucher_mgr::create_voucher(&ctx, NewVoucher::default()).expect("a");
    let b = voucher_mgr::create_voucher(&ctx, NewVoucher::default()).expect("b");
    attach(&ctx, a.id);
    ctx.db
        .with_conn(|conn| voucher_repo::set_attachment_required(conn, b.id, false))
        .expect("b 豁免");
    ledger::set_booked(&ctx, a.id, true).expect("a 入账");

    let (booked, total) = voucher_mgr::list(
        &ctx,
        &VoucherQuery {
            booked: Some(true),
            page: 1,
            page_size: 20,
            ..Default::default()
        },
    )
    .expect("筛入账");
    assert_eq!(total, 1);
    assert_eq!(booked[0].id, a.id);

    let (required, total) = voucher_mgr::list(
        &ctx,
        &VoucherQuery {
            attachment_required: Some(true),
            page: 1,
            page_size: 20,
            ..Default::default()
        },
    )
    .expect("筛 required");
    assert_eq!(total, 1);
    assert_eq!(required[0].id, a.id);
}

#[test]
fn create_with_attachment_rejects_paths_outside_uploads() {
    let (dir, ctx) = test_ctx();
    let outside = dir.path().join("outside.png");
    make_png(&outside);
    let err = voucher_mgr::create_with_attachment(&ctx, NewVoucher::default(), &[outside], false)
        .expect_err("非 uploads 路径必须拒绝");
    assert!(matches!(err, CoreError::Invalid(_)), "实际：{err:?}");
    assert_eq!(voucher_count(&ctx).expect("张数"), 0, "拒绝时不得建条目");
}

#[test]
fn create_with_attachment_rolls_back_on_import_failure() {
    let (_dir, ctx) = test_ctx();
    let uploads = ctx.paths.temp.join("uploads");
    std::fs::create_dir_all(&uploads).expect("uploads");
    let bad = uploads.join("bad.txt");
    std::fs::write(&bad, b"not an image").expect("写文件");
    let err = voucher_mgr::create_with_attachment(&ctx, NewVoucher::default(), &[bad], false)
        .expect_err("不支持的类型应失败");
    assert!(err.to_string().contains("条目未创建") || err.to_string().contains("不支持"));
    assert_eq!(voucher_count(&ctx).expect("张数"), 0, "失败后不得留空条目");
}

#[test]
fn create_with_attachment_succeeds_and_cleans_temp() {
    let (_dir, ctx) = test_ctx();
    let uploads = ctx.paths.temp.join("uploads");
    std::fs::create_dir_all(&uploads).expect("uploads");
    let img = uploads.join("ok.png");
    make_png(&img);
    let detail = voucher_mgr::create_with_attachment(
        &ctx,
        NewVoucher {
            description: "带凭证的新条目".into(),
            ..Default::default()
        },
        std::slice::from_ref(&img),
        false,
    )
    .expect("建条目并挂凭证");
    assert_eq!(detail.attachments.len(), 1);
    assert!(detail.row_meta.attachment_required);
    assert!(!img.exists(), "临时文件应清理");
    assert_eq!(voucher_count(&ctx).expect("张数"), 1);
}
