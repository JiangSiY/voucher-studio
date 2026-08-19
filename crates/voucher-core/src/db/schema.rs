//! Schema（自由设计，v4.3 的 7 张表仅作功能参考）
//!
//! 版本管理用 `PRAGMA user_version`。旧数据不保留：不存在任何迁移逻辑，
//! 版本号只服务于新版自身的未来升级。

use rusqlite::Connection;

use crate::Result;

pub const SCHEMA_VERSION: i64 = 1;

/// v1：七张表
/// vouchers / attachments / export_batches / export_items / snapshots / ocr_cache / config
pub const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS vouchers (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    voucher_no    TEXT    NOT NULL UNIQUE,
    voucher_date  TEXT    NOT NULL,
    description   TEXT    NOT NULL DEFAULT '',
    amount_cents  INTEGER,
    status        TEXT    NOT NULL DEFAULT 'draft'
                  CHECK (status IN ('draft','reviewed','exported','archived')),
    tags          TEXT    NOT NULL DEFAULT '[]',
    note          TEXT    NOT NULL DEFAULT '',
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_vouchers_date   ON vouchers(voucher_date);
CREATE INDEX IF NOT EXISTS idx_vouchers_status ON vouchers(status);

CREATE TABLE IF NOT EXISTS attachments (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    voucher_id  INTEGER NOT NULL REFERENCES vouchers(id) ON DELETE CASCADE,
    file_name   TEXT    NOT NULL,
    file_path   TEXT    NOT NULL,
    file_hash   TEXT    NOT NULL,
    file_size   INTEGER NOT NULL DEFAULT 0,
    mime_type   TEXT    NOT NULL DEFAULT 'application/octet-stream',
    ocr_status  TEXT    NOT NULL DEFAULT 'pending'
                CHECK (ocr_status IN ('pending','running','done','failed')),
    ocr_text    TEXT,
    created_at  TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_attachments_voucher ON attachments(voucher_id);
CREATE INDEX IF NOT EXISTS idx_attachments_hash    ON attachments(file_hash);

CREATE TABLE IF NOT EXISTS export_batches (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_no      TEXT    NOT NULL UNIQUE,
    voucher_count INTEGER NOT NULL DEFAULT 0,
    pdf_path      TEXT    NOT NULL,
    status        TEXT    NOT NULL DEFAULT 'done'
                  CHECK (status IN ('done','failed')),
    created_at    TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS export_items (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id    INTEGER NOT NULL REFERENCES export_batches(id) ON DELETE CASCADE,
    voucher_id  INTEGER NOT NULL REFERENCES vouchers(id) ON DELETE CASCADE,
    sort_order  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_export_items_batch ON export_items(batch_id);

CREATE TABLE IF NOT EXISTS snapshots (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    voucher_id  INTEGER NOT NULL REFERENCES vouchers(id) ON DELETE CASCADE,
    payload     TEXT    NOT NULL,
    reason      TEXT    NOT NULL DEFAULT '',
    created_at  TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_snapshots_voucher ON snapshots(voucher_id);

CREATE TABLE IF NOT EXISTS ocr_cache (
    cache_key  TEXT PRIMARY KEY,
    engine     TEXT NOT NULL,
    text       TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS config (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

/// 初始化/升级 schema（幂等）
pub fn init(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(SCHEMA_V1)?;
        conn.pragma_update(None, "user_version", 1)?;
    }
    Ok(())
}
