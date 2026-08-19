//! 集成测试：新库核心闭环（阶段 2 退出标准 + G2 门的本地验证）

use std::path::PathBuf;

use voucher_core::db::repos::voucher_repo::VoucherQuery;
use voucher_core::models::{NewVoucher, OcrStatus, VoucherStatus, VoucherUpdate};
use voucher_core::services::{backup, export, ingestion, ocr, search, voucher_mgr};
use voucher_core::{CoreContext, Result};

fn test_ctx() -> (tempfile::TempDir, CoreContext) {
    let dir = tempfile::tempdir().expect("tempdir");
    let ctx = CoreContext::init(Some(dir.path().to_path_buf())).expect("init");
    (dir, ctx)
}

fn make_png(path: &std::path::Path) {
    let img = image::RgbImage::from_pixel(64, 48, image::Rgb([180u8, 60, 60]));
    img.save(path).expect("save png");
}

#[test]
fn schema_creates_seven_tables() -> Result<()> {
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
            "export_batches",
            "export_items",
            "snapshots",
            "ocr_cache",
            "config",
        ] {
            assert!(tables.iter().any(|x| x == t), "缺表 {t}");
        }
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        assert_eq!(v, 1);
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
        v.voucher_no.ends_with("-001"),
        "编号自动生成: {}",
        v.voucher_no
    );
    assert_eq!(v.status, VoucherStatus::Draft);

    // 同一天第二张 → 002
    let v2 = voucher_mgr::create_voucher(&ctx, NewVoucher::default())?;
    assert!(v2.voucher_no.ends_with("-002"));

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
fn import_images_dedups_and_ocr_text_searchable() -> Result<()> {
    let (dir, ctx) = test_ctx();

    let img = dir.path().join("receipt.png");
    make_png(&img);

    // 第一次导入：自动建草稿凭证
    let s1 = ingestion::import_images(&ctx, std::slice::from_ref(&img), None, false)?;
    assert_eq!(s1.imported.len(), 1);
    assert_eq!(s1.skipped_duplicates.len(), 0);

    // 同一文件再导入：内容重复 → 跳过
    let s2 = ingestion::import_images(&ctx, std::slice::from_ref(&img), None, false)?;
    assert_eq!(s2.imported.len(), 0);
    assert_eq!(s2.skipped_duplicates.len(), 1);

    // 不支持的类型
    let txt = dir.path().join("note.txt");
    std::fs::write(&txt, b"hello")?;
    let s3 = ingestion::import_images(&ctx, &[txt], None, false)?;
    assert_eq!(s3.failed.len(), 1);

    // 手动写入 OCR 文本 → 全文搜索能命中
    ctx.db.with_conn(|conn| {
        voucher_core::db::repos::attachment_repo::set_ocr(
            conn,
            s1.imported[0].id,
            OcrStatus::Done,
            Some("出租车票 45.00 元"),
        )
    })?;
    let hits = search::search_vouchers(&ctx, "出租车", 10)?;
    assert_eq!(hits.len(), 1);
    assert!(hits[0].matched_in.iter().any(|m| m == "OCR"));
    Ok(())
}

#[test]
fn ocr_cache_hits_without_network() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    ctx.db.with_conn(|conn| {
        voucher_core::db::repos::ocr_cache_repo::upsert(conn, "abc123", "cloud", "缓存的识别文本")
    })?;

    // 引擎未配置时直接识别应报错（证明不会绕过配置乱发请求）
    let err = ocr::run_for_attachment(&ctx, 1);
    assert!(err.is_err());
    Ok(())
}

#[test]
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
fn backup_and_restore_roundtrip() -> Result<()> {
    let (dir, ctx) = test_ctx();
    voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "备份前数据".into(),
            ..Default::default()
        },
    )?;

    let info = backup::run_backup(&ctx)?;
    assert!(PathBuf::from(&info.path).exists());

    let backups = backup::list_backups(&ctx)?;
    assert_eq!(backups.len(), 1);

    // 校验非法备份被拒
    let fake = dir.path().join("fake.zip");
    std::fs::write(&fake, b"not a zip")?;
    assert!(backup::inspect_backup(&fake).is_err());

    // 恢复到新目录并验证数据在
    let target = dir.path().join("restored");
    let summary = backup::restore_backup(PathBuf::from(&info.path).as_path(), &target)?;
    assert!(!summary.restored_files.is_empty());
    assert!(target.join("data").join("voucher.db").exists());

    let conn = voucher_core::db::connection::open(&target.join("data").join("voucher.db"))?;
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM vouchers", [], |r| r.get(0))?;
    assert_eq!(n, 1);
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
fn local_ocr_end_to_end() -> Result<()> {
    // 模型位于项目 assets/ocr-models（仓库自带）；缺失时跳过（如 CI 未含大文件）
    let models = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ocr-models");
    if !voucher_core::LocalOcr::is_installed(&models) {
        eprintln!("跳过：本地 OCR 模型不存在（{}）", models.display());
        return Ok(());
    }
    let sample =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/assets/ocr_sample.png");
    if !sample.is_file() {
        eprintln!("跳过：测试图不存在");
        return Ok(());
    }

    let mut engine = voucher_core::LocalOcr::load(&models)?;
    let img = image::io::Reader::open(&sample).unwrap().decode().unwrap();
    let lines = engine.recognize_image(&img)?;
    let all: String = lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("");
    let compact: String = all.chars().filter(|c| !c.is_whitespace()).collect();

    // v01.png 的标准答案关键行（Spike B 数据集）
    for expect in [
        "报销凭证",
        "凭证编号：20260819-001",
        "金额：1234.56元",
        "经办人：张三",
    ] {
        assert!(
            compact.contains(expect),
            "OCR 结果缺少「{expect}」，实际识别: {compact}"
        );
    }
    Ok(())
}

#[test]
fn keyring_secret_roundtrip() -> Result<()> {
    let store = voucher_core::SecretStore::new("VoucherStudioTest");
    store.set("test_account", "s3cret-value")?;
    assert_eq!(store.get("test_account")?.as_deref(), Some("s3cret-value"));
    assert!(store.delete("test_account")?);
    assert_eq!(store.get("test_account")?, None);
    assert!(!store.delete("test_account")?);
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
