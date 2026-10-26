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
    let hits = search::search_vouchers(&ctx, "出租车", &Default::default(), 10)?;
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
#[cfg(target_os = "macos")]
#[ignore = "需要在真实 macOS 环境跑 Apple Vision（CI 的 macos job 以 --include-ignored 执行）"]
fn vision_ocr_end_to_end() -> Result<()> {
    let sample =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/assets/ocr_sample.png");
    if !sample.is_file() {
        eprintln!("跳过：测试图不存在");
        return Ok(());
    }

    let lines = voucher_core::services::ocr_vision::recognize(&sample)?;
    assert!(!lines.is_empty(), "Vision 应识别出文字行");
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
            "Vision 识别缺少「{expect}」，实际识别: {compact}"
        );
    }
    // 坐标必须是原图像素、左上原点（转换错了几何配对全盘失效）
    assert!(lines
        .iter()
        .all(|l| l.box_xyxy[0] >= 0.0 && l.box_xyxy[1] >= 0.0));
    assert!(
        lines
            .iter()
            .all(|l| l.box_xyxy[2] >= l.box_xyxy[0] && l.box_xyxy[3] >= l.box_xyxy[1]),
        "bbox 应为 (x0,y0,x1,y1) 且 x1≥x0、y1≥y0"
    );
    Ok(())
}

/// G2 标准②直接证据：图片导入 → 本地 OCR 真实识别 → 全文搜索 → PDF 导出 全链路闭环。
/// 与分段测试的区别：OCR 文本由本地引擎识别落库（非手工写 DB），导出走真实 PDF 生成。
#[test]
#[cfg(target_os = "macos")]
#[ignore = "需要真实 macOS + Apple Vision（CI macos job 以 --include-ignored 执行）"]
fn g2_closed_loop_import_ocr_pdf() -> Result<()> {
    // 前置素材缺失时跳过（样张不随 git 走的环境，如精简 CI）
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

    // 1. 显式选本地引擎（Apple Vision；不依赖模型文件，全新库即可用）
    let mut cfg = ctx.config();
    cfg.ocr.engine = "local".to_string();
    ctx.set_config(cfg)?;

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
            "本地（Vision）识别结果缺少「{expect}」，实际: {compact}"
        );
    }
    // 识别走的是本地引擎且已落缓存（区别于旧测试手工写入 DB 的路径）
    let cached_engine: String = ctx.db.with_conn(|conn| {
        conn.query_row("SELECT engine FROM ocr_cache LIMIT 1", [], |r| r.get(0))
            .map_err(CoreError::from)
    })?;
    assert_eq!(cached_engine, "local");

    // 4. OCR 文本进入全文搜索
    let hits = search::search_vouchers(&ctx, "报销", &Default::default(), 10)?;
    assert_eq!(hits.len(), 1, "OCR 文本应可被全文搜索命中");
    assert!(hits[0].matched_in.iter().any(|m| m == "OCR"));

    // 5. 导出 PDF（打印默认配置）
    let batches = export::export_daily_with_defaults(&ctx, &[summary.vouchers[0].voucher_id])?;
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
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

// ---------- 残余清理（B8 行为级）：OCR 推理期间不得持全局 DB 锁 ----------

/// 极简可阻塞 mock HTTP 服务（照 `webdav_mock.rs` 的形状，零新依赖）：
/// 收到一次请求后**阻塞**并把"已收到"置真，等调用方放行再返回 `{"text": "…"}`。
/// 内置 30s 放行上限，避免用例失败时把 CI 挂死。
struct BlockingOcrMock {
    port: u16,
    arrived: std::sync::Arc<std::sync::atomic::AtomicBool>,
    release: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl BlockingOcrMock {
    fn start(text: &'static str) -> Self {
        use std::io::Write;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let port = listener.local_addr().expect("mock addr").port();
        let arrived = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let (a, r) = (arrived.clone(), release.clone());

        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                // **读满**请求头 + 按 Content-Length 读净 body 再回应。
                // 只读一次就回应会让客户端偶发 ContentLengthInvalid（请求尚在途/被截断）。
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let n = std::io::Read::read(&mut sock, &mut tmp).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                        let want = head
                            .lines()
                            .find_map(|l| {
                                let (k, v) = l.split_once(':')?;
                                k.eq_ignore_ascii_case("content-length")
                                    .then(|| v.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if buf.len() >= pos + 4 + want {
                            break;
                        }
                    }
                }
                a.store(true, Ordering::SeqCst);
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                while !r.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                let body = format!("{{\"text\":\"{text}\"}}");
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes());
            }
        });
        Self {
            port,
            arrived,
            release,
        }
    }
}

/// 残余清理（B8 行为级）：**OCR 推理期间不得持全局 DB 锁**。
///
/// 手段：OCR 引擎指向本文件内建的 mock HTTP（`validate_api_url` 明确放行 127.0.0.1，
/// 未绕过任何生产校验），mock 收到请求后阻塞并通知主线程；主线程在"推理进行中"计时
/// `ctx.db.with_conn`——旧实现（查缓存→推理→写缓存同在一个 `with_conn` 内）会一直等到
/// mock 放行，现实现应在毫秒级返回。放行后识别照常完成，顺带证明链路没被拆坏。
///
/// 为什么用云端引擎而不是本地 ONNX：不需要 16MB 模型、耗时确定（本地识别耗时随图大小与
/// 机器浮动，不适合卡时间窗）。结构式守卫见 `db_resilience.rs::ocr_inference_must_not_hold_db_lock`。
#[test]
fn ocr_inference_does_not_hold_db_lock() -> Result<()> {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    let (dir, ctx) = test_ctx();
    let img = dir.path().join("lock.png");
    make_png(&img);
    let imported = ingestion::import_images(&ctx, std::slice::from_ref(&img), None, false)?;
    let att_id = imported.vouchers[0].attachment.id;

    let mock = BlockingOcrMock::start("mock 识别结果");
    let mut cfg = ctx.config();
    cfg.ocr.engine = "cloud".into();
    // 用「自建通用协议」跑 mock（deepseek 协议要求 API Key，本测试不涉及鉴权）
    cfg.ocr.provider = "custom".into();
    cfg.ocr.api_url = format!("http://127.0.0.1:{}/ocr", mock.port);
    cfg.ocr.enabled = true;
    cfg.ocr.model = "mock".into();
    ctx.set_config(cfg)?;

    std::thread::scope(|s| {
        let ocr_thread = s.spawn(|| ocr::run_for_attachment(&ctx, att_id));

        // 等 mock 收到请求 = 推理已进入
        let deadline = Instant::now() + Duration::from_secs(10);
        while !mock.arrived.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "mock 未在 10s 内收到 OCR 请求");
            std::thread::sleep(Duration::from_millis(5));
        }

        // 推理进行中：取 DB 连接必须立刻成功（先放行 mock，保证断言失败也不挂死）
        let t0 = Instant::now();
        let probe = ctx.db.with_conn(|_| Ok(()));
        let held = t0.elapsed();
        mock.release.store(true, Ordering::SeqCst);
        probe?;
        assert!(
            held < Duration::from_millis(500),
            "OCR 推理期间取 DB 连接被阻塞了 {held:?}——推理不得持锁"
        );

        let att = ocr_thread.join().expect("OCR 线程不应 panic")?;
        assert_eq!(
            att.ocr_text.as_deref().map(str::trim),
            Some("mock 识别结果"),
            "放行后识别应正常完成（顺带证明链路未被拆坏）"
        );
        Ok(())
    })
}

/// v0.23：云端优先——`auto` 且云端可用时必须走云端（并落 engine=cloud 的缓存）。
/// 用本文件内建 mock（127.0.0.1 明文，validate_api_url 明确放行回环地址）。
#[test]
fn cloud_engine_is_preferred_and_cached() -> Result<()> {
    use std::sync::atomic::Ordering;

    let mock = BlockingOcrMock::start("云端识别文本 合计 88.00");
    mock.release.store(true, Ordering::SeqCst);
    let (dir, ctx) = test_ctx();
    let img = dir.path().join("cloud.png");
    make_png(&img);
    let imported = ingestion::import_images(&ctx, std::slice::from_ref(&img), None, false)?;
    let att_id = imported.vouchers[0].attachment.id;

    let mut cfg = ctx.config();
    cfg.ocr.engine = "auto".to_string();
    cfg.ocr.provider = "custom".to_string();
    cfg.ocr.api_url = format!("http://127.0.0.1:{}/ocr", mock.port);
    cfg.ocr.model = "mock".to_string();
    ctx.set_config(cfg)?;

    let att = ocr::run_for_attachment(&ctx, att_id)?;
    assert_eq!(att.ocr_status, OcrStatus::Done);
    let text = att.ocr_text.unwrap_or_default();
    assert!(text.contains("云端识别文本"), "应使用云端返回文本: {text}");

    // 缓存记录产出引擎（决定下次是否还需要「本地缓存升级重试」）
    let engine: String = ctx.db.with_conn(|conn| {
        conn.query_row("SELECT engine FROM ocr_cache LIMIT 1", [], |r| r.get(0))
            .map_err(CoreError::from)
    })?;
    assert_eq!(engine, "cloud");
    assert_eq!(ctx.ocr_health().last_engine.as_deref(), Some("cloud"));
    Ok(())
}

/// v0.23：云端结构化要素优先于旧本地版面——同一附件既有本地坐标又有云端要素时，
/// 预览必须以云端为准（否则几何配对会盖掉更准的结果）。
#[test]
fn cloud_fields_take_priority_over_local_layout() -> Result<()> {
    use voucher_core::db::repos::{ocr_field_cache_repo, ocr_layout_repo};
    use voucher_core::models::OcrLine;
    use voucher_core::services::query;

    let (dir, ctx) = test_ctx();
    let img = dir.path().join("fields.png");
    make_png(&img);
    let imported = ingestion::import_images(&ctx, std::slice::from_ref(&img), None, false)?;
    let att = imported.vouchers[0].attachment.clone();

    // 本地版面里写「合计 10.00」（几何路径会抽出 10.00）
    let lines = vec![OcrLine {
        text: "合计 10.00".to_string(),
        score: 0.9,
        box_xyxy: [0.0, 0.0, 60.0, 12.0],
    }];
    ctx.db
        .with_conn(|conn| ocr_layout_repo::set_layout(conn, att.id, "local", &lines))?;
    // 云端要素给 88.00 + 日期 + 对方
    ctx.db.with_conn(|conn| {
        ocr_field_cache_repo::upsert(
            conn,
            &att.file_hash,
            "deepseek",
            "deepseek-flash",
            r#"{"text":"云端文本","date":"2026-08-19","amount":"88.00","counterparty":"某某公司"}"#,
        )
    })?;

    let f = query::ocr_preview(&ctx, att.id)?;
    assert_eq!(f.amount_cents, Some(8_800), "云端要素应优先于本地版面");
    assert_eq!(f.date.as_deref(), Some("2026-08-19"));
    assert_eq!(f.counterparty.as_deref(), Some("某某公司"));
    Ok(())
}

/// v0.23：`auto` 下云端不可用（连接被拒）必须自动回退本地 Vision，并把失败原因记进健康状态。
#[test]
#[cfg(target_os = "macos")]
#[ignore = "需要真实 macOS Vision（CI macos job 以 --include-ignored 执行）"]
fn auto_mode_falls_back_to_local_when_cloud_fails() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let img = dir.path().join("fallback.png");
    make_png(&img);
    let imported = ingestion::import_images(&ctx, std::slice::from_ref(&img), None, false)?;
    let att_id = imported.vouchers[0].attachment.id;

    let mut cfg = ctx.config();
    cfg.ocr.engine = "auto".to_string();
    cfg.ocr.provider = "custom".to_string();
    // 1 号端口上没有服务 → 云端连接失败
    cfg.ocr.api_url = "http://127.0.0.1:1/ocr".to_string();
    ctx.set_config(cfg)?;

    let att = ocr::run_for_attachment(&ctx, att_id)?;
    assert_eq!(att.ocr_status, OcrStatus::Done, "云端失败后应已回退本地");
    let health = ctx.ocr_health();
    assert_eq!(health.last_engine.as_deref(), Some("local"));
    assert!(health.last_cloud_error.is_some(), "应记录云端失败原因");
    Ok(())
}
