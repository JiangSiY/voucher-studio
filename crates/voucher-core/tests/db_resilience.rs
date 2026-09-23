//! DB 容错与并发回归（体检第三批第 15 项）：
//! 损坏数据库文件不 panic、多线程并发写经 UoW 串行化不丢数据、
//! with_tx 闭包 Err 时整体回滚零残留。

mod common;

use voucher_core::db;
use voucher_core::models::NewVoucher;
use voucher_core::services::voucher_mgr;
use voucher_core::{CoreContext, CoreError, Result};

/// 介质损坏（垃圾字节当数据库）：打开应报错而非 panic，
/// 且错误是可展示的 Db 变体（应用层能引导用户走恢复流程）。
#[test]
fn corrupted_db_file_fails_open_gracefully() {
    let dir = tempfile::tempdir().unwrap();
    // 运行库固定在数据目录根：base/voucher.db（AppPaths::db_file）
    let db_path = dir.path().join("voucher.db");
    std::fs::write(&db_path, b"this is definitely not a sqlite database!!!!").unwrap();

    let r = db::connection::open(&db_path);
    assert!(
        r.is_err(),
        "垃圾字节文件应报错（WAL PRAGMA 会读 header），实际 Ok"
    );
    // CoreContext::init 全链路也不应 panic（迁移或打开失败转为 Err）
    let r2 = CoreContext::init(Some(dir.path().to_path_buf()));
    assert!(r2.is_err(), "损坏库 init 应整体报错");
}

/// 契约守卫（2026-09-23 审计 B8）：OCR 推理**不得**被 `with_conn` 包住。
///
/// 背景：全局只有一条 DB 连接（`Mutex<Connection>`），推理期间持锁会让列表、保存、
/// 备份等**所有** DB 操作排队等待——本地 ONNX 每张数百 ms 至数秒，云端超时上限 60s。
/// 旧实现把「查缓存 → 推理 → 写缓存」写在同一个 `with_conn` 闭包里，正是这个症状。
///
/// 这是**结构式守卫**（与 CI 的越层 grep 门禁同风格）：静态读源码文本，禁止
/// ①`with_conn` 附近出现推理调用、②推理函数接收 `Connection`、③旧的合并实现回归。
/// 真正的行为级验证（推理期间 DB 可写）需要可注入的慢引擎，本轮未做。
#[test]
fn ocr_inference_must_not_hold_db_lock() {
    let src = include_str!("../src/services/ocr.rs");

    // ① with_conn 之后紧跟的若干行内不得出现推理调用
    let lines: Vec<&str> = src.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.contains("with_conn") {
            let window = lines[i..(i + 8).min(lines.len())].join("\n");
            assert!(
                !window.contains("recognize("),
                "第 {} 行起的 with_conn 闭包内出现推理调用：推理不得持 DB 锁",
                i + 1
            );
        }
    }

    // ② 推理函数不得接收数据库连接
    let pos = src
        .find("fn infer_ocr(")
        .expect("infer_ocr 应存在（推理入口）");
    let end = src[pos..].find(')').expect("签名应有右括号") + pos;
    assert!(
        !src[pos..end].contains("Connection"),
        "infer_ocr 不得接收 Connection：推理必须在锁外"
    );

    // ③ 旧的「推理 + 缓存」合并实现不得回归
    assert!(
        !src.contains("fn recognize_with_cache("),
        "推理不得再与缓存读写合并进同一个持锁函数（见 cached_ocr / infer_ocr / store_ocr_cache）"
    );
}

/// 多线程并发写入：UoW 单连接 + Mutex 串行化下，
/// 8 线程 × 10 张凭证全部落库、无丢失、无重复编号。
#[test]
fn concurrent_writes_serialize_without_loss() -> Result<()> {
    let (_dir, ctx) = common::test_ctx();
    let ctx = std::sync::Arc::new(ctx);
    let threads = 8;
    let per_thread = 10;

    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for t in 0..threads {
            let ctx = std::sync::Arc::clone(&ctx);
            handles.push(s.spawn(move || -> Result<()> {
                for i in 0..per_thread {
                    voucher_mgr::create_voucher(
                        &ctx,
                        NewVoucher {
                            description: format!("并发-线程{t}-{i}"),
                            voucher_no: format!("20260909-C{t:02}{i:02}"),
                            ..Default::default()
                        },
                    )?;
                }
                Ok(())
            }));
        }
        for h in handles {
            h.join().expect("线程不应 panic").expect("写入应成功");
        }
    });

    assert_eq!(
        common::voucher_count(&ctx)?,
        (threads * per_thread) as i64,
        "并发写入总数应精确"
    );
    Ok(())
}

/// with_tx 原子性：闭包中途返回 Err → 已执行的语句整体回滚，零残留。
#[test]
fn with_tx_rolls_back_on_closure_error() -> Result<()> {
    let (_dir, ctx) = common::test_ctx();
    let err: Result<()> = ctx.db.with_tx(|tx| {
        tx.execute(
            "INSERT INTO vouchers (voucher_no, voucher_date, description, status, tags, created_at, updated_at)
             VALUES ('20260909-RB', '2026-09-09', '将回滚', 'draft', '[]', '2026-09-09T00:00:00', '2026-09-09T00:00:00')",
            [],
        )?;
        Err(CoreError::Invalid("模拟事务中途失败".into()))
    });
    assert!(err.is_err(), "闭包 Err 应上抛");

    let n: i64 = ctx.db.with_conn(|conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM vouchers WHERE voucher_no = '20260909-RB'",
            [],
            |r| r.get(0),
        )
        .map_err(CoreError::from)
    })?;
    assert_eq!(n, 0, "回滚后不应有残留行");
    Ok(())
}

/// 同一 CoreContext 并发混用 with_conn / with_tx：
/// 串行化保证下互不干扰（读侧始终看到已提交数据）。
#[test]
fn mixed_conn_and_tx_concurrent_reads_see_committed_only() -> Result<()> {
    let (_dir, ctx) = common::test_ctx();
    let ctx = std::sync::Arc::new(ctx);

    voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "基线".into(),
            voucher_no: "20260909-BASE".into(),
            ..Default::default()
        },
    )?;

    std::thread::scope(|s| {
        let ctx2 = std::sync::Arc::clone(&ctx);
        let writer = s.spawn(move || -> Result<()> {
            for i in 0..5 {
                voucher_mgr::create_voucher(
                    &ctx2,
                    NewVoucher {
                        description: format!("追加-{i}"),
                        voucher_no: format!("20260909-M{i}"),
                        ..Default::default()
                    },
                )?;
            }
            Ok(())
        });
        let reader = s.spawn(move || -> Result<i64> {
            let mut seen: Vec<i64> = Vec::new();
            for _ in 0..20 {
                let n = common::voucher_count(&ctx)?;
                seen.push(n);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(*seen.iter().max().unwrap())
        });
        writer.join().unwrap().expect("写入成功");
        let max_seen = reader.join().unwrap().expect("读取成功");
        assert_eq!(max_seen, 6, "读侧最终应看到全部已提交数据");
    });
    Ok(())
}
