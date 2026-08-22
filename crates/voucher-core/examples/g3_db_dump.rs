//! G3 走查：只读 dump 生产数据库（应用运行中也可安全读取，WAL 并发读）
//!
//! 用法：cargo run -p voucher-core --example g3_db_dump
//!
//! 输出：user_version / 凭证清单 / 附件 OCR 状态 / 快照数 / ocr_cache 数
//! 只读打开，不触碰 .lock，不执行迁移。

use rusqlite::OpenFlags;

fn main() {
    let data_dir = std::env::var("APPDATA")
        .map(|p| {
            std::path::PathBuf::from(p)
                .join("VoucherStudio")
                .join("data")
        })
        .expect("APPDATA");
    let db_path = data_dir.join("voucher.db");
    println!("DB: {}", db_path.display());

    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("open read-only");

    let ver: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(-1);
    println!("user_version = {ver}");

    println!("\n== vouchers ==");
    {
        let mut stmt = conn
            .prepare("SELECT id, voucher_no, voucher_date, description, amount_cents, status FROM vouchers ORDER BY id")
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        while let Ok(Some(r)) = rows.next() {
            let id: i64 = r.get(0).unwrap();
            let no: String = r.get(1).unwrap();
            let date: String = r.get(2).unwrap();
            let desc: String = r.get(3).unwrap();
            let amt: Option<i64> = r.get(4).unwrap();
            let status: String = r.get(5).unwrap();
            println!("  #{id} {no} {date} [{status}] amt={amt:?} desc={desc}");
        }
    }

    println!("\n== attachments ==");
    {
        let mut stmt = conn
            .prepare("SELECT id, voucher_id, file_name, ocr_status, substr(coalesce(ocr_text,''),1,60), enhanced_source, enhanced_path IS NOT NULL FROM attachments ORDER BY id")
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        while let Ok(Some(r)) = rows.next() {
            let id: i64 = r.get(0).unwrap();
            let vid: i64 = r.get(1).unwrap();
            let name: String = r.get(2).unwrap();
            let st: String = r.get(3).unwrap();
            let text: String = r.get(4).unwrap();
            let src: String = r.get(5).unwrap();
            let enh: bool = r.get(6).unwrap();
            println!("  att#{id} v{vid} [{st}] enh={enh}({src}) {name} | ocr: {text}");
        }
    }

    println!("\n== snapshots ==");
    {
        let mut stmt = conn
            .prepare("SELECT voucher_id, reason, count(*) FROM snapshots GROUP BY voucher_id, reason ORDER BY voucher_id")
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        while let Ok(Some(r)) = rows.next() {
            let vid: i64 = r.get(0).unwrap();
            let reason: String = r.get(1).unwrap();
            let n: i64 = r.get(2).unwrap();
            println!("  v{vid} reason={reason} x{n}");
        }
    }

    let n_ocr: i64 = conn
        .query_row("SELECT count(*) FROM ocr_cache", [], |r| r.get(0))
        .unwrap_or(-1);
    println!("\nocr_cache rows = {n_ocr}");

    let n_hits: i64 = conn
        .query_row("SELECT count(*) FROM enhance_hits", [], |r| r.get(0))
        .unwrap_or(-1);
    println!("enhance_hits rows = {n_hits}");

    let n_batches: i64 = conn
        .query_row("SELECT count(*) FROM export_batches", [], |r| r.get(0))
        .unwrap_or(-1);
    println!("export_batches rows = {n_batches}");
}
