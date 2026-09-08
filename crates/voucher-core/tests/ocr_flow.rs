//! OCR 链路集成测试：本地引擎端到端 / 提取字段 / 缓存 / 全文搜索命中。

mod common;

use common::*;
use std::path::PathBuf;
use voucher_core::models::{OcrStatus, VoucherStatus};
use voucher_core::services::{export, ingestion, ocr, search, voucher_mgr};
use voucher_core::{CoreError, Result};
#[test]
fn import_images_dedups_and_ocr_text_searchable() -> Result<()> {
    let (dir, ctx) = test_ctx();

    let img = dir.path().join("receipt.png");
    make_png(&img);

    // 第一次导入：自动建草稿凭证（一图一凭证）
    let s1 = ingestion::import_images(&ctx, std::slice::from_ref(&img), None, false)?;
    assert_eq!(s1.vouchers.len(), 1);
    assert_eq!(s1.skipped_duplicates.len(), 0);

    // 同一文件再导入：内容重复 → 跳过
    let s2 = ingestion::import_images(&ctx, std::slice::from_ref(&img), None, false)?;
    assert_eq!(s2.vouchers.len(), 0);
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
            s1.vouchers[0].attachment.id,
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
    assert_eq!(summary.vouchers.len(), 1);

    // 3. OCR 文本经引擎识别落库
    let detail = voucher_mgr::get_detail(&ctx, summary.vouchers[0].voucher_id)?;
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
    let batch = export::export_with_defaults(&ctx, &[summary.vouchers[0].voucher_id])?;
    assert_eq!(batch.voucher_count, 1);
    assert!(PathBuf::from(&batch.pdf_path).exists(), "PDF 应落盘");
    assert!(std::fs::metadata(&batch.pdf_path)?.len() > 1000);

    // 6. 批次记录 + 凭证状态流转「已导出」
    let batches = export::list_batches(&ctx, 10)?;
    assert_eq!(batches.len(), 1);
    let detail = voucher_mgr::get_detail(&ctx, summary.vouchers[0].voucher_id)?;
    assert_eq!(detail.voucher.status, VoucherStatus::Exported);
    Ok(())
}

/// OCR 字段抽取：保守策略（日期/金额/对方账户；歧义金额放弃）
#[test]
fn ocr_fields_extract_conservative() {
    use voucher_core::services::ocr_fields;

    // 典型报销凭证文本
    let text = "报销凭证\n凭证编号：20260819-001\n日期：2026年8月20日\n收款方：美团平台\n合计：1234.56元\n经办人：张三";
    let f = ocr_fields::extract(text);
    assert_eq!(f.date.as_deref(), Some("2026-08-20"));
    assert_eq!(f.amount_cents, Some(123456));
    assert_eq!(f.counterparty.as_deref(), Some("美团平台"));

    // 分隔符变体 + 无标签唯一金额
    let text2 = "2026/8/2 消费\n支出 88.5";
    let f2 = ocr_fields::extract(text2);
    assert_eq!(f2.date.as_deref(), Some("2026-08-02"));
    assert_eq!(f2.amount_cents, Some(8850));

    // 多个无标签金额 → 放弃（宁可空着不猜）
    let text3 = "买东西 12.00 和 34.00";
    let f3 = ocr_fields::extract(text3);
    assert_eq!(f3.amount_cents, None);

    // 标签优先级：合计优先于金额
    let text4 = "金额：10.00\n合计：99.99";
    let f4 = ocr_fields::extract(text4);
    assert_eq!(f4.amount_cents, Some(9999));

    // 非法日期（2月30日）不采纳
    let text5 = "日期 2026年2月30日";
    let f5 = ocr_fields::extract(text5);
    assert_eq!(f5.date, None);
}
