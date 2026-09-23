//! 凭证链路集成测试：CRUD / 批量 / 命名 / 相邻导航 / 搜索 / 报表 / 导出 / 导入去重。

mod common;

use std::path::PathBuf;

use chrono::Datelike;
use common::*;
use voucher_core::db::repos::attachment_repo;
use voucher_core::db::repos::voucher_repo::VoucherQuery;
use voucher_core::models::{ExportFormat, NewVoucher, OcrStatus, VoucherStatus, VoucherUpdate};
use voucher_core::services::{export, ingestion, reports, search, voucher_mgr};
use voucher_core::utils::config::{
    AppConfig, SIDEBAR_WIDTH_DEFAULT, SIDEBAR_WIDTH_MAX, SIDEBAR_WIDTH_MIN,
};
use voucher_core::{CoreContext, CoreError, Result};
#[test]
fn schema_creates_eight_tables() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    ctx.db.with_conn(|conn| {
        let tables: Vec<String> = {
            let mut stmt =
                conn.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
            let mut rows = stmt.query([])?;
            let mut v = Vec::new();
            while let Some(r) = rows.next()? {
                v.push(r.get::<_, String>(0)?);
            }
            v
        };
        for t in [
            "vouchers",
            "attachments",
            "enhance_hits", // v4：D-2 增强版命中埋点
            "export_batches",
            "export_items",
            "snapshots",
            "ocr_cache",
            "config",
            "period_locks", // v6（v0.11 会计期间锁；v0.10 起随 schema 幂等建表）
        ] {
            assert!(tables.iter().any(|x| x == t), "缺表 {t}");
        }
        // v6：export_batches 补 format 列（v0.10 表格式导出）
        let eb_cols: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(export_batches)")?;
            let mut rows = stmt.query([])?;
            let mut v = Vec::new();
            while let Some(r) = rows.next()? {
                v.push(r.get::<_, String>(1)?);
            }
            v
        };
        assert!(
            eb_cols.iter().any(|c| c == "format"),
            "缺列 export_batches.format"
        );
        // v3 升级：documents 增强（5 步流水线 + 来源分类）
        // 验证 attachments 表的列名包含 enhanced_path + enhanced_source
        let cols: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(attachments)")?;
            let mut rows = stmt.query([])?;
            let mut v = Vec::new();
            while let Some(r) = rows.next()? {
                v.push(r.get::<_, String>(1)?);
            }
            v
        };
        assert!(
            cols.iter().any(|c| c == "enhanced_path"),
            "缺列 enhanced_path"
        );
        assert!(
            cols.iter().any(|c| c == "enhanced_source"),
            "缺列 enhanced_source"
        );
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        assert_eq!(v, 4, "user_version 应为 4（v0.6.5 D-2 增强缓存管理）");
        Ok(())
    })
}

#[test]
fn voucher_crud_roundtrip() -> Result<()> {
    let (_dir, ctx) = test_ctx();

    let v = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "八月办公用品".into(),
            amount_cents: Some(12345),
            tags: vec!["办公".into()],
            note: "备注一".into(),
            ..Default::default()
        },
    )?;
    assert!(
        v.voucher_no.ends_with("-01"),
        "编号自动生成: {}",
        v.voucher_no
    );
    assert_eq!(v.status, VoucherStatus::Draft);

    // 同一天第二张 → 02
    let v2 = voucher_mgr::create_voucher(&ctx, NewVoucher::default())?;
    assert!(v2.voucher_no.ends_with("-02"));

    // 更新（金额清空）
    let updated = voucher_mgr::update_voucher(
        &ctx,
        v.id,
        VoucherUpdate {
            description: Some("八月办公用品（修订）".into()),
            clear_amount: true,
            status: Some(VoucherStatus::Reviewed),
            ..Default::default()
        },
    )?;
    assert_eq!(updated.description, "八月办公用品（修订）");
    assert_eq!(updated.amount_cents, None);
    assert_eq!(updated.status, VoucherStatus::Reviewed);

    // 列表 + 统计
    let (items, total) = voucher_mgr::list(
        &ctx,
        &VoucherQuery {
            page: 1,
            page_size: 10,
            ..Default::default()
        },
    )?;
    assert_eq!(total, 2);
    assert_eq!(items.len(), 2);
    let stats = voucher_mgr::stats(&ctx)?;
    assert_eq!(stats.total, 2);
    assert_eq!(stats.reviewed, 1);

    // 删除
    voucher_mgr::delete_voucher(&ctx, v.id)?;
    let (_, total) = voucher_mgr::list(
        &ctx,
        &VoucherQuery {
            page: 1,
            page_size: 10,
            ..Default::default()
        },
    )?;
    assert_eq!(total, 1);
    Ok(())
}

#[test]
fn update_creates_snapshot() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let v = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "原摘要".into(),
            ..Default::default()
        },
    )?;
    voucher_mgr::update_voucher(
        &ctx,
        v.id,
        VoucherUpdate {
            description: Some("新摘要".into()),
            ..Default::default()
        },
    )?;

    let detail = voucher_mgr::get_detail(&ctx, v.id)?;
    assert_eq!(detail.snapshots.len(), 1);
    assert_eq!(detail.snapshots[0].payload["description"], "原摘要");
    assert_eq!(detail.snapshots[0].reason, "update");
    Ok(())
}

#[test]
fn duplicate_voucher_no_rejected() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_no: "MANUAL-1".into(),
            ..Default::default()
        },
    )?;
    let err = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_no: "MANUAL-1".into(),
            ..Default::default()
        },
    );
    assert!(err.is_err());
    Ok(())
}

#[test]
#[ignore = "环境缺模型/字体时自动跳过；CI 以 --include-ignored 全量执行"]
fn export_pdf_and_batch_recorded() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    if voucher_core::services::pdf::find_cjk_font().is_none() {
        eprintln!("跳过：系统无中文字体");
        return Ok(());
    }

    let v = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "测试导出凭证".into(),
            amount_cents: Some(9900),
            tags: vec!["测试".into()],
            ..Default::default()
        },
    )?;

    let batch = export::export_vouchers_pdf(
        &ctx,
        &[v.id],
        &export::ExportOptions {
            title: "凭证整理单".into(),
            card_style: "standard".into(),
            columns: 3,
            include_images: false,
            daily_template: "compact".into(),
        },
    )?;
    assert!(batch.voucher_count == 1);
    assert!(PathBuf::from(&batch.pdf_path).exists(), "PDF 已生成");
    assert!(std::fs::metadata(&batch.pdf_path).unwrap().len() > 1000);

    // 批次可见 + 凭证状态流转
    let batches = export::list_batches(&ctx, 10)?;
    assert_eq!(batches.len(), 1);
    let detail = voucher_mgr::get_detail(&ctx, v.id)?;
    assert_eq!(detail.voucher.status, VoucherStatus::Exported);
    Ok(())
}

#[test]
fn delete_vouchers_cleans_attachment_files() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let img = dir.path().join("receipt.png");
    make_png(&img);
    let summary = ingestion::import_images(&ctx, &[img], None, false)?;
    assert_eq!(summary.vouchers.len(), 1);

    // 附件物理文件存在
    let rel = summary.vouchers[0].attachment.file_path.clone();
    let abs = ctx.paths.attachment_abs(&rel);
    assert!(abs.is_file(), "附件文件应已落盘: {}", abs.display());

    // 造一个增强版文件并登记（模拟 5 步增强已跑过）
    let att_id = summary.vouchers[0].attachment.id;
    let stem = rel.trim_end_matches(".png");
    let enh_rel = format!("{stem}.enhanced.jpg");
    let enh_abs = ctx.paths.attachment_abs(&enh_rel);
    std::fs::copy(&abs, &enh_abs)?;
    ctx.db
        .with_conn(|conn| attachment_repo::set_enhanced(conn, att_id, &enh_rel))?;

    // 批量删除（含一个不存在的 id，验证幂等跳过）
    let vid = summary.vouchers[0].voucher_id;
    let n = voucher_mgr::delete_vouchers(&ctx, &[vid, 99999])?;
    assert_eq!(n, 1, "只应删除实际存在的 1 张");
    assert!(!abs.exists(), "附件物理文件应随凭证删除被清理");
    assert!(
        !enh_abs.exists(),
        "增强版文件应随凭证删除一并清理: {}",
        enh_abs.display()
    );
    let att_count: i64 = ctx.db.with_conn(|conn| {
        conn.query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0))
            .map_err(CoreError::from)
    })?;
    assert_eq!(att_count, 0, "附件行应随外键级联清空");

    // 幂等：再删同一 id 返回 0
    let again = voucher_mgr::delete_vouchers(&ctx, &[vid])?;
    assert_eq!(again, 0);
    Ok(())
}

#[test]
fn import_batch_dedups_same_content() -> Result<()> {
    let (dir, ctx) = test_ctx();
    // 同内容不同文件名的两张图，同一批导入
    let img1 = dir.path().join("a.png");
    let img2 = dir.path().join("b.png");
    make_png(&img1);
    std::fs::copy(&img1, &img2)?;

    let summary = ingestion::import_images(&ctx, &[img1, img2], None, false)?;
    assert_eq!(summary.vouchers.len(), 1, "同批同内容只入库一次");
    assert_eq!(summary.skipped_duplicates.len(), 1);
    Ok(())
}

/// 2026-09-23 缺陷锁（审计 B9）：**并发**导入同一张图也只能入库一条附件记录。
///
/// 旧实现是「先查后插」：查重走 `with_conn`、插入在之后的 `with_tx` 里，两者之间有窗口；
/// 拖放导入与对话框导入都在 `spawn_blocking` 里，真并发时两条记录会指向同一物理文件
/// （rel 由内容 hash 决定），之后删掉其中一张凭证会连带删掉另一张的图。
/// 修法是在插入事务内复查同一 hash。本用例锁住这条不变量。
#[test]
fn concurrent_import_of_same_image_creates_single_attachment() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let src = dir.path().join("same.png");
    make_png(&src);
    // 两次「同时」导入同一文件：线程各自调用同一入口（模拟拖放与对话框并发）
    let (r1, r2) = std::thread::scope(|s| {
        let h1 =
            s.spawn(|| ingestion::import_images(&ctx, std::slice::from_ref(&src), None, false));
        let h2 =
            s.spawn(|| ingestion::import_images(&ctx, std::slice::from_ref(&src), None, false));
        (
            h1.join().expect("线程 1 不应 panic"),
            h2.join().expect("线程 2 不应 panic"),
        )
    });
    let r1 = r1?;
    let r2 = r2?;

    let rows: i64 = ctx.db.with_conn(|conn| {
        Ok(conn.query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0))?)
    })?;
    assert_eq!(
        rows,
        1,
        "并发导入同一张图只应有一条附件记录（两线程各得汇总：{:?}/{:?}）",
        r1.vouchers.len(),
        r2.vouchers.len()
    );
    // 两侧加起来的入库数必须是 1，另一侧应判为重复跳过（而非失败）
    assert_eq!(
        r1.vouchers.len() + r2.vouchers.len(),
        1,
        "合计入库数必须为 1"
    );
    assert_eq!(
        r1.failed.len() + r2.failed.len(),
        0,
        "重复侧应计入 skipped_duplicates 而不是 failed"
    );
    Ok(())
}

#[test]
fn set_status_creates_snapshot() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let v = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "状态流转测试".into(),
            ..Default::default()
        },
    )?;
    voucher_mgr::set_status(&ctx, v.id, VoucherStatus::Reviewed)?;

    let detail = voucher_mgr::get_detail(&ctx, v.id)?;
    assert_eq!(detail.voucher.status, VoucherStatus::Reviewed);
    assert_eq!(detail.snapshots.len(), 1, "状态变更应落快照");
    assert_eq!(detail.snapshots[0].reason, "status");
    assert_eq!(detail.snapshots[0].payload["description"], "状态流转测试");
    Ok(())
}

/// v0.7 B-1：批量状态流转——单事务逐张快照、幂等跳过、不存在跳过
#[test]
fn batch_set_status_snapshots_and_skips() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let v1 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "批1".into(),
            ..Default::default()
        },
    )?;
    let v2 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "批2".into(),
            ..Default::default()
        },
    )?;
    // v2 先单独复核，批量时已是目标状态 → 应跳过
    voucher_mgr::set_status(&ctx, v2.id, VoucherStatus::Reviewed)?;

    // 批量复核：v1 生效、v2 幂等跳过、99999 不存在跳过
    let n = voucher_mgr::batch_set_status(&ctx, &[v1.id, v2.id, 99999], VoucherStatus::Reviewed)?;
    assert_eq!(n, 1, "只有 v1 实际变更");

    let d1 = voucher_mgr::get_detail(&ctx, v1.id)?;
    assert_eq!(d1.voucher.status, VoucherStatus::Reviewed);
    assert_eq!(d1.snapshots.len(), 1, "v1 应有一条 batch_status 快照");
    assert_eq!(d1.snapshots[0].reason, "batch_status");

    let d2 = voucher_mgr::get_detail(&ctx, v2.id)?;
    assert_eq!(d2.snapshots.len(), 1, "v2 不应因幂等跳过产生新快照");

    // 空列表 → 0
    assert_eq!(
        voucher_mgr::batch_set_status(&ctx, &[], VoucherStatus::Archived)?,
        0
    );
    Ok(())
}

/// 功能深化 2：批量编辑——备注/标签替换、幂等跳过、空补丁报错
#[test]
fn batch_edit_notes_tags_and_skips() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let v1 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "批1".into(),
            ..Default::default()
        },
    )?;
    let v2 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "批2".into(),
            tags: vec!["已有标签".into()],
            ..Default::default()
        },
    )?;

    let n = voucher_mgr::batch_edit(
        &ctx,
        &[v1.id, v2.id, 99999],
        &voucher_mgr::BatchEditPatch {
            note: Some("统一备注".into()),
            tags: Some(vec!["差旅".into(), "报销".into()]),
            ..Default::default()
        },
    )?;
    assert_eq!(n, 2, "v1/v2 变更，不存在的 99999 跳过");

    let d1 = voucher_mgr::get_detail(&ctx, v1.id)?;
    assert_eq!(d1.voucher.note, "统一备注");
    assert_eq!(
        d1.voucher.tags,
        vec!["差旅".to_string(), "报销".to_string()]
    );
    assert_eq!(d1.snapshots.len(), 1, "v1 一条 batch_edit 快照");
    assert_eq!(d1.snapshots[0].reason, "batch_edit");
    let d2 = voucher_mgr::get_detail(&ctx, v2.id)?;
    assert_eq!(
        d2.voucher.tags,
        vec!["差旅".to_string(), "报销".to_string()],
        "标签整体替换而非追加"
    );

    // 幂等：同补丁再来一遍 → 0 变更、无新快照
    let n2 = voucher_mgr::batch_edit(
        &ctx,
        &[v1.id, v2.id],
        &voucher_mgr::BatchEditPatch {
            note: Some("统一备注".into()),
            tags: Some(vec!["差旅".into(), "报销".into()]),
            ..Default::default()
        },
    )?;
    assert_eq!(n2, 0);
    assert_eq!(voucher_mgr::get_detail(&ctx, v1.id)?.snapshots.len(), 1);

    // 空补丁 → 报错（防误触）
    assert!(matches!(
        voucher_mgr::batch_edit(&ctx, &[v1.id], &voucher_mgr::BatchEditPatch::default()),
        Err(CoreError::Invalid(_))
    ));
    // 空列表 → 0
    assert_eq!(
        voucher_mgr::batch_edit(
            &ctx,
            &[],
            &voucher_mgr::BatchEditPatch {
                note: Some("x".into()),
                ..Default::default()
            }
        )?,
        0
    );
    Ok(())
}

/// 功能深化 2：批量改日期——档号按新日期重排、非法日期拒绝
#[test]
fn batch_edit_date_renumbers() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let v = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "改日期".into(),
            ..Default::default()
        },
    )?;
    let orig_no = v.voucher_no.clone();

    let n = voucher_mgr::batch_edit(
        &ctx,
        &[v.id],
        &voucher_mgr::BatchEditPatch {
            voucher_date: Some("2026-01-15".into()),
            ..Default::default()
        },
    )?;
    assert_eq!(n, 1);
    let d = voucher_mgr::get_detail(&ctx, v.id)?;
    assert_eq!(d.voucher.voucher_date, "2026-01-15");
    assert_ne!(d.voucher.voucher_no, orig_no, "档号应随日期重排");
    assert!(
        d.voucher.voucher_no.contains("2026-01-15") || d.voucher.voucher_no.contains("20260115"),
        "新档号应基于新日期：{}",
        d.voucher.voucher_no
    );
    // 两条快照：batch_edit + renumber
    assert!(d.snapshots.iter().any(|s| s.reason == "renumber"));

    // 非法日期 → 整个事务拒绝（凭证未被改动）
    assert!(matches!(
        voucher_mgr::batch_edit(
            &ctx,
            &[v.id],
            &voucher_mgr::BatchEditPatch {
                voucher_date: Some("2026-02-30".into()),
                ..Default::default()
            }
        ),
        Err(CoreError::Invalid(_))
    ));
    assert_eq!(
        voucher_mgr::get_detail(&ctx, v.id)?.voucher.voucher_date,
        "2026-01-15"
    );
    Ok(())
}

/// v0.7 R-1：Top 对方账户——按金额降序、空名排除、limit 生效
#[test]
fn top_counterparties_orders_by_amount() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let mk = |desc: &str, cp: &str, cents: i64| {
        voucher_mgr::create_voucher(
            &ctx,
            NewVoucher {
                description: desc.into(),
                counterparty: cp.into(),
                amount_cents: Some(cents),
                ..Default::default()
            },
        )
    };
    mk("a", "美团", 5000)?;
    mk("b", "美团", 3000)?;
    mk("c", "京东", 9000)?;
    mk("d", "", 12345)?; // 空对方：不进榜
    mk("e", "滴滴", 100)?;

    // 全部历史（months=0）
    let all = voucher_core::services::reports::top_counterparties(&ctx, 0, 10)?;
    assert_eq!(all.len(), 3, "空名排除，三个对方入榜");
    assert_eq!(all[0].name, "京东");
    assert_eq!(all[0].amount_cents, 9000);
    assert_eq!(all[1].name, "美团");
    assert_eq!(all[1].count, 2, "美团两张合计 8000");
    assert_eq!(all[1].amount_cents, 8000);
    assert_eq!(all[2].name, "滴滴");

    // limit 截断
    let top1 = voucher_core::services::reports::top_counterparties(&ctx, 0, 1)?;
    assert_eq!(top1.len(), 1);
    assert_eq!(top1[0].name, "京东");
    Ok(())
}

/// 功能深化 3：标签分类统计——按金额降序、多标签各计一次、未分类末尾不占名额、limit 生效
#[test]
fn tag_stats_orders_by_amount() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let mk = |desc: &str, tags: Vec<&str>, cents: i64| {
        voucher_mgr::create_voucher(
            &ctx,
            NewVoucher {
                description: desc.into(),
                tags: tags.into_iter().map(String::from).collect(),
                amount_cents: Some(cents),
                ..Default::default()
            },
        )
    };
    mk("a", vec!["差旅", "报销"], 10000)?; // 双标签：两个分类各计 1 次
    mk("b", vec!["差旅"], 5000)?;
    mk("c", vec![], 3000)?; // 无标签 → 未分类

    let rows = voucher_core::services::reports::tag_stats(&ctx, 0, 10)?;
    assert_eq!(rows.len(), 3, "差旅/报销/未分类");
    assert_eq!(rows[0].tag, "差旅");
    assert_eq!(rows[0].count, 2, "双标签凭证在差旅下计 2 张");
    assert_eq!(rows[0].amount_cents, 15000);
    assert_eq!(rows[1].tag, "报销");
    assert_eq!(rows[1].count, 1);
    assert_eq!(rows[1].amount_cents, 10000);
    assert_eq!(rows[2].tag, "", "未分类固定末尾");
    assert_eq!(rows[2].count, 1);
    assert_eq!(rows[2].amount_cents, 3000);

    // limit 截断：只留差旅；未分类不占名额仍返回
    let top1 = voucher_core::services::reports::tag_stats(&ctx, 0, 1)?;
    assert_eq!(top1.len(), 2, "limit=1 → 差旅 + 未分类（不占名额）");
    assert_eq!(top1[0].tag, "差旅");
    assert_eq!(top1[1].tag, "");

    // 近 1 月窗口（凭证默认今天）结果一致；空白标签不入榜、计入未分类
    mk("d", vec!["  "], 7000)?; // 空白标签：不单独入榜，凭证算「无有效标签」
    let m1 = voucher_core::services::reports::tag_stats(&ctx, 1, 10)?;
    let tagged_names: Vec<&str> = m1
        .iter()
        .filter(|r| !r.tag.is_empty())
        .map(|r| r.tag.as_str())
        .collect();
    assert_eq!(tagged_names, vec!["差旅", "报销"], "空白标签不单独成行");
    assert_eq!(m1.last().unwrap().count, 2, "c 与 d 均无有效标签");
    Ok(())
}

#[test]
fn update_with_blank_voucher_no_keeps_old() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let v = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "空编号防御".into(),
            ..Default::default()
        },
    )?;
    let updated = voucher_mgr::update_voucher(
        &ctx,
        v.id,
        VoucherUpdate {
            voucher_no: Some("   ".into()),
            description: Some("空编号防御（修订）".into()),
            ..Default::default()
        },
    )?;
    assert_eq!(updated.voucher_no, v.voucher_no, "空白编号应视为不修改");
    assert_eq!(updated.description, "空编号防御（修订）");
    Ok(())
}

#[test]
fn search_escapes_like_wildcards() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "进度100%完成".into(),
            ..Default::default()
        },
    )?;
    voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "普通摘要".into(),
            ..Default::default()
        },
    )?;

    // '%' 应只命中字面含 % 的凭证，而非充当通配符匹配全部
    let hits = search::search_vouchers(&ctx, "%", &Default::default(), 10)?;
    assert_eq!(hits.len(), 1, "关键词 % 不应被当作通配符");
    assert_eq!(hits[0].voucher.description, "进度100%完成");

    // '_' 同理：无人含字面下划线 → 零命中
    let hits2 = search::search_vouchers(&ctx, "_", &Default::default(), 10)?;
    assert_eq!(hits2.len(), 0, "关键词 _ 不应匹配所有非空文本");
    Ok(())
}

#[test]
fn naming_seq_skips_holes_after_deletion() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    // 同日建三张：01 / 02 / 03
    let d = "2026-08-20";
    let v1 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: d.into(),
            ..Default::default()
        },
    )?;
    let v2 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: d.into(),
            ..Default::default()
        },
    )?;
    let v3 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: d.into(),
            ..Default::default()
        },
    )?;
    assert!(v1.voucher_no.ends_with("01"));
    assert!(v2.voucher_no.ends_with("02"));
    assert!(v3.voucher_no.ends_with("03"));

    // 删中间的 02 → count=2 → 朴素 count+1 会算出 03 撞 UNIQUE
    voucher_mgr::delete_voucher(&ctx, v2.id)?;
    let v4 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: d.into(),
            ..Default::default()
        },
    )?;
    assert!(
        v4.voucher_no.ends_with("04"),
        "应跳过空洞递增到 04，实际 {}",
        v4.voucher_no
    );
    Ok(())
}

#[test]
fn voucher_no_renumbers_on_date_change() -> Result<()> {
    let (_dir, ctx) = test_ctx();

    // 2026-08-20 已有一张 → -01
    let existing = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-20".into(),
            ..Default::default()
        },
    )?;
    assert!(existing.voucher_no.ends_with("2026-08-20-01"));

    // 2026-08-21 一张（-01），改日期到 08-20 → 重排为 -02（档号日期以凭证日期为准）
    let v = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-21".into(),
            ..Default::default()
        },
    )?;
    let moved = voucher_mgr::update_voucher(
        &ctx,
        v.id,
        VoucherUpdate {
            voucher_date: Some("2026-08-20".into()),
            ..Default::default()
        },
    )?;
    assert_eq!(moved.voucher_no, "2026-08-20-02");

    // 重排落了 renumber 快照
    let detail = voucher_mgr::get_detail(&ctx, v.id)?;
    assert!(
        detail.snapshots.iter().any(|s| s.reason == "renumber"),
        "重排应落快照，实际 reasons: {:?}",
        detail
            .snapshots
            .iter()
            .map(|s| s.reason.as_str())
            .collect::<Vec<_>>()
    );

    // 已导出的凭证改日期 → 编号锁定不重排
    let locked = voucher_mgr::update_voucher(
        &ctx,
        v.id,
        VoucherUpdate {
            voucher_date: Some("2026-08-22".into()),
            status: Some(VoucherStatus::Exported),
            ..Default::default()
        },
    )?;
    assert_eq!(locked.voucher_date, "2026-08-22");
    assert_eq!(locked.voucher_no, "2026-08-20-02", "已导出编号应锁定");

    Ok(())
}

#[test]
fn export_creates_snapshot_and_marks_exported() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let v = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "导出快照测试".into(),
            ..Default::default()
        },
    )?;

    let batch = export::export_with_defaults(&ctx, &[v.id])?;
    assert_eq!(batch.voucher_count, 1);
    assert!(batch.pdf_path.ends_with(".pdf"));

    // 状态已流转 + 导出前快照存在
    let detail = voucher_mgr::get_detail(&ctx, v.id)?;
    assert_eq!(detail.voucher.status, VoucherStatus::Exported);
    assert!(
        detail.snapshots.iter().any(|s| s.reason == "export"),
        "导出改状态应落快照，实际 reasons: {:?}",
        detail
            .snapshots
            .iter()
            .map(|s| s.reason.as_str())
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn config_roundtrip_via_context() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let mut cfg = ctx.config();
    cfg.naming.template = "{date}-K{seq}".into();
    cfg.print.columns = 2;
    ctx.set_config(cfg)?;

    // 新上下文加载同一目录 → 持久化生效
    let ctx2 = CoreContext::init(Some(dir.path().to_path_buf()))?;
    assert_eq!(ctx2.config().naming.template, "{date}-K{seq}");
    assert_eq!(ctx2.config().print.columns, 2);
    Ok(())
}

#[test]
fn sidebar_width_is_clamped_on_write_and_read() -> Result<()> {
    let (dir, ctx) = test_ctx();
    // 越界值写入即被钳制：手改 app.json 不得破坏布局
    let mut cfg = ctx.config();
    cfg.appearance.sidebar_width = 9999;
    ctx.set_config(cfg)?;
    assert_eq!(ctx.config().appearance.sidebar_width, SIDEBAR_WIDTH_MAX);

    let mut cfg = ctx.config();
    cfg.appearance.sidebar_width = 10;
    ctx.set_config(cfg)?;
    assert_eq!(ctx.config().appearance.sidebar_width, SIDEBAR_WIDTH_MIN);

    // 合法值写盘后能在新上下文读回
    let mut cfg = ctx.config();
    cfg.appearance.sidebar_width = 320;
    ctx.set_config(cfg)?;
    let ctx2 = CoreContext::init(Some(dir.path().to_path_buf()))?;
    assert_eq!(ctx2.config().appearance.sidebar_width, 320);
    Ok(())
}

#[test]
fn legacy_config_without_sidebar_width_uses_default() -> Result<()> {
    let (dir, _ctx) = test_ctx();
    // 旧格式 app.json：appearance 只有 theme、没有 sidebarWidth（容器级 serde default 补默认值）
    let paths = voucher_core::AppPaths::resolve(Some(dir.path().to_path_buf()))?;
    std::fs::write(paths.config_file(), r#"{"appearance":{"theme":"dark"}}"#)
        .expect("写旧格式配置");

    let loaded = AppConfig::load(&paths.config_file())?;
    assert_eq!(loaded.appearance.theme, "dark");
    assert_eq!(loaded.appearance.sidebar_width, SIDEBAR_WIDTH_DEFAULT);
    Ok(())
}

#[test]
fn monthly_summary_pivots_and_zero_fills() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let today = chrono::Local::now().date_naive();
    let this_month = today.format("%Y-%m").to_string();
    // 上月同位日期：先回到本月 1 日再取前一天，任何「今天」都成立
    let prev_day = today
        .with_day(1)
        .and_then(|d| d.pred_opt())
        .expect("上月日期");
    let prev_month = prev_day.format("%Y-%m").to_string();
    let this_day = today.format("%Y-%m-%d").to_string();
    let prev_date = prev_day.format("%Y-%m-%d").to_string();

    // 本月 3 张（2 草稿含金额 + 1 已导出空金额），上月 1 张已复核
    voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: this_day.clone(),
            amount_cents: Some(1000),
            ..Default::default()
        },
    )?;
    voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: this_day.clone(),
            amount_cents: Some(2500),
            ..Default::default()
        },
    )?;
    let v3 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: this_day,
            ..Default::default()
        },
    )?;
    voucher_mgr::set_status(&ctx, v3.id, VoucherStatus::Exported)?;
    let v4 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: prev_date,
            amount_cents: Some(999),
            ..Default::default()
        },
    )?;
    voucher_mgr::set_status(&ctx, v4.id, VoucherStatus::Reviewed)?;

    // 近 6 月：长度固定、升序、空月补零
    let s6 = reports::monthly_summary(&ctx, 6)?;
    assert_eq!(s6.len(), 6, "空月也应出现（补零）");
    assert_eq!(s6[5].month, this_month, "末位应为当月");
    assert_eq!(s6[4].month, prev_month, "倒数第二位应为上月");
    assert!(
        s6.iter()
            .all(|m| { m.count == m.draft + m.reviewed + m.exported + m.archived }),
        "状态分布之和应等于张数"
    );

    // 当月透视与金额（空金额不计入合计）
    let cur = &s6[5];
    assert_eq!(cur.count, 3);
    assert_eq!(cur.amount_cents, 3500);
    assert_eq!(cur.draft, 2);
    assert_eq!(cur.exported, 1);
    assert_eq!(cur.reviewed, 0);

    // 上月
    let prev = &s6[4];
    assert_eq!(prev.count, 1);
    assert_eq!(prev.amount_cents, 999);
    assert_eq!(prev.reviewed, 1);

    // 窗口收缩到 1 → 只剩当月（窗口外的上月数据不可见）；0 → 空
    let s1 = reports::monthly_summary(&ctx, 1)?;
    assert_eq!(s1.len(), 1);
    assert_eq!(s1[0].month, this_month);
    assert_eq!(s1[0].count, 3);
    assert!(reports::monthly_summary(&ctx, 0)?.is_empty());
    Ok(())
}

#[test]
fn file_lock_prevents_second_instance() -> Result<()> {
    let (dir, _ctx) = test_ctx();
    let lock_path = dir.path().join(".lock2");
    let lock = voucher_core::utils::file_lock::FileLockGuard::try_acquire(&lock_path)?;
    assert!(voucher_core::utils::file_lock::FileLockGuard::try_acquire(&lock_path).is_err());
    drop(lock);
    Ok(())
}

#[test]
fn one_image_creates_one_voucher_each() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let imgs: Vec<PathBuf> = (0..3)
        .map(|i| {
            let p = dir.path().join(format!("receipt-{i}.png"));
            make_png_variant(&p, 40 + i as u8 * 30);
            p
        })
        .collect();

    let summary = ingestion::import_images(&ctx, &imgs, None, false)?;
    assert_eq!(summary.vouchers.len(), 3, "3 张图各建一张凭证");
    assert_eq!(voucher_count(&ctx)?, 3);

    // 每张凭证恰有 1 个附件（DB 唯一索引的语义层面验证）
    for v in &summary.vouchers {
        let detail = voucher_mgr::get_detail(&ctx, v.voucher_id)?;
        assert_eq!(detail.attachments.len(), 1);
        assert_eq!(detail.voucher.status, VoucherStatus::Draft);
    }

    // 凭证两两不同
    let ids: std::collections::HashSet<i64> =
        summary.vouchers.iter().map(|v| v.voucher_id).collect();
    assert_eq!(ids.len(), 3);
    Ok(())
}

/// G3-3 回归：同内容文件二次导入不产生空凭证（空卡缺陷的根治验证）
#[test]
fn g3_3_duplicate_reimport_creates_no_empty_voucher() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let img = dir.path().join("dup.png");
    make_png(&img);

    let s1 = ingestion::import_images(&ctx, std::slice::from_ref(&img), None, false)?;
    assert_eq!(s1.vouchers.len(), 1);
    let count_after_first = voucher_count(&ctx)?;

    // 同内容文件换名再导入 → 跳过，凭证数不变，无空卡
    let renamed = dir.path().join("dup-copy.png");
    std::fs::copy(&img, &renamed)?;
    let s2 = ingestion::import_images(&ctx, &[renamed], None, false)?;
    assert_eq!(s2.vouchers.len(), 0);
    assert_eq!(s2.skipped_duplicates.len(), 1);
    assert_eq!(
        voucher_count(&ctx)?,
        count_after_first,
        "重复导入不得新增凭证"
    );

    // 全库无「零附件凭证」（空卡）
    let empty: i64 = ctx.db.with_conn(|conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM vouchers v WHERE NOT EXISTS (SELECT 1 FROM attachments a WHERE a.voucher_id = v.id)",
            [],
            |r| r.get(0),
        )
        .map_err(CoreError::from)
    })?;
    assert_eq!(empty, 0, "G3-3：库中不应存在无附件的空凭证");
    Ok(())
}

/// 附加模式一图上限：已有图片的凭证拒绝再挂图
#[test]
fn attach_mode_rejects_second_image() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let img1 = dir.path().join("a.png");
    let img2 = dir.path().join("b.png");
    make_png_variant(&img1, 50);
    make_png_variant(&img2, 120);

    let s = ingestion::import_images(&ctx, &[img1], None, false)?;
    let vid = s.vouchers[0].voucher_id;

    // 附加模式：目标已有图 → 拒绝
    let img2b = dir.path().join("b2.png");
    std::fs::copy(&img2, &img2b)?;
    let err = ingestion::import_images(&ctx, &[img2b], Some(vid), false);
    assert!(err.is_err(), "一图一凭证：已有图的凭证不能再挂图");

    // 批量附加模式：首图成功，其余失败（不转新建）
    let img3 = dir.path().join("c.png");
    make_png_variant(&img3, 200);
    let v2 = voucher_mgr::create_voucher(&ctx, NewVoucher::default())?;
    let s2 = ingestion::import_images(&ctx, &[img2, img3], Some(v2.id), false)?;
    assert_eq!(s2.vouchers.len(), 1, "附加模式仅第一张入库");
    assert_eq!(s2.failed.len(), 1, "第二张应报失败而非新建凭证");
    let detail = voucher_mgr::get_detail(&ctx, v2.id)?;
    assert_eq!(detail.attachments.len(), 1);
    Ok(())
}

/// 更换图片：换图不换卡（旧图清理、新哈希落库、快照可追溯）
#[test]
fn replace_attachment_swaps_image() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let img1 = dir.path().join("old.png");
    let img2 = dir.path().join("new.png");
    make_png_variant(&img1, 50);
    make_png_variant(&img2, 140);

    let s = ingestion::import_images(&ctx, std::slice::from_ref(&img1), None, false)?;
    let vid = s.vouchers[0].voucher_id;
    let old_att = s.vouchers[0].attachment.clone();
    let old_abs = ctx.paths.attachment_abs(&old_att.file_path);
    assert!(old_abs.is_file());

    // 同内容换图 → 幂等成功（不换）
    let same = ingestion::replace_attachment(&ctx, vid, &img1)?;
    assert_eq!(same.file_hash, old_att.file_hash);

    // 换新图：凭证不换（仍 1 张卡），附件行替换，旧物理文件清理
    let new_att = ingestion::replace_attachment(&ctx, vid, &img2)?;
    assert_ne!(new_att.file_hash, old_att.file_hash);
    assert!(!old_abs.exists(), "旧物理文件应被清理");
    let detail = voucher_mgr::get_detail(&ctx, vid)?;
    assert_eq!(detail.attachments.len(), 1, "换图不换卡，附件仍 1 张");
    assert_eq!(detail.attachments[0].file_hash, new_att.file_hash);

    // 快照留痕
    assert!(
        detail.snapshots.iter().any(|s| s.reason == "replace_image"),
        "换图应落快照，实际: {:?}",
        detail
            .snapshots
            .iter()
            .map(|s| s.reason.as_str())
            .collect::<Vec<_>>()
    );

    // 新图已存在于其他凭证 → 拒绝（内容唯一性）
    let v3 = voucher_mgr::create_voucher(&ctx, NewVoucher::default())?;
    let img3 = dir.path().join("third.png");
    make_png_variant(&img3, 220);
    let _ = ingestion::import_images(&ctx, &[img3], Some(v3.id), false)?;
    let img2c = dir.path().join("new-copy.png");
    std::fs::copy(&img2, &img2c)?;
    let err = ingestion::replace_attachment(&ctx, v3.id, &img2c);
    assert!(err.is_err(), "新图已存在于其他凭证时应拒绝");
    Ok(())
}

/// 按天分组导出：不同日期的凭证各生成一份日报 PDF + 各自批次记录
#[test]
#[ignore = "环境缺模型/字体时自动跳过；CI 以 --include-ignored 全量执行"]
fn export_daily_groups_by_date() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    if voucher_core::services::pdf::find_cjk_font().is_none() {
        eprintln!("跳过：系统无中文字体");
        return Ok(());
    }

    let d1 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-01".into(),
            description: "日报A1".into(),
            counterparty: "文一商店".into(),
            amount_cents: Some(1000),
            ..Default::default()
        },
    )?;
    let d1b = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-01".into(),
            description: "日报A2".into(),
            amount_cents: Some(2500),
            ..Default::default()
        },
    )?;
    let d2 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-02".into(),
            description: "日报B1".into(),
            amount_cents: Some(500),
            ..Default::default()
        },
    )?;

    let batches = export::export_vouchers_daily(
        &ctx,
        &[d2.id, d1.id, d1b.id], // 乱序传入，验证按日期升序输出
        &export::ExportOptions {
            title: "凭证日报".into(),
            card_style: "standard".into(),
            columns: 3,
            include_images: false,
            daily_template: "compact".into(),
        },
    )?;
    assert_eq!(batches.len(), 2, "两个日期应各出一份日报");
    // BTreeMap 升序：旧日期在前
    assert!(
        batches[0].batch_no.contains("EXP-D20260801"),
        "批次号带日期段: {}",
        batches[0].batch_no
    );
    assert!(batches[1].batch_no.contains("EXP-D20260802"));
    assert_eq!(batches[0].voucher_count, 2);
    assert_eq!(batches[1].voucher_count, 1);
    for b in &batches {
        assert!(PathBuf::from(&b.pdf_path).exists(), "日报 PDF 应落盘");
        assert!(std::fs::metadata(&b.pdf_path)?.len() > 1000);
    }

    // 批次记录与状态流转
    let all = export::list_batches(&ctx, 10)?;
    assert_eq!(all.len(), 2);
    for id in [d1.id, d1b.id, d2.id] {
        let detail = voucher_mgr::get_detail(&ctx, id)?;
        assert_eq!(detail.voucher.status, VoucherStatus::Exported);
    }
    Ok(())
}

/// v0.9 编辑动线：同筛选/同排序下的相邻凭证与位置（「保存并下一张」后端）
#[test]
fn neighbor_matches_list_order_and_filters() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    // 构造：08-01 两张（同日按 id 倒序）、08-02 一张；08-02 那张已复核
    let a = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-01".into(),
            description: "A".into(),
            ..Default::default()
        },
    )?;
    let b = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-01".into(),
            description: "B".into(),
            ..Default::default()
        },
    )?;
    let c = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-02".into(),
            description: "C".into(),
            ..Default::default()
        },
    )?;
    voucher_mgr::set_status(&ctx, c.id, VoucherStatus::Reviewed)?;

    // 列表序（DESC, DESC）：c(08-02) > b(08-01,id大) > a(08-01,id小)
    let (items, total) = voucher_mgr::list(
        &ctx,
        &VoucherQuery {
            page: 1,
            page_size: 10,
            ..Default::default()
        },
    )?;
    assert_eq!(total, 3);
    let order: Vec<i64> = items.iter().map(|v| v.id).collect();
    assert_eq!(order, vec![c.id, b.id, a.id], "列表序前提");

    // 中间张：上一张 = c，下一张 = a，位置 2/3
    let mid = voucher_mgr::neighbor(&ctx, b.id, &VoucherQuery::default())?;
    assert_eq!(mid.prev_id, Some(c.id));
    assert_eq!(mid.next_id, Some(a.id));
    assert_eq!(mid.position, 2);
    assert_eq!(mid.total, 3);

    // 首张：无上一张；末张：无下一张
    let first = voucher_mgr::neighbor(&ctx, c.id, &VoucherQuery::default())?;
    assert_eq!(first.prev_id, None);
    assert_eq!(first.next_id, Some(b.id));
    assert_eq!(first.position, 1);
    let last = voucher_mgr::neighbor(&ctx, a.id, &VoucherQuery::default())?;
    assert_eq!(last.prev_id, Some(b.id));
    assert_eq!(last.next_id, None);
    assert_eq!(last.position, 3);

    // 筛选 = 草稿：只剩 b > a；当前是已复核的 c（不在筛选内）→
    // 下一张仍指向筛选内的 b（键位基准，支撑「改状态后继续核对草稿」）
    let draft_q = VoucherQuery {
        status: Some(VoucherStatus::Draft),
        ..Default::default()
    };
    let from_c = voucher_mgr::neighbor(&ctx, c.id, &draft_q)?;
    assert_eq!(
        from_c.next_id,
        Some(b.id),
        "已复核凭证在草稿筛选下仍有下一张"
    );
    assert_eq!(from_c.prev_id, None);
    assert_eq!(from_c.total, 2);

    // 关键词筛选
    let kw_q = VoucherQuery {
        keyword: Some("A".into()),
        ..Default::default()
    };
    let kw = voucher_mgr::neighbor(&ctx, a.id, &kw_q)?;
    assert_eq!(kw.total, 1);
    assert_eq!(kw.position, 1);
    assert_eq!(kw.prev_id, None);
    assert_eq!(kw.next_id, None);

    // 不存在的凭证 → NotFound
    assert!(matches!(
        voucher_mgr::neighbor(&ctx, 99999, &VoucherQuery::default()),
        Err(CoreError::NotFound(_))
    ));
    Ok(())
}

/// v0.9 搜索动线：include_ocr 口径——「下一张」与搜索结果（含 OCR 命中）同序
#[test]
fn neighbor_include_ocr_follows_search_scope() -> Result<()> {
    let (dir, ctx) = test_ctx();
    // 两张凭证：摘要不含关键词、OCR 文本含关键词；中间夹一张无关凭证
    let a = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-01".into(),
            description: "甲".into(),
            ..Default::default()
        },
    )?;
    let mid = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-02".into(),
            description: "无关".into(),
            ..Default::default()
        },
    )?;
    let b = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-03".into(),
            description: "乙".into(),
            ..Default::default()
        },
    )?;
    // 给 a / b 挂带 OCR 文本的附件（一图一凭证：导入即建卡）
    for (v, shade) in [(a.id, 60u8), (b.id, 180u8)] {
        let img = dir.path().join(format!("ocr-{v}.png"));
        let pic = image::RgbImage::from_pixel(64, 48, image::Rgb([shade, 30, 200]));
        pic.save(&img).expect("save png");
        let s = ingestion::import_images(&ctx, std::slice::from_ref(&img), Some(v), false)?;
        assert_eq!(s.vouchers.len(), 1);
        ctx.db.with_conn(|conn| {
            voucher_core::db::repos::attachment_repo::set_ocr(
                conn,
                s.vouchers[0].attachment.id,
                OcrStatus::Done,
                Some("出租车票 45.00 元"),
            )
        })?;
    }

    // 字段口径（列表页搜索）：关键词「出租车」零命中
    let field_q = VoucherQuery {
        keyword: Some("出租车".into()),
        ..Default::default()
    };
    let (_, total) = voucher_mgr::list(&ctx, &field_q)?;
    assert_eq!(total, 0);

    // OCR 口径（搜索页）：a、b 命中（mid 无附件不命中）
    let ocr_q = VoucherQuery {
        keyword: Some("出租车".into()),
        include_ocr: true,
        ..Default::default()
    };
    let (items, total) = voucher_mgr::list(&ctx, &ocr_q)?;
    assert_eq!(total, 2);
    assert_eq!(items.len(), 2, "仅 OCR 命中的 a/b 可见");
    // 列表序：b(08-03) 在 a(08-01) 前；无附件的 mid 不混入
    let ids: Vec<i64> = items.iter().map(|v| v.id).collect();
    assert_eq!(ids, vec![b.id, a.id]);
    assert!(!ids.contains(&mid.id), "无附件的 mid 不在 OCR 命中结果内");

    // 从 a 的「上一张」应到 b（同口径），位置/总数与搜索结果一致
    let n = voucher_mgr::neighbor(&ctx, a.id, &ocr_q)?;
    assert_eq!(n.prev_id, Some(b.id), "OCR 口径下 a 的上一张是 b");
    assert_eq!(n.next_id, None);
    assert_eq!(n.position, 2);
    assert_eq!(n.total, 2);

    // 字段口径下同凭证：无邻居（零命中集合内无相邻）
    let n2 = voucher_mgr::neighbor(&ctx, a.id, &field_q)?;
    assert_eq!(n2.prev_id, None);
    assert_eq!(n2.total, 0);
    Ok(())
}

// ---------- v0.10：表格式导出 / 疑似重复检测 / 高频摘要 ----------

/// v0.10：Excel / CSV 表格式导出——文件落盘、批次记录格式、状态流转与 PDF 同口径
#[test]
fn export_table_xlsx_and_csv_records_batch() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let a = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "办公用品,采购".into(),
            counterparty: "文一店".into(),
            amount_cents: Some(12345),
            ..Default::default()
        },
    )?;
    let b = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "无金额凭证".into(),
            amount_cents: None,
            ..Default::default()
        },
    )?;

    // XLSX
    let bx = export::export_vouchers_table(&ctx, &[a.id, b.id], ExportFormat::Xlsx)?;
    assert_eq!(bx.voucher_count, 2);
    assert!(PathBuf::from(&bx.pdf_path).exists(), "xlsx 已生成");
    assert!(bx.pdf_path.ends_with(".xlsx"));
    assert!(
        std::fs::metadata(&bx.pdf_path).unwrap().len() > 1000,
        "合法 zip 容器"
    );
    assert!(
        bx.batch_no.contains("EXP-X"),
        "xlsx 批次号前缀: {}",
        bx.batch_no
    );

    // CSV（含 BOM）
    let bc = export::export_vouchers_table(&ctx, &[a.id], ExportFormat::Csv)?;
    assert!(bc.pdf_path.ends_with(".csv"));
    assert!(
        bc.batch_no.contains("EXP-C"),
        "csv 批次号前缀: {}",
        bc.batch_no
    );
    let bytes = std::fs::read(&bc.pdf_path)?;
    assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF], "CSV 带 UTF-8 BOM");
    let text = String::from_utf8_lossy(&bytes[3..]);
    assert!(text.contains("凭证号,日期,"));
    assert!(
        text.contains("\"办公用品,采购\""),
        "含逗号字段 RFC4180 转义"
    );
    assert!(text.contains("123.45"), "金额数值口径");

    // 批次表：格式字段 + 列表可见；凭证状态流转与 PDF 一致（非归档 → 已导出）
    let batches = export::list_batches(&ctx, 10)?;
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].format, ExportFormat::Csv, "最新为 csv");
    assert_eq!(batches[1].format, ExportFormat::Xlsx);
    let detail = voucher_mgr::get_detail(&ctx, a.id)?;
    assert_eq!(detail.voucher.status, VoucherStatus::Exported);

    // PDF 走表格式导出接口应报错（路由保护）
    let err = export::export_vouchers_table(&ctx, &[a.id], ExportFormat::Pdf);
    assert!(matches!(err, Err(CoreError::Invalid(_))));
    Ok(())
}

/// v0.10：疑似重复——单凭证查询 + 全库分组；空金额不参与判定
#[test]
fn duplicate_detection_suspects_and_groups() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let a = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-09-01".into(),
            description: "第一笔".into(),
            amount_cents: Some(9900),
            ..Default::default()
        },
    )?;
    let b = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-09-01".into(),
            description: "疑似重复".into(),
            amount_cents: Some(9900),
            ..Default::default()
        },
    )?;
    // 同金额不同日期：不构成疑似重复
    let c = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-09-02".into(),
            description: "不同日期".into(),
            amount_cents: Some(9900),
            ..Default::default()
        },
    )?;
    // 空金额：无判定意义
    let d = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-09-01".into(),
            description: "空金额".into(),
            amount_cents: None,
            ..Default::default()
        },
    )?;

    // 单凭证视角：a 的疑似重复 = {b}
    let suspects = ctx
        .db
        .with_conn(|conn| voucher_core::services::dedup::suspected_by_id(conn, a.id))?;
    assert_eq!(suspects.len(), 1);
    assert_eq!(suspects[0].id, b.id);

    // 空金额凭证：无疑似
    let empty = ctx
        .db
        .with_conn(|conn| voucher_core::services::dedup::suspected_by_id(conn, d.id))?;
    assert!(empty.is_empty());

    // 全库扫描：仅一组（a+b），c/d 不混入
    let groups = ctx
        .db
        .with_conn(voucher_core::services::dedup::scan_groups)?;
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].voucher_date, "2026-09-01");
    assert_eq!(groups[0].amount_cents, 9900);
    let ids: Vec<i64> = groups[0].vouchers.iter().map(|v| v.id).collect();
    assert!(ids.contains(&a.id) && ids.contains(&b.id));
    assert!(!ids.contains(&c.id));
    assert!(!ids.contains(&d.id));
    Ok(())
}

/// v0.10：高频摘要——按次数降序、空摘要排除
#[test]
fn frequent_descriptions_orders_by_count() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    for (desc, n) in [("办公用品采购", 3), ("差旅住宿", 2), ("市内交通", 1)] {
        for _ in 0..n {
            voucher_mgr::create_voucher(
                &ctx,
                NewVoucher {
                    description: desc.into(),
                    ..Default::default()
                },
            )?;
        }
    }
    // 空摘要（文件名默认值场景）不应参与联想
    voucher_mgr::create_voucher(&ctx, NewVoucher::default())?;

    let rows = reports::frequent_descriptions(&ctx, 8)?;
    assert_eq!(rows.len(), 3, "空摘要被排除: {rows:?}");
    assert_eq!(rows[0].description, "办公用品采购");
    assert_eq!(rows[0].count, 3);
    assert_eq!(rows[1].description, "差旅住宿");
    assert_eq!(rows[2].description, "市内交通");
    Ok(())
}

// ---------- v0.11：期间锁定 / 年度报表 / 搜索组合筛选 ----------

/// 期间锁：锁定后增删改全部拦截（改出/改入月都算），解锁恢复；OCR 口径不受限由 apply 逻辑保证
#[test]
fn period_lock_blocks_writes_until_unlock() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let v = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-15".into(),
            description: "八月凭证".into(),
            amount_cents: Some(1000),
            ..Default::default()
        },
    )?;
    let w = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-09-10".into(),
            description: "九月凭证".into(),
            ..Default::default()
        },
    )?;

    voucher_core::services::period::lock_period(&ctx, "2026-08")?;

    // 更新（改摘要）被拦
    let err = voucher_mgr::update_voucher(
        &ctx,
        v.id,
        VoucherUpdate {
            description: Some("试图修改".into()),
            ..Default::default()
        },
    );
    assert!(matches!(err, Err(CoreError::Invalid(ref e)) if e.contains("2026-08 已封账")));

    // 日期从九月改到八月（改入锁定月）被拦
    let err2 = voucher_mgr::update_voucher(
        &ctx,
        w.id,
        VoucherUpdate {
            voucher_date: Some("2026-08-20".into()),
            ..Default::default()
        },
    );
    assert!(err2.is_err(), "改入锁定月应被拦截");

    // 删除被拦（单张与批量）
    assert!(voucher_mgr::delete_vouchers(&ctx, &[v.id]).is_err());

    // 状态变更被拦（单张与批量）
    assert!(voucher_mgr::set_status(&ctx, v.id, VoucherStatus::Reviewed).is_err());
    assert!(voucher_mgr::batch_set_status(&ctx, &[v.id], VoucherStatus::Reviewed).is_err());

    // 批量编辑被拦
    assert!(voucher_mgr::batch_edit(
        &ctx,
        &[v.id],
        &voucher_mgr::BatchEditPatch {
            note: Some("批量备注".into()),
            ..Default::default()
        },
    )
    .is_err());

    // 未锁月份凭证不受影响（九月凭证可正常更新）
    voucher_mgr::update_voucher(
        &ctx,
        w.id,
        VoucherUpdate {
            description: Some("九月凭证（改）".into()),
            ..Default::default()
        },
    )?;

    // 解锁后恢复可写
    voucher_core::services::period::unlock_period(&ctx, "2026-08")?;
    voucher_mgr::update_voucher(
        &ctx,
        v.id,
        VoucherUpdate {
            description: Some("解锁后修改".into()),
            ..Default::default()
        },
    )?;
    Ok(())
}

/// 期间锁：换图被拦（附件域守卫）
#[test]
fn period_lock_blocks_replace_image() -> Result<()> {
    let (dir, ctx) = test_ctx();
    // 建卡 + 挂图（八月）
    let img1 = dir.path().join("a.png");
    let pic = image::RgbImage::from_pixel(64, 48, image::Rgb([60, 60, 200]));
    pic.save(&img1).expect("save png");
    let s = ingestion::import_images(&ctx, &[img1], None, false)?;
    assert_eq!(s.vouchers.len(), 1);
    let vid = s.vouchers[0].voucher_id;
    let today = voucher_core::utils::paths::today();

    // 锁定该凭证所在月（today 的月份）
    let period = &today[..7];
    voucher_core::services::period::lock_period(&ctx, period)?;

    let img2 = dir.path().join("b.png");
    let pic2 = image::RgbImage::from_pixel(64, 48, image::Rgb([120, 90, 30]));
    pic2.save(&img2).expect("save png");
    let err = ingestion::replace_attachment(&ctx, vid, &img2);
    assert!(matches!(err, Err(CoreError::Invalid(ref e)) if e.contains("已封账")));
    Ok(())
}

/// 年度汇总：12 个月空月补零、口径与月度汇总一致
#[test]
fn yearly_summary_zero_fills_twelve_months() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    // 2025 年 3 张（2 月 2 张 + 5 月 1 张），2026 年 1 张（当月默认日期）
    for (d, amt) in [
        ("2025-02-01", Some(1000)),
        ("2025-02-15", Some(2500)),
        ("2025-05-20", None),
    ] {
        voucher_mgr::create_voucher(
            &ctx,
            NewVoucher {
                voucher_date: d.into(),
                amount_cents: amt,
                ..Default::default()
            },
        )?;
    }
    voucher_mgr::create_voucher(&ctx, NewVoucher::default())?;

    let y2025 = reports::yearly_summary(&ctx, 2025)?;
    assert_eq!(y2025.len(), 12, "12 个月补齐");
    assert_eq!(y2025[0].month, "2025-01");
    assert_eq!(y2025[11].month, "2025-12");
    assert_eq!(y2025[1].count, 2, "2 月 2 张");
    assert_eq!(y2025[1].amount_cents, 3500);
    assert_eq!(y2025[4].count, 1, "5 月 1 张（空金额不计入金额合计）");
    assert_eq!(y2025[4].amount_cents, 0);
    assert_eq!(y2025.iter().map(|m| m.count).sum::<i64>(), 3, "全年 3 张");
    // 2026 年仅当月 1 张
    let y2026 = reports::yearly_summary(&ctx, 2026)?;
    assert_eq!(y2026.iter().map(|m| m.count).sum::<i64>(), 1);
    // 2024 年无凭证
    let y2024 = reports::yearly_summary(&ctx, 2024)?;
    assert!(y2024.iter().all(|m| m.count == 0));
    Ok(())
}

/// 搜索组合筛选：日期范围 + 金额范围与关键词 AND 组合
#[test]
fn search_filters_combine_date_and_amount() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    for (d, amt, desc) in [
        ("2026-08-01", Some(5000), "办公采购"),
        ("2026-08-20", Some(15000), "办公采购"),
        ("2026-09-05", Some(5000), "办公采购"),
        ("2026-08-10", None, "办公采购空金额"),
    ] {
        voucher_mgr::create_voucher(
            &ctx,
            NewVoucher {
                voucher_date: d.into(),
                amount_cents: amt,
                description: desc.into(),
                ..Default::default()
            },
        )?;
    }

    use voucher_core::db::repos::voucher_repo::SearchFilters;
    // 仅关键词：4 张全命中
    let all = search::search_vouchers(&ctx, "办公", &SearchFilters::default(), 50)?;
    assert_eq!(all.len(), 4);

    // 日期范围：仅 8 月（3 张）
    let by_date = search::search_vouchers(
        &ctx,
        "办公",
        &SearchFilters {
            date_from: Some("2026-08-01".into()),
            date_to: Some("2026-08-31".into()),
            ..Default::default()
        },
        50,
    )?;
    assert_eq!(by_date.len(), 3);
    assert!(by_date
        .iter()
        .all(|h| h.voucher.voucher_date.starts_with("2026-08")));

    // 金额下限：≥ 100 元（1 张：150 元；空金额与 50 元被排除）
    let by_amount = search::search_vouchers(
        &ctx,
        "办公",
        &SearchFilters {
            amount_from: Some(10000),
            ..Default::default()
        },
        50,
    )?;
    assert_eq!(by_amount.len(), 1);
    assert_eq!(by_amount[0].voucher.amount_cents, Some(15000));

    // 组合：8 月 + ≥100 元（1 张：150 元那张）
    let combined = search::search_vouchers(
        &ctx,
        "办公",
        &SearchFilters {
            date_from: Some("2026-08-01".into()),
            date_to: Some("2026-08-31".into()),
            amount_from: Some(10000),
            ..Default::default()
        },
        50,
    )?;
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0].voucher.amount_cents, Some(15000));
    Ok(())
}

/// 年度报表导出：PDF 落盘 + 批次记录（EXP-Y 前缀）+ 不改凭证状态（统计报表口径）
#[test]
#[ignore = "环境缺模型/字体时自动跳过；CI 以 --include-ignored 全量执行"]
fn export_yearly_report_pdf_and_batch() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    if voucher_core::services::pdf::find_cjk_font().is_none() {
        eprintln!("跳过：系统无中文字体");
        return Ok(());
    }
    for (d, amt) in [("2026-02-01", Some(1000)), ("2026-07-15", Some(2500))] {
        voucher_mgr::create_voucher(
            &ctx,
            NewVoucher {
                voucher_date: d.into(),
                amount_cents: amt,
                ..Default::default()
            },
        )?;
    }

    let batch = export::export_yearly_report(&ctx, 2026)?;
    assert!(
        batch.batch_no.starts_with("EXP-Y2026"),
        "年度前缀: {}",
        batch.batch_no
    );
    assert!(PathBuf::from(&batch.pdf_path).exists());
    assert!(std::fs::metadata(&batch.pdf_path).unwrap().len() > 1000);
    assert!(batch.voucher_count >= 2);

    // 批次明细可反查（该年全部凭证）
    let detail = export::batch_vouchers(&ctx, batch.id)?;
    assert_eq!(detail.len(), 2);

    // 状态不流转（区别于 PDF 归档导出）
    for v in detail {
        assert_eq!(v.status, VoucherStatus::Draft, "年度报表不改状态");
    }

    // 空年度报错
    assert!(export::export_yearly_report(&ctx, 2020).is_err());
    Ok(())
}

// ---------- v0.12：手动裁剪 / 旋转 ----------

/// 手动裁剪：90° 旋转 + 裁剪覆盖写增强版（原图字节不变）、尺寸正确、期间锁拦截
#[test]
fn crop_attachment_rotates_and_overwrites_enhanced() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let img = dir.path().join("crop-src.png");
    make_png_variant(&img, 200);
    let s = ingestion::import_images(&ctx, &[img], None, false)?;
    assert_eq!(s.vouchers.len(), 1);
    let att = s.vouchers[0].attachment.clone();
    let orig_abs = ctx.paths.attachment_abs(&att.file_path);
    let orig_bytes = std::fs::read(&orig_abs)?;

    // 90° 旋转：64×48 → 48×64；裁剪 (8,16)-(32,24) → 24×8
    let updated = voucher_core::services::crop::crop_attachment(
        &ctx,
        att.id,
        voucher_core::services::crop::CropRect {
            x: 8,
            y: 16,
            w: 24,
            h: 8,
        },
        90,
    )?;

    // 增强版路径已设置且文件存在；尺寸 = 裁剪框
    let enhanced = updated.enhanced_path.expect("enhanced_path 已设置");
    let enhanced_abs = ctx.paths.attachment_abs(&enhanced);
    assert!(enhanced_abs.exists(), "增强版落盘");
    assert!(enhanced.ends_with(".enhanced.jpg"));
    let out = image::open(&enhanced_abs).expect("可解码");
    assert_eq!((out.width(), out.height()), (24, 8), "旋转后按框裁剪的尺寸");

    // 原图字节不变（裁剪只写工作版本）
    assert_eq!(std::fs::read(&orig_abs)?, orig_bytes, "原图不可变");

    // 微调角度（非 90 倍数）：白底重采样路径不报错，尺寸 = 裁剪框
    let updated2 = voucher_core::services::crop::crop_attachment(
        &ctx,
        att.id,
        voucher_core::services::crop::CropRect {
            x: 0,
            y: 0,
            w: 40,
            h: 40,
        },
        10,
    )?;
    let out2 = image::open(
        ctx.paths
            .attachment_abs(updated2.enhanced_path.as_deref().unwrap_or(&enhanced)),
    )
    .expect("可解码");
    assert_eq!((out2.width(), out2.height()), (40, 40));

    // 空裁剪框报错
    let err = voucher_core::services::crop::crop_attachment(
        &ctx,
        att.id,
        voucher_core::services::crop::CropRect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        },
        0,
    );
    assert!(matches!(err, Err(voucher_core::CoreError::Invalid(_))));

    // 期间锁拦截（与换图同口径）
    let today = voucher_core::utils::paths::today();
    voucher_core::services::period::lock_period(&ctx, &today[..7])?;
    let err2 = voucher_core::services::crop::crop_attachment(
        &ctx,
        att.id,
        voucher_core::services::crop::CropRect {
            x: 0,
            y: 0,
            w: 24,
            h: 24,
        },
        0,
    );
    assert!(
        matches!(err2, Err(voucher_core::CoreError::Invalid(ref e)) if e.contains("已封账")),
        "封账月不可调整图片"
    );
    Ok(())
}

// ---------- v0.13：账单 CSV 对账 ----------

/// 对账匹配三类结果：✅ 匹配 / 🔵 账单无凭证 / 🔴 凭证无账单；报告 CSV 落盘
#[test]
fn reconcile_matches_and_reports_gaps() -> Result<()> {
    let (dir, ctx) = test_ctx();
    // 凭证：一张能对上（8-20 · 123.45），一张对不上（8-21 同日不同金额 · 99.00）
    voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-20".into(),
            description: "办公耗材".into(),
            amount_cents: Some(12345),
            ..Default::default()
        },
    )?;
    voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: "2026-08-21".into(),
            description: "现金采购".into(),
            amount_cents: Some(9900),
            ..Default::default()
        },
    )?;

    let csv = dir.path().join("bill.csv");
    std::fs::write(
        &csv,
        "微信支付账单明细\n交易时间,交易类型,交易对方,商品,收/支,金额(元),支付方式\n\
         2026-08-20 12:30:45,商户消费,文一店,办公耗材,支出,¥123.45,零钱\n\
         2026-08-21 18:00:00,商户消费,美团,外卖,支出,¥35.50,零钱\n",
    )?;

    let report = voucher_core::services::reconcile::reconcile(&ctx, &csv)?;
    assert_eq!(report.bill_count, 2);
    assert_eq!(report.matched.len(), 1, "一条账单匹配到凭证");
    assert_eq!(report.matched[0].bill.amount_cents, 12345);
    assert_eq!(report.matched[0].voucher.amount_cents, Some(12345));
    assert_eq!(report.bill_only.len(), 1, "一条账单无对应凭证（漏拍图）");
    assert_eq!(report.bill_only[0].subject, "外卖");
    assert_eq!(report.voucher_only.len(), 1, "一张凭证无对应账单");
    assert_eq!(report.voucher_only[0].amount_cents, Some(9900));
    assert_eq!(report.date_from, "2026-08-20");
    assert_eq!(report.date_to, "2026-08-21");
    // 报告 CSV 已落盘且可读
    let path = report.report_path.expect("报告落盘");
    assert!(PathBuf::from(&path).exists());
    let text = String::from_utf8_lossy(&std::fs::read(&path)?).to_string();
    assert!(text.contains("✅ 匹配"));
    assert!(text.contains("🔵 账单无凭证"));
    assert!(text.contains("🔴 凭证无账单"));
    Ok(())
}

/// 残余清理（B11）：导出失败**不得**留下最终文件或批次记录。
///
/// 失败注入：把 exports 目录设成只读（0555）。导出应在写临时文件阶段就失败——目录里
/// 既无最终 PDF/CSV、也无 `.tmp` 残留，且不产生批次记录（此前直接写最终名时，半截文件
/// 会留在导出目录里、而界面看不到批次）。
///
/// 仅 unix：Windows 的目录只读位不阻止创建文件。用例会先**探测**只读是否真的生效
/// （root 下权限位无效）——探测失败即软跳过，避免假绿。
#[cfg(unix)]
#[test]
fn export_failure_leaves_no_file_or_batch() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, ctx) = test_ctx();
    let v = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "导出失败注入".into(),
            ..Default::default()
        },
    )?;
    let exports = ctx.paths.exports.clone();
    let orig = std::fs::metadata(&exports)?.permissions();

    std::fs::set_permissions(&exports, std::fs::Permissions::from_mode(0o555))?;
    let probe = exports.join(".probe");
    if std::fs::write(&probe, b"x").is_ok() {
        let _ = std::fs::remove_file(&probe);
        std::fs::set_permissions(&exports, orig)?;
        eprintln!("跳过：当前用户可无视目录只读位（root？），权限注入无效");
        return Ok(());
    }

    let result = export::export_vouchers_table(&ctx, &[v.id], ExportFormat::Csv);
    std::fs::set_permissions(&exports, orig)?; // 先恢复，避免影响后续断言

    assert!(result.is_err(), "只读目录下导出必须报错，而不是静默成功");
    assert!(
        export::list_batches(&ctx, 10)?.is_empty(),
        "失败不得留下批次记录"
    );
    let left: Vec<String> = std::fs::read_dir(&exports)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        left.is_empty(),
        "失败不得留下最终文件或 .tmp 残留：{left:?}"
    );
    Ok(())
}
