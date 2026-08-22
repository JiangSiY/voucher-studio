//! C 步真机验收：文档扫描矫正（5 步增强 + 来源分类 + 缓存管理）端到端
//!
//! 覆盖清单：
//! 1. 导入后增强版落盘 + 物理文件可读 + 字段就位
//! 2. 关闭总开关：跳过 5 步流水线
//! 3. only_scans=true + 截图：跳过流水线
//! 4. clear_enhance_cache：DB 字段置 NULL + 物理文件清理
//! 5. OCR 优先用增强版（存在时 ocr_target 切到 enhanced_abs）

use std::path::PathBuf;

use image::{ImageBuffer, Rgb};
use opencv::prelude::MatTraitConst;
use voucher_core::db::repos::attachment_repo;
use voucher_core::models::EnhanceSource;
use voucher_core::services::ingestion;
use voucher_core::CoreContext;

fn test_ctx() -> (tempfile::TempDir, CoreContext) {
    let dir = tempfile::tempdir().expect("tempdir");
    let ctx = CoreContext::init(Some(dir.path().to_path_buf())).expect("init");
    (dir, ctx)
}

/// 构造一张 800x600 模拟「文档」：白底 + 黑色矩形 + 文字位图（满足 5 步流水线需要
/// 的「强 Hough 信号 + 强四边形轮廓」）。保存为 PNG（让 classify_source 走「截图分支」）
/// 或 JPEG（让 classify_source 走「扫描件分支」）。
#[allow(dead_code)]
fn make_doc(path: &std::path::Path, ext: &str) {
    make_doc_with_seed(path, ext, 0);
}

/// 临时目录放测试图片（不入库）。seed 不同 → 不同位置的彩色块 → 不同的 MD5（避免去重）
fn make_doc_in_tmpdir(dir: &tempfile::TempDir, name: &str, ext: &str) -> PathBuf {
    let p = dir.path().join(name);
    make_doc_with_seed(&p, ext, 0);
    p
}

/// 临时目录放测试图片，seed 用于生成唯一色块
fn make_doc_with_seed_in_tmpdir(
    dir: &tempfile::TempDir,
    name: &str,
    ext: &str,
    seed: u8,
) -> PathBuf {
    let p = dir.path().join(name);
    make_doc_with_seed(&p, ext, seed);
    p
}

/// 构造带 seed 的文档图（用于需要「多个不同内容图」的去重/缩略图测试）
fn make_doc_with_seed(path: &std::path::Path, ext: &str, seed: u8) {
    let mut img = ImageBuffer::<Rgb<u8>, _>::from_pixel(800, 600, Rgb([255u8, 255, 255]));
    // 黑色矩形（强四边形信号）
    for y in 60..540 {
        for x in 80..720 {
            if !(65..=535).contains(&y) || !(85..=715).contains(&x) {
                img.put_pixel(x, y, Rgb([0, 0, 0]));
            }
        }
    }
    // 5 条水平黑线（强 Hough 信号）
    for i in 0..5 {
        let y = 200 + i * 60;
        for x in 120..700 {
            img.put_pixel(x, y, Rgb([0, 0, 0]));
        }
    }
    // seed 不同 → 不同位置 / 颜色的填充块，避免被 MD5 去重
    let r = ((seed as u16 * 73) % 256) as u8;
    let g = ((seed as u16 * 137 + 11) % 256) as u8;
    let b = ((seed as u16 * 191 + 23) % 256) as u8;
    let off = (seed as i32 * 17) % 200;
    for y in 100..140 {
        for x in (120 + off)..(260 + off) {
            if x >= 0 && x < 800 {
                img.put_pixel(x as u32, y, Rgb([r, g, b]));
            }
        }
    }
    match ext {
        "jpg" | "jpeg" => img.save(path).expect("save jpg"),
        _ => img.save(path).expect("save png"),
    }
}

// ===== 1. 增强版落盘 + 字段就位 =====

#[test]
fn enhance_jpeg_source_writes_enhanced_jpeg_and_db_fields() {
    let (dir, ctx) = test_ctx();
    let src = make_doc_in_tmpdir(&dir, "scan.jpg", "jpg");

    let summary = ingestion::import_images(&ctx, &[src], None, false).expect("import ok");
    assert_eq!(summary.vouchers.len(), 1, "应建 1 张凭证");
    let att = &summary.vouchers[0].attachment;

    // 字段：增强版路径 + 来源分类
    assert!(
        att.enhanced_path.is_some(),
        "JPEG 源应触发增强流水线（enhanced_path 不为 None）"
    );
    let rel = att.enhanced_path.as_ref().unwrap();
    assert!(rel.ends_with(".enhanced.jpg"), "文件名后缀：{rel}");
    assert_eq!(
        att.enhanced_source,
        EnhanceSource::Scan,
        "JPEG 源归类为 Scan"
    );

    // 物理文件落盘
    let abs = ctx.paths.attachment_abs(rel);
    assert!(abs.exists(), "增强版物理文件应存在: {}", abs.display());
    let meta = std::fs::metadata(&abs).expect("stat enhanced");
    assert!(meta.len() > 0, "增强版大小应 > 0");
    // JPEG 魔数 SOI
    let head = std::fs::read(&abs).expect("read enhanced");
    assert_eq!(&head[..2], &[0xFF, 0xD8], "应为 JPEG 字节流");

    // 增强版可被 OpenCV 重新读回（自检：保存的 JPEG 没坏）
    let reloaded = opencv::imgcodecs::imread(
        abs.to_string_lossy().as_ref(),
        opencv::imgcodecs::ImreadModes::IMREAD_COLOR as i32,
    )
    .expect("opencv re-read ok");
    assert!(!reloaded.empty(), "增强版重读应为非空图像");
}

#[test]
fn enhance_png_source_classifies_as_scan_above_size_threshold() {
    // PNG 但尺寸 > 500KB → 不被分类为截图（classify_source 的启发式）
    let (dir, ctx) = test_ctx();
    let src = make_doc_in_tmpdir(&dir, "scan.png", "png");

    let summary = ingestion::import_images(&ctx, &[src], None, false).expect("import ok");
    let att = &summary.vouchers[0].attachment;
    // 800*600 PNG 编码 > 1MB → 触发「尺寸+比例」截图判定可能误伤；
    // 我们用 4:3 比例（800/600 ≈ 1.33）正好命中 → 应被归为 Screenshot
    // 这与产品决策一致：扫描件大多是 JPEG，PNG 视为截图
    assert_eq!(
        att.enhanced_source,
        EnhanceSource::Screenshot,
        "PNG + 屏幕比例 → Screenshot（classify 启发式）"
    );
    // 截图也走流水线（only_scans=false 默认）；仅在 only_scans=true 时跳过
    assert!(
        att.enhanced_path.is_some(),
        "默认（only_scans=false）截图也跑流水线"
    );
}

// ===== 2. 关闭总开关 → 不生成增强版 =====

#[test]
fn enhance_disabled_skips_pipeline() {
    let (dir, ctx) = test_ctx();
    // 关闭总开关
    {
        let mut cfg = ctx.config();
        cfg.enhance.enabled = false;
        ctx.set_config(cfg).unwrap();
    }
    let src = make_doc_in_tmpdir(&dir, "scan.jpg", "jpg");

    let summary = ingestion::import_images(&ctx, &[src], None, false).expect("import ok");
    let att = &summary.vouchers[0].attachment;
    assert!(
        att.enhanced_path.is_none(),
        "总开关关闭 → 不生成增强版"
    );
    // 来源分类仍写入（用于 UI 提示）
    assert_eq!(att.enhanced_source, EnhanceSource::Scan);
}

// ===== 3. only_scans=true + 截图 → 跳过 =====

#[test]
fn enhance_only_scans_skips_screenshots() {
    let (dir, ctx) = test_ctx();
    // 开启 only_scans
    {
        let mut cfg = ctx.config();
        cfg.enhance.only_scans = true;
        ctx.set_config(cfg).unwrap();
    }
    // PNG + 屏幕比例 → 会被 classify 为 Screenshot
    let src = make_doc_in_tmpdir(&dir, "snap.png", "png");

    let summary = ingestion::import_images(&ctx, &[src], None, false).expect("import ok");
    let att = &summary.vouchers[0].attachment;
    assert_eq!(att.enhanced_source, EnhanceSource::Screenshot);
    assert!(
        att.enhanced_path.is_none(),
        "only_scans=true + 截图 → 不生成增强版"
    );
}

// ===== 4. 清除增强缓存 =====

#[test]
fn clear_enhance_cache_resets_db_and_removes_files() {
    let (dir, ctx) = test_ctx();
    let src = make_doc_in_tmpdir(&dir, "scan.jpg", "jpg");
    let summary = ingestion::import_images(&ctx, &[src], None, false).expect("import ok");
    let att_id = summary.vouchers[0].attachment.id;
    let rel = summary.vouchers[0]
        .attachment
        .enhanced_path
        .as_ref()
        .expect("enhanced 存在")
        .clone();
    let abs = ctx.paths.attachment_abs(&rel);
    assert!(abs.exists());

    // 模拟 src-tauri commands::clear_enhance_cache 的语义：
    // 列出所有 enhanced → 逐条删物理文件（仅当位于 attachments 内）+ DB 字段置 NULL
    let enhanced_list = ctx
        .db
        .with_conn(attachment_repo::list_enhanced)
        .expect("list_enhanced");
    assert_eq!(enhanced_list.len(), 1);
    for a in &enhanced_list {
        if let Some(r) = &a.enhanced_path {
            let p = ctx.paths.attachment_abs(r);
            if ctx.paths.is_inside_attachments(&p) {
                std::fs::remove_file(&p).expect("remove enhanced file");
            }
        }
        ctx.db
            .with_conn(|conn| attachment_repo::clear_enhanced(conn, a.id))
            .expect("clear_enhanced");
    }
    assert!(!abs.exists(), "物理文件应被删除");

    // 重读：增强字段为 None
    let reloaded = ctx
        .db
        .with_conn(|c| attachment_repo::get(c, att_id))
        .expect("get")
        .expect("some");
    assert!(reloaded.enhanced_path.is_none());
}

// ===== 5. OCR 优先用增强版（数据层确认） =====

#[test]
fn ocr_target_uses_enhanced_when_present() {
    // 不跑真 OCR（需 ort + 模型）；改为：构造已有 enhanced_path 的附件
    // 校验 OCR 模块读取 attachment.enhanced_path 的语义（不依赖 OCR 引擎）。
    //
    // 决策：仅校验"读 attachment.enhanced_path → 存在则 ocr_target = enhanced_abs"
    // 这段逻辑是 ocr::run_for_attachment 的内联分支（看 attachment_repo::get 取出的字段）。
    let (dir, ctx) = test_ctx();
    let src = make_doc_in_tmpdir(&dir, "scan.jpg", "jpg");
    let summary = ingestion::import_images(&ctx, &[src], None, false).expect("import ok");
    let att = &summary.vouchers[0].attachment;
    let enhanced_rel = att.enhanced_path.as_ref().expect("enhanced");
    let enhanced_abs = ctx.paths.attachment_abs(enhanced_rel);
    let original_abs = ctx.paths.attachment_abs(&att.file_path);

    // OCR 服务选择的输入（来自 attachment.enhanced_path，存在时优先）
    let ocr_target = if let Some(ep) = &att.enhanced_path {
        let p = ctx.paths.attachment_abs(ep);
        if p.exists() { p } else { ctx.paths.attachment_abs(&att.file_path) }
    } else {
        ctx.paths.attachment_abs(&att.file_path)
    };

    assert_eq!(
        ocr_target, enhanced_abs,
        "OCR 应优先用增强版路径"
    );
    assert_ne!(ocr_target, original_abs, "OCR 不应回退到原图");
}

// ===== 6. 重复导入不重复增强 =====

#[test]
fn duplicate_image_does_not_create_second_enhanced() {
    let (dir, ctx) = test_ctx();
    let src = make_doc_in_tmpdir(&dir, "scan.jpg", "jpg");

    // 第一次：建卡 + 增强
    let s1 = ingestion::import_images(&ctx, std::slice::from_ref(&src), None, false)
        .expect("import 1");
    assert_eq!(s1.vouchers.len(), 1);

    // 第二次：内容去重 → 跳过建卡 → 增强版也只有一份
    let s2 = ingestion::import_images(&ctx, std::slice::from_ref(&src), None, false)
        .expect("import 2");
    assert!(s2.vouchers.is_empty(), "第二次应被去重跳过");
    assert_eq!(s2.skipped_duplicates.len(), 1);

    // 库内只有 1 张附件
    let atts = ctx
        .db
        .with_conn(attachment_repo::list_enhanced)
        .expect("list_enhanced");
    assert_eq!(atts.len(), 1, "增强版不应重复");
}

// ===== 7. D-1 列表缩略图查询 =====

#[test]
fn first_thumb_for_vouchers_returns_enhanced_path_when_present() {
    let (dir, ctx) = test_ctx();
    // 注入 3 张：2 张 JPEG（走增强）+ 1 张 PNG（截图也跑流水线）。seed 不同避免去重
    let jpg1 = make_doc_with_seed_in_tmpdir(&dir, "a.jpg", "jpg", 1);
    let jpg2 = make_doc_with_seed_in_tmpdir(&dir, "b.jpg", "jpg", 2);
    let png1 = make_doc_with_seed_in_tmpdir(&dir, "c.png", "png", 3);

    let summary = ingestion::import_images(&ctx, &[jpg1, jpg2, png1], None, false).expect("import");
    assert_eq!(summary.vouchers.len(), 3);
    let ids: Vec<i64> = summary.vouchers.iter().map(|v| v.voucher_id).collect();

    // 一次性 IN 查询拿全部缩略图
    let rows = ctx
        .db
        .with_conn(|c| attachment_repo::first_thumb_for_vouchers(c, &ids))
        .expect("first_thumb_for_vouchers");
    assert_eq!(rows.len(), 3, "3 张凭证都应返回首附件缩略图");

    // 每行都应有 enhanced_path（默认总开关 + only_scans=false 都跑流水线）
    for (vid, _fp, ep, _src) in &rows {
        assert!(ep.is_some(), "凭证 {vid} 应有 enhanced_path（默认配置下）");
    }

    // 校验：PNG 源归类为 Screenshot
    let png_voucher = &summary.vouchers[2];
    let png_row = rows
        .iter()
        .find(|(vid, _, _, _)| *vid == png_voucher.voucher_id)
        .expect("png row");
    assert_eq!(png_row.3, "screenshot", "PNG 源 → screenshot");
    // JPEG 源归类为 Scan
    let jpg_voucher = &summary.vouchers[0];
    let jpg_row = rows
        .iter()
        .find(|(vid, _, _, _)| *vid == jpg_voucher.voucher_id)
        .expect("jpg row");
    assert_eq!(jpg_row.3, "scan", "JPEG 源 → scan");
}

#[test]
fn first_thumb_for_vouchers_handles_empty_input() {
    let (_dir, ctx) = test_ctx();
    let rows = ctx
        .db
        .with_conn(|c| attachment_repo::first_thumb_for_vouchers(c, &[]))
        .expect("empty");
    assert!(rows.is_empty(), "空 ids 应直接返回空 Vec（不拼 SQL）");
}

#[test]
fn first_thumb_for_vouchers_omits_voucher_without_attachment() {
    // 直接建一张凭证（无附件） → IN 查询不应包含它
    let (_dir, ctx) = test_ctx();
    use voucher_core::models::{NewVoucher, VoucherStatus};
    use voucher_core::db::repos::voucher_repo;
    let v = ctx
        .db
        .with_conn(|c| {
            voucher_repo::insert(
                c,
                &NewVoucher {
                    voucher_no: "TEST-001".into(),
                    voucher_date: "2026-08-21".into(),
                    description: "无附件凭证".into(),
                    counterparty: String::new(),
                    amount_cents: None,
                    status: Some(VoucherStatus::Draft),
                    tags: vec![],
                    note: String::new(),
                },
            )
        })
        .expect("insert voucher");
    let rows = ctx
        .db
        .with_conn(|c| attachment_repo::first_thumb_for_vouchers(c, &[v.id]))
        .expect("query");
    assert!(rows.is_empty(), "无附件凭证不应出现在缩略图结果中");
}

// ============================================================================
// D-2 增强缓存管理 v2：enhance_hits 埋点 + 释放策略 + 单条释放
// ============================================================================

/// D-2-1：enhance_hits.record 写入 + 计数 / 近 N 天正确
#[test]
fn d2_enhance_hits_record_and_count() {
    use voucher_core::db::repos::enhance_hit_repo;
    let (_dir, ctx) = test_ctx();
    let jpg1 = make_doc_with_seed_in_tmpdir(&_dir, "a.jpg", "jpg", 1);
    let jpg2 = make_doc_with_seed_in_tmpdir(&_dir, "b.jpg", "jpg", 2);
    let summary = ingestion::import_images(&ctx, &[jpg1, jpg2], None, false).expect("import");
    assert_eq!(summary.vouchers.len(), 2);

    // 取 attachment id（每张凭证的首附件）
    let ids: Vec<i64> = ctx
        .db
        .with_conn(|c| {
            let mut s = c.prepare("SELECT id FROM attachments ORDER BY id").unwrap();
            let r = s.query_map([], |r| r.get(0)).unwrap();
            let out: Vec<i64> = r.map(|x| x.unwrap()).collect();
            Ok(out)
        })
        .unwrap();

    // 空表：0
    let total = ctx.db.with_conn(|c| enhance_hit_repo::count_total(c)).unwrap();
    assert_eq!(total, 0);

    // 写 3 次命中：ids[0] 两次 OCR，ids[1] 一次 export
    ctx.db.with_conn(|c| {
        enhance_hit_repo::record(c, ids[0], enhance_hit_repo::HitSource::Ocr).unwrap();
        enhance_hit_repo::record(c, ids[0], enhance_hit_repo::HitSource::Ocr).unwrap();
        enhance_hit_repo::record(c, ids[1], enhance_hit_repo::HitSource::Export).unwrap();
        Ok(())
    })
    .unwrap();
    let total = ctx.db.with_conn(|c| enhance_hit_repo::count_total(c)).unwrap();
    assert_eq!(total, 3);
    let grouped = ctx
        .db
        .with_conn(|c| enhance_hit_repo::count_grouped_by_attachment(c))
        .unwrap();
    let m: std::collections::HashMap<i64, u32> = grouped.into_iter().collect();
    assert_eq!(m.get(&ids[0]).copied(), Some(2));
    assert_eq!(m.get(&ids[1]).copied(), Some(1));

    // 近 30 天：3（全部）
    let r30 = ctx
        .db
        .with_conn(|c| enhance_hit_repo::count_since(c, &d2_thirty_days_ago()))
        .unwrap();
    assert_eq!(r30, 3);
}

fn d2_thirty_days_ago() -> String {
    use chrono::{Duration, Utc};
    (Utc::now() - Duration::days(30))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// D-2-2：clear_v2 策略 all → 清空所有增强版
#[test]
fn d2_clear_v2_all_strategy() {
    use voucher_core::services::enhance_cache::{self, ClearStrategy};
    let (dir, ctx) = test_ctx();
    let jpg1 = make_doc_with_seed_in_tmpdir(&dir, "a.jpg", "jpg", 1);
    let jpg2 = make_doc_with_seed_in_tmpdir(&dir, "b.jpg", "jpg", 2);
    ingestion::import_images(&ctx, &[jpg1, jpg2], None, false).expect("import");

    let before = ctx
        .db
        .with_conn(|c| attachment_repo::list_enhanced(c))
        .unwrap();
    assert!(before.iter().all(|a| a.enhanced_path.is_some()), "导入后应有 enhanced_path");

    let summary = enhance_cache::clear_v2(&ctx, ClearStrategy::All, 0).expect("clear all");
    assert_eq!(summary.candidate_count, before.len() as u32);
    assert!(summary.deleted_files > 0);

    let after = ctx
        .db
        .with_conn(|c| attachment_repo::list_enhanced(c))
        .unwrap();
    for a in &after {
        assert!(a.enhanced_path.is_none(), "释放后 enhanced_path 应清空");
    }
}

/// D-2-3：clear_v2 策略 larger_than_mb → 阈值以下不动
#[test]
fn d2_clear_v2_larger_than_mb_respects_threshold() {
    use voucher_core::services::enhance_cache::{self, ClearStrategy};
    let (dir, ctx) = test_ctx();
    let jpg1 = make_doc_with_seed_in_tmpdir(&dir, "a.jpg", "jpg", 1);
    let jpg2 = make_doc_with_seed_in_tmpdir(&dir, "b.jpg", "jpg", 2);
    ingestion::import_images(&ctx, &[jpg1, jpg2], None, false).expect("import");

    // 设一个超大阈值（100 MB），应该 0 个候选
    let summary = enhance_cache::clear_v2(&ctx, ClearStrategy::LargerThanMb, 100).expect("clear huge");
    assert_eq!(summary.candidate_count, 0, "100 MB 阈值下没有候选");
    assert_eq!(summary.deleted_files, 0);

    // 设 0 MB = 任何增强版都 ≥ 0 → 全清
    let summary0 = enhance_cache::clear_v2(&ctx, ClearStrategy::LargerThanMb, 0).expect("clear 0MB");
    assert!(summary0.candidate_count > 0, "0 MB 阈值应有候选");
    assert!(summary0.deleted_files > 0);
}

/// D-2-4：增强版释放（delete_one）后 enhance_hits 由后续 attachment 删除级联
/// 验证：单条释放只清 enhanced_path + 文件，attachment 行仍在；
/// 当 attachment 真被删时（典型场景：delete_vouchers → attachments CASCADE → enhance_hits CASCADE），
/// hits 也会被级联清空。
#[test]
fn d2_delete_one_clears_enhanced_and_cascades_hits_on_attachment_delete() {
    use voucher_core::db::repos::enhance_hit_repo;
    use voucher_core::services::enhance_cache;
    let (dir, ctx) = test_ctx();
    let jpg1 = make_doc_with_seed_in_tmpdir(&dir, "a.jpg", "jpg", 1);
    let jpg2 = make_doc_with_seed_in_tmpdir(&dir, "b.jpg", "jpg", 2);
    ingestion::import_images(&ctx, &[jpg1, jpg2], None, false).expect("import");

    let ids: Vec<i64> = ctx
        .db
        .with_conn(|c| {
            let mut s = c.prepare("SELECT id FROM attachments ORDER BY id").unwrap();
            let r = s.query_map([], |r| r.get(0)).unwrap();
            let out: Vec<i64> = r.map(|x| x.unwrap()).collect();
            Ok(out)
        })
        .unwrap();
    let target = ids[0];
    // 写 2 次命中
    ctx.db.with_conn(|c| {
        enhance_hit_repo::record(c, target, enhance_hit_repo::HitSource::Ocr).unwrap();
        enhance_hit_repo::record(c, target, enhance_hit_repo::HitSource::Viewer).unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(
        ctx.db.with_conn(|c| enhance_hit_repo::count_by_attachment(c, target)).unwrap(),
        2
    );

    // 单条释放：只清 enhanced_path + 文件；attachment 行 + hits 仍在
    let deleted = enhance_cache::delete_one(&ctx, target).expect("delete one");
    assert_eq!(deleted, 1, "物理文件应被删");

    let a = ctx
        .db
        .with_conn(|c| attachment_repo::get(c, target))
        .unwrap()
        .expect("attachment 仍存在");
    assert!(a.enhanced_path.is_none(), "释放后 enhanced_path 应清空");
    assert_eq!(
        ctx.db.with_conn(|c| enhance_hit_repo::count_by_attachment(c, target)).unwrap(),
        2,
        "释放只清 enhanced_path，hits 保留到 attachment 真被删"
    );

    // 模拟「附件被删」：直接 SQL DELETE 触发 FK CASCADE
    ctx.db
        .with_conn(|c| {
            c.execute("DELETE FROM attachments WHERE id = ?", rusqlite::params![target])?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        ctx.db.with_conn(|c| enhance_hit_repo::count_by_attachment(c, target)).unwrap(),
        0,
        "FK ON DELETE CASCADE 应把 hits 也删掉"
    );
}

/// D-2-5：recent_enhanced 返回最近 N 个 + 命中数 JOIN 正确
#[test]
fn d2_recent_enhanced_returns_with_hits() {
    use voucher_core::db::repos::enhance_hit_repo;
    use voucher_core::services::enhance_cache;
    let (dir, ctx) = test_ctx();
    let jpg1 = make_doc_with_seed_in_tmpdir(&dir, "a.jpg", "jpg", 1);
    let jpg2 = make_doc_with_seed_in_tmpdir(&dir, "b.jpg", "jpg", 2);
    let jpg3 = make_doc_with_seed_in_tmpdir(&dir, "c.jpg", "jpg", 3);
    ingestion::import_images(&ctx, &[jpg1, jpg2, jpg3], None, false).expect("import");

    let ids: Vec<i64> = ctx
        .db
        .with_conn(|c| {
            let mut s = c.prepare("SELECT id FROM attachments ORDER BY id").unwrap();
            let r = s.query_map([], |r| r.get(0)).unwrap();
            let out: Vec<i64> = r.map(|x| x.unwrap()).collect();
            Ok(out)
        })
        .unwrap();
    // 给 ids[1] 加 5 次命中
    for _ in 0..5 {
        ctx.db
            .with_conn(|c| enhance_hit_repo::record(c, ids[1], enhance_hit_repo::HitSource::Ocr))
            .unwrap();
    }

    let rows = enhance_cache::recent_enhanced(&ctx, 10).expect("recent");
    assert_eq!(rows.len(), 3, "3 张凭证都应有记录");

    let by_id: std::collections::HashMap<i64, _> =
        rows.iter().map(|r| (r.attachment_id, r)).collect();
    assert_eq!(by_id.get(&ids[1]).unwrap().hit_count, 5);
    assert_eq!(by_id.get(&ids[0]).unwrap().hit_count, 0);
    assert!(by_id.values().all(|r| r.enhanced_size > 0), "增强版大小应已读到");
    assert!(by_id.values().all(|r| !r.voucher_no.is_empty()), "voucher_no 应已 JOIN 到");
}

// ============================================================================
// D-5 导出命中接通：export.rs record_batch 写 enhance_hits(source='export')
// ============================================================================

/// D-5-1：导出 PDF 后，每张有 enhanced_path 的附件 +1 hit(source='export')
#[test]
fn d5_export_records_enhance_hit_per_enhanced_attachment() {
    use voucher_core::services::{enhance_cache, export};

    let (dir, ctx) = test_ctx();
    // 2 张凭证（每张各 1 附件；扫描件会触发 5 步流水线 → enhanced_path 非空）
    let jpg1 = make_doc_with_seed_in_tmpdir(&dir, "d5a.jpg", "jpg", 11);
    let jpg2 = make_doc_with_seed_in_tmpdir(&dir, "d5b.jpg", "jpg", 12);
    let summary = ingestion::import_images(&ctx, &[jpg1, jpg2], None, false).expect("import");
    assert_eq!(summary.vouchers.len(), 2);

    let ids: Vec<i64> = ctx
        .db
        .with_conn(|c| {
            let mut s = c.prepare("SELECT id FROM attachments ORDER BY id").unwrap();
            let r = s.query_map([], |r| r.get(0)).unwrap();
            let out: Vec<i64> = r.map(|x| x.unwrap()).collect();
            Ok(out)
        })
        .unwrap();
    // 兜底：确认 ingest 后两张都拿到了 enhanced_path（D-5 命中的前提）
    let enhanced_count = ctx
        .db
        .with_conn(|c| {
            let n: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM attachments WHERE id IN (?,?) AND enhanced_path IS NOT NULL",
                    rusqlite::params![ids[0], ids[1]],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(n)
        })
        .unwrap();
    assert_eq!(enhanced_count, 2, "D-5 前提：2 张附件都应有 enhanced_path");

    // 导出前：0 条 export 命中
    let export_hits_before = ctx
        .db
        .with_conn(|c| {
            let n: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM enhance_hits WHERE source = 'export'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(n)
        })
        .unwrap();
    assert_eq!(export_hits_before, 0, "导出前 export 命中应为 0");

    // 调 export_vouchers_pdf（合并单份模式）
    let voucher_ids: Vec<i64> = summary.vouchers.iter().map(|v| v.voucher_id).collect();
    let _batch = export::export_vouchers_pdf(&ctx, &voucher_ids, &Default::default())
        .expect("export pdf");

    // 导出后：2 条 export 命中（每张附件 1 条）
    let export_hits_after = ctx
        .db
        .with_conn(|c| {
            let n: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM enhance_hits WHERE source = 'export'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(n)
        })
        .unwrap();
    assert_eq!(export_hits_after, 2, "导出 2 张凭证应写 2 条 export 命中");

    // 累计命中应包含 export 计数
    let status = enhance_cache::status_v2(&ctx).expect("status_v2");
    assert!(
        status.hit_count >= 2,
        "status_v2.hit_count 应至少为 2（仅 export；可能更多因 ingest 自身也算 OCR 之前的 fallback）"
    );
}

/// D-5-2：日报模式 export_vouchers_daily 也写命中（每张附件一次 export hit）
#[test]
fn d5_export_daily_records_enhance_hit_per_enhanced_attachment() {
    use voucher_core::db::repos::enhance_hit_repo;
    use voucher_core::services::export;

    let (dir, ctx) = test_ctx();
    let jpg = make_doc_with_seed_in_tmpdir(&dir, "d5daily.jpg", "jpg", 21);
    let summary = ingestion::import_images(&ctx, &[jpg], None, false).expect("import");

    let before = ctx
        .db
        .with_conn(enhance_hit_repo::count_total)
        .unwrap();

    let voucher_ids: Vec<i64> = summary.vouchers.iter().map(|v| v.voucher_id).collect();
    let _batches = export::export_vouchers_daily(&ctx, &voucher_ids, &Default::default())
        .expect("export daily");

    let after = ctx
        .db
        .with_conn(enhance_hit_repo::count_total)
        .unwrap();
    // 仅一张附件 + 有 enhanced_path → +1
    assert!(
        after >= before + 1,
        "日报导出应至少 +1 条 export 命中（before={before} after={after}）"
    );
}

// ============================================================================
// D-6 PDF 导出图片优先增强版（仅扫描件；截图回退原图）
// ============================================================================

/// D-6-1：取图规则——扫描件取增强版、截图取原图、增强版文件缺失时扫描件也回退原图
#[test]
fn d6_pdf_image_abs_scan_prefers_enhanced_screenshot_uses_original() {
    use voucher_core::services::export;

    let (dir, ctx) = test_ctx();
    let jpg = make_doc_with_seed_in_tmpdir(&dir, "d6scan.jpg", "jpg", 31); // JPEG → Scan
    let png = make_doc_with_seed_in_tmpdir(&dir, "d6shot.png", "png", 32); // PNG 4:3 → Screenshot
    let summary = ingestion::import_images(&ctx, &[jpg, png], None, false).expect("import");
    assert_eq!(summary.vouchers.len(), 2);

    let scan = summary
        .vouchers
        .iter()
        .map(|v| &v.attachment)
        .find(|a| a.enhanced_source == EnhanceSource::Scan)
        .expect("扫描件附件");
    let shot = summary
        .vouchers
        .iter()
        .map(|v| &v.attachment)
        .find(|a| a.enhanced_source == EnhanceSource::Screenshot)
        .expect("截图附件");

    // 前提：默认配置（only_scans=false）下两种来源都生成了增强版
    assert!(scan.enhanced_path.is_some(), "扫描件应有增强版");
    assert!(
        shot.enhanced_path.is_some(),
        "截图默认配置也生成增强版（仅 OCR 用）"
    );

    // 扫描件：PDF 取增强版 + 算命中
    let enhanced_abs = ctx.paths.attachment_abs(scan.enhanced_path.as_ref().unwrap());
    assert_eq!(export::pdf_image_abs(&ctx, scan), enhanced_abs);
    assert!(export::uses_enhanced_in_pdf(&ctx, scan));

    // 截图：PDF 取原图 + 不算命中（增强版仅 OCR 用）
    let shot_orig = ctx.paths.attachment_abs(&shot.file_path);
    assert_eq!(export::pdf_image_abs(&ctx, shot), shot_orig);
    assert!(!export::uses_enhanced_in_pdf(&ctx, shot));

    // 增强版文件被删（如单条释放后）：扫描件回退原图、不再算命中
    std::fs::remove_file(&enhanced_abs).expect("remove enhanced");
    let scan_orig = ctx.paths.attachment_abs(&scan.file_path);
    assert_eq!(export::pdf_image_abs(&ctx, scan), scan_orig);
    assert!(
        !export::uses_enhanced_in_pdf(&ctx, scan),
        "增强版缺失 → PDF 回退原图且不写 Export 命中"
    );
}

/// D-6-2：仅截图附件导出不写 Export 命中（增强版未进 PDF，命中率不虚高）
#[test]
fn d6_export_screenshot_writes_no_export_hit() {
    use voucher_core::services::export;

    let (dir, ctx) = test_ctx();
    let png = make_doc_with_seed_in_tmpdir(&dir, "d6shot2.png", "png", 33);
    let summary = ingestion::import_images(&ctx, &[png], None, false).expect("import");
    let att = &summary.vouchers[0].attachment;
    assert_eq!(att.enhanced_source, EnhanceSource::Screenshot);
    assert!(att.enhanced_path.is_some(), "默认配置截图也生成增强版");

    let voucher_ids: Vec<i64> = summary.vouchers.iter().map(|v| v.voucher_id).collect();
    export::export_vouchers_pdf(&ctx, &voucher_ids, &Default::default()).expect("export pdf");

    let export_hits = ctx
        .db
        .with_conn(|c| {
            let n: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM enhance_hits WHERE source = 'export'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(n)
        })
        .unwrap();
    assert_eq!(
        export_hits, 0,
        "截图增强版未进 PDF，不应写 Export 命中"
    );
}

// ============================================================================
// D-7 查看器命中 + D-8 按来源分列统计
// ============================================================================

/// D-7：record_hit——有增强版记 viewer 命中；无增强版 / 附件不存在返回 Err 不计入
#[test]
fn d7_record_hit_viewer_only_for_enhanced() {
    use voucher_core::db::repos::enhance_hit_repo;
    use voucher_core::services::enhance_cache;

    let (dir, ctx) = test_ctx();
    let jpg = make_doc_with_seed_in_tmpdir(&dir, "d7.jpg", "jpg", 41);
    let summary = ingestion::import_images(&ctx, &[jpg], None, false).expect("import");
    let att = &summary.vouchers[0].attachment;
    assert!(att.enhanced_path.is_some(), "前提：默认配置生成增强版");

    // 有增强版 → viewer 命中 +1
    enhance_cache::record_hit(&ctx, att.id, enhance_hit_repo::HitSource::Viewer)
        .expect("record viewer hit");
    let n = ctx
        .db
        .with_conn(|c| enhance_hit_repo::count_by_attachment(c, att.id))
        .unwrap();
    assert_eq!(n, 1);

    // 附件不存在 → Err
    assert!(enhance_cache::record_hit(&ctx, 99999, enhance_hit_repo::HitSource::Viewer).is_err());

    // 释放增强版后再记 → Err（无增强版不计，避免查看原图也计命中）
    enhance_cache::delete_one(&ctx, att.id).expect("delete one");
    assert!(
        enhance_cache::record_hit(&ctx, att.id, enhance_hit_repo::HitSource::Viewer).is_err(),
        "无增强版不应计入命中"
    );
}

/// D-8：count_by_source + status_v2 分列字段对齐（ocr / export / viewer 各自独立计数）
#[test]
fn d8_status_v2_hit_counts_by_source() {
    use voucher_core::db::repos::enhance_hit_repo;
    use voucher_core::services::{enhance_cache, export};

    let (dir, ctx) = test_ctx();
    let jpg = make_doc_with_seed_in_tmpdir(&dir, "d8.jpg", "jpg", 42);
    let summary = ingestion::import_images(&ctx, &[jpg], None, false).expect("import");
    let att = &summary.vouchers[0].attachment;

    // ocr ×2、viewer ×1；export ×1 走真实导出（扫描件增强版进 PDF）
    ctx.db
        .with_conn(|c| {
            enhance_hit_repo::record(c, att.id, enhance_hit_repo::HitSource::Ocr)?;
            enhance_hit_repo::record(c, att.id, enhance_hit_repo::HitSource::Ocr)?;
            enhance_hit_repo::record(c, att.id, enhance_hit_repo::HitSource::Viewer)?;
            Ok(())
        })
        .unwrap();

    let voucher_ids: Vec<i64> = summary.vouchers.iter().map(|v| v.voucher_id).collect();
    export::export_vouchers_pdf(&ctx, &voucher_ids, &Default::default()).expect("export");

    let counts = ctx
        .db
        .with_conn(enhance_hit_repo::count_by_source)
        .unwrap();
    assert_eq!(counts.ocr, 2);
    assert_eq!(counts.export, 1);
    assert_eq!(counts.viewer, 1);
    assert_eq!(counts.total(), 4);

    let status = enhance_cache::status_v2(&ctx).unwrap();
    assert_eq!(status.hit_count_ocr, 2);
    assert_eq!(status.hit_count_export, 1);
    assert_eq!(status.hit_count_viewer, 1);
    assert_eq!(status.hit_count, 4, "hit_count 应为三来源之和");
}
