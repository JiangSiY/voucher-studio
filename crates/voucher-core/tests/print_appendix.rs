//! v0.22 批次③：日报「相关凭证清单」页（附件清单页）与**批次口径不变**。
//!
//! 两条关键不变量：
//! 1. 清单页是**数据驱动**的——当天没有任何关联就没有这一页（不是空页）；
//! 2. 清单页**不写 `export_items`**——批次张数与「从未导出」台账只认主凭证，
//!    被列进清单的相关凭证**不会**因此被算作"已导出"。

mod common;

use common::*;
use voucher_core::models::enums::VoucherLinkKind;
use voucher_core::models::NewVoucher;
use voucher_core::services::{export, links, voucher_mgr};
use voucher_core::{CoreContext, Result};

fn mk(ctx: &CoreContext, no: &str, amount_cents: Option<i64>) -> Result<i64> {
    Ok(voucher_mgr::create_voucher(
        ctx,
        NewVoucher {
            voucher_no: no.into(),
            voucher_date: "2026-08-20".into(),
            description: format!("{no} 摘要"),
            counterparty: "文一办公用品店".into(),
            amount_cents,
            status: None,
            tags: Vec::new(),
            note: String::new(),
        },
    )?
    .id)
}

/// printpdf 的页面框（与 `spike_a_print.rs` 同一套读法）
fn pdf_text(path: &str) -> String {
    String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned()
}

fn page_count(text: &str) -> usize {
    text.matches("MediaBox").count()
}

/// 某批次写进 `export_items` 的凭证 id（排序后）
fn batch_items(ctx: &CoreContext, batch_id: i64) -> Result<Vec<i64>> {
    ctx.db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT voucher_id FROM export_items WHERE batch_id = ?1 ORDER BY voucher_id",
        )?;
        let rows = stmt.query_map([batch_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<i64>>>()?)
    })
}

#[test]
fn daily_report_appendix_page_is_data_driven_and_keeps_batch_scope() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let a = mk(&ctx, "记-A", Some(1_000))?;
    let b = mk(&ctx, "记-B", Some(2_000))?;
    let c = mk(&ctx, "记-C", Some(3_000))?; // 只是"相关凭证"，不参与本次导出

    links::link(&ctx, a, c, VoucherLinkKind::Explains, "这笔付款对应的发票")?;
    links::link(&ctx, b, c, VoucherLinkKind::Detail, "")?;

    // ---- ① 关掉清单页：只有汇总表页 ----
    let off = export::ExportOptions {
        include_links: false,
        ..Default::default()
    };
    let b_off = export::export_vouchers_daily(&ctx, &[a, b], &off)?;
    assert_eq!(b_off.len(), 1, "同一天一个批次");
    assert_eq!(page_count(&pdf_text(&b_off[0].pdf_path)), 1);

    // ---- ② 默认带清单页：多出恰好一页（2 行关系 + 2 行小计，一页放得下）----
    let on = export::ExportOptions {
        include_links: true,
        ..Default::default()
    };
    let b_on = export::export_vouchers_daily(&ctx, &[a, b], &on)?;
    assert_eq!(b_on.len(), 1);
    let pages_on = page_count(&pdf_text(&b_on[0].pdf_path));
    assert_eq!(pages_on, 2, "汇总表 1 页 + 清单 1 页");

    // ---- ③ 口径不变：批次张数只数主凭证；相关凭证**不进** export_items ----
    assert_eq!(b_on[0].voucher_count, 2);
    let items_on = batch_items(&ctx, b_on[0].id)?;
    assert_eq!(items_on, vec![a, b], "清单页不改变批次明细");
    assert!(!items_on.contains(&c), "被列进清单的相关凭证不算已导出");

    // ---- ④ 数据驱动：当天没有任何关联就没有清单页（不是空页）----
    // 注意 `c` 自己是有关联的（a、b 都指向它），所以要另造一张"干净"的凭证
    let d = mk(&ctx, "记-D", Some(4_000))?;
    let solo = export::export_vouchers_daily(&ctx, &[d], &on)?;
    assert_eq!(
        page_count(&pdf_text(&solo[0].pdf_path)),
        1,
        "没有关联 → 不加页"
    );
    let _ = c;

    // ---- ⑤ 清单另存 CSV：可核、且把「不并入合计」印在文件里 ----
    let csv_path = ctx
        .paths
        .exports
        .join(format!("{}-关联清单.csv", b_on[0].batch_no));
    let csv = std::fs::read_to_string(&csv_path).expect("关联清单 CSV 应存在");
    assert!(
        csv.starts_with('\u{FEFF}'),
        "UTF-8 BOM（Excel 直接打开不乱码）"
    );
    assert!(csv.contains("主凭证,关系类型,相关凭证,说明,参考金额（元）,是否并入合计"));
    assert!(csv.contains("记-A,附件说明,记-C,记-C 摘要 · 这笔付款对应的发票,30.00,否（仅参考）"));
    assert!(csv.contains("记-B,明细,记-C,"));
    // 关掉开关时不写 CSV
    assert!(!ctx
        .paths
        .exports
        .join(format!("{}-关联清单.csv", b_off[0].batch_no))
        .exists());

    // ---- ⑥ 关联多到超过一页时会接着分页（25 行关系 > 每页 24 行）----
    let mut linked: Vec<i64> = Vec::new();
    for i in 0..25 {
        linked.push(mk(&ctx, &format!("记-L{i:02}"), Some(100 + i))?);
    }
    for id in &linked {
        links::link(&ctx, a, *id, VoucherLinkKind::Explains, "佐证")?;
    }
    let many = export::export_vouchers_daily(&ctx, &[a], &on)?;
    let pages_many = page_count(&pdf_text(&many[0].pdf_path));
    assert!(
        pages_many >= 3,
        "27 行清单（25 关系 + 2 小计）应至少占 2 页，实测 {pages_many} 页"
    );
    Ok(())
}
