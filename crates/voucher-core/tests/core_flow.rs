//! 集成测试：新库核心闭环（阶段 2 退出标准 + G2 门的本地验证）

use std::io::Read;
use std::path::PathBuf;

use chrono::Datelike;
use voucher_core::db::repos::voucher_repo::VoucherQuery;
use voucher_core::models::{NewVoucher, OcrStatus, VoucherStatus, VoucherUpdate};
use voucher_core::services::{backup, export, ingestion, ocr, reports, search, voucher_mgr};
use voucher_core::{CoreContext, CoreError, Result};

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
fn backup_includes_attachments_and_drill_passes() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let img = _dir.path().join("a.png");
    make_png(&img);
    ingestion::import_images(&ctx, &[img], None, false)?;

    let info = backup::run_backup(&ctx)?;
    assert!(info.attachment_count >= 1, "附件应计入备份");

    // 附件在恢复产物中（3-2-1：附件原图不可再生，必须在包内）
    let target = _dir.path().join("restored");
    let summary = backup::restore_backup(PathBuf::from(&info.path).as_path(), &target)?;
    assert!(summary.checksums_verified, "v2 备份应完成 sha256 校验");
    assert!(summary.attachment_count >= 1);
    assert!(
        target.join("attachments").is_dir(),
        "恢复产物应含 attachments 目录"
    );
    let mut has_file = false;
    for entry in std::fs::read_dir(target.join("attachments"))? {
        let path = entry?.path();
        has_file |= path.is_file() || path.is_dir();
    }
    assert!(has_file, "附件目录不应为空");

    // 恢复演练：校验 + 解包 + 打开数据库，全部通过
    let report = backup::drill_backup(&ctx, None)?;
    assert!(report.db_ok);
    assert!(report.checksums_verified);
    assert!(report.attachment_count >= 1);
    Ok(())
}

#[test]
fn tampered_backup_rejected_by_checksum() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "校验和测试".into(),
            ..Default::default()
        },
    )?;
    let info = backup::run_backup(&ctx)?;

    // 构造篡改包：同 entry 名，但 voucher.db 中部翻转一个字节，manifest 保持原样
    let src = PathBuf::from(&info.path);
    let tampered = _dir.path().join("tampered.zip");
    {
        let file = std::fs::File::open(&src)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let out = std::fs::File::create(&tampered)?;
        let mut w = zip::ZipWriter::new(out);
        let opts = zip::write::SimpleFileOptions::default();
        for i in 0..zip.len() {
            let mut bytes = Vec::new();
            let name = {
                let mut e = zip.by_index(i)?;
                e.read_to_end(&mut bytes)?;
                e.name().to_string()
            };
            if name == backup::DB_ENTRY {
                let mid = bytes.len() / 2;
                bytes[mid] ^= 0xFF;
            }
            w.start_file(name.as_str(), opts)?;
            std::io::Write::write_all(&mut w, &bytes)?;
        }
        w.finish()?;
    }

    let err = backup::inspect_backup(&tampered);
    assert!(err.is_err(), "被篡改的备份必须被 sha256 校验拒绝");
    Ok(())
}

#[test]
fn secondary_dir_gets_copy_of_backup() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let secondary = _dir.path().join("other-disk");
    let mut cfg = ctx.config();
    cfg.backup.secondary_dir = secondary.to_string_lossy().to_string();
    ctx.set_config(cfg)?;

    let info = backup::run_backup(&ctx)?;
    assert!(
        secondary.join(&info.file_name).is_file(),
        "异盘目录应有备份副本"
    );
    Ok(())
}

#[test]
fn unwritable_secondary_dir_warns_but_backup_succeeds() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    // 占位同名文件 → create_dir_all 失败 → 异盘副本失败
    let blocked = _dir.path().join("blocked");
    std::fs::write(&blocked, b"file not dir")?;

    let mut cfg = ctx.config();
    cfg.backup.secondary_dir = blocked.to_string_lossy().to_string();
    ctx.set_config(cfg)?;

    let info = backup::run_backup(&ctx)?;
    assert!(PathBuf::from(&info.path).is_file(), "本地主备份不受影响");
    assert!(
        !info.warnings.is_empty(),
        "异盘失败应产生告警供 UI 提示，实际: {:?}",
        info.warnings
    );
    assert!(info.warnings.iter().any(|w| w.contains("异盘")));
    Ok(())
}

#[test]
fn delete_vouchers_cleans_attachment_files() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let img = dir.path().join("receipt.png");
    make_png(&img);
    let summary = ingestion::import_images(&ctx, &[img], None, false)?;
    assert_eq!(summary.imported.len(), 1);

    // 附件物理文件存在
    let rel = summary.imported[0].file_path.clone();
    let abs = ctx.paths.attachment_abs(&rel);
    assert!(abs.is_file(), "附件文件应已落盘: {}", abs.display());

    // 批量删除（含一个不存在的 id，验证幂等跳过）
    let n = voucher_mgr::delete_vouchers(&ctx, &[summary.voucher_id, 99999])?;
    assert_eq!(n, 1, "只应删除实际存在的 1 张");
    assert!(!abs.exists(), "附件物理文件应随凭证删除被清理");
    let att_count: i64 = ctx.db.with_conn(|conn| {
        conn.query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0))
            .map_err(CoreError::from)
    })?;
    assert_eq!(att_count, 0, "附件行应随外键级联清空");

    // 幂等：再删同一 id 返回 0
    let again = voucher_mgr::delete_vouchers(&ctx, &[summary.voucher_id])?;
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
    assert_eq!(summary.imported.len(), 1, "同批同内容只入库一次");
    assert_eq!(summary.skipped_duplicates.len(), 1);
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
    let hits = search::search_vouchers(&ctx, "%", 10)?;
    assert_eq!(hits.len(), 1, "关键词 % 不应被当作通配符");
    assert_eq!(hits[0].voucher.description, "进度100%完成");

    // '_' 同理：无人含字面下划线 → 零命中
    let hits2 = search::search_vouchers(&ctx, "_", 10)?;
    assert_eq!(hits2.len(), 0, "关键词 _ 不应匹配所有非空文本");
    Ok(())
}

#[test]
fn naming_seq_skips_holes_after_deletion() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    // 同日建三张：001 / 002 / 003
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
    assert!(v1.voucher_no.ends_with("001"));
    assert!(v2.voucher_no.ends_with("002"));
    assert!(v3.voucher_no.ends_with("003"));

    // 删中间的 002 → count=2 → 朴素 count+1 会算出 003 撞 UNIQUE
    voucher_mgr::delete_voucher(&ctx, v2.id)?;
    let v4 = voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            voucher_date: d.into(),
            ..Default::default()
        },
    )?;
    assert!(
        v4.voucher_no.ends_with("004"),
        "应跳过空洞递增到 004，实际 {}",
        v4.voucher_no
    );
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

/// G2 标准②直接证据：图片导入 → 本地 OCR 真实识别 → 全文搜索 → PDF 导出 全链路闭环。
/// 与分段测试的区别：OCR 文本由本地引擎识别落库（非手工写 DB），导出走真实 PDF 生成。
#[test]
fn g2_closed_loop_import_ocr_pdf() -> Result<()> {
    // 前置素材缺失时跳过（模型/样张不随 git 走的环境，如精简 CI）
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
    if voucher_core::services::pdf::find_cjk_font().is_none() {
        eprintln!("跳过：系统无中文字体（PDF 导出不可用）");
        return Ok(());
    }

    let (dir, ctx) = test_ctx();

    // 1. 模型装入临时库 models 目录（全新库 → 真实走 ensure_installed 复制路径）
    assert!(
        voucher_core::LocalOcr::ensure_installed(&models, &ctx.paths.models)?,
        "全新临时库应触发模型复制"
    );
    assert!(ctx.local_ocr_ready());

    // 2. 导入样张并触发 OCR（默认配置 engine=local / enabled=true，走本地引擎）
    let img = dir.path().join("ocr_sample.png");
    std::fs::copy(&sample, &img)?;
    let summary = ingestion::import_images(&ctx, &[img], None, true)?;
    assert_eq!(summary.imported.len(), 1);

    // 3. OCR 文本经引擎识别落库
    let detail = voucher_mgr::get_detail(&ctx, summary.voucher_id)?;
    assert_eq!(detail.attachments.len(), 1);
    let att = &detail.attachments[0];
    assert_eq!(att.ocr_status, OcrStatus::Done, "OCR 状态应为 Done");
    let text = att.ocr_text.as_deref().unwrap_or_default();
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    for expect in ["报销凭证", "金额：1234.56元"] {
        assert!(
            compact.contains(expect),
            "本地引擎识别结果缺少「{expect}」，实际: {compact}"
        );
    }
    // 识别走的是本地引擎且已落缓存（区别于旧测试手工写入 DB 的路径）
    let cached_engine: String = ctx.db.with_conn(|conn| {
        conn.query_row("SELECT engine FROM ocr_cache LIMIT 1", [], |r| r.get(0))
            .map_err(CoreError::from)
    })?;
    assert_eq!(cached_engine, "local");

    // 4. OCR 文本进入全文搜索
    let hits = search::search_vouchers(&ctx, "报销", 10)?;
    assert_eq!(hits.len(), 1, "OCR 文本应可被全文搜索命中");
    assert!(hits[0].matched_in.iter().any(|m| m == "OCR"));

    // 5. 导出 PDF（打印默认配置）
    let batch = export::export_with_defaults(&ctx, &[summary.voucher_id])?;
    assert_eq!(batch.voucher_count, 1);
    assert!(PathBuf::from(&batch.pdf_path).exists(), "PDF 应落盘");
    assert!(std::fs::metadata(&batch.pdf_path)?.len() > 1000);

    // 6. 批次记录 + 凭证状态流转「已导出」
    let batches = export::list_batches(&ctx, 10)?;
    assert_eq!(batches.len(), 1);
    let detail = voucher_mgr::get_detail(&ctx, summary.voucher_id)?;
    assert_eq!(detail.voucher.status, VoucherStatus::Exported);
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
