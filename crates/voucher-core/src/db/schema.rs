//! Schema（自由设计，v4.3 的 7 张表仅作功能参考）
//!
//! 版本管理用 `PRAGMA user_version`。旧数据不保留：不存在任何迁移逻辑，
//! 版本号只服务于新版自身的未来升级。
//!
//! v2 变化（一图一凭证）：
//! - vouchers 新增 counterparty（对方账户）
//! - attachments 对 voucher_id 建唯一索引：一张凭证最多一张图片（DB 层强约束）
//!
//! v3 变化（v0.6 文档扫描矫正）：
//! - attachments 新增 enhanced_path：5 步增强后的彩色 JPEG 路径（NULL = 未增强）
//! - attachments 新增 enhanced_source：来源分类（screenshot / scan / unknown）
//!
//! v4 变化（v0.6.5 增强缓存管理）：
//! - 新表 enhance_hits：记录「增强版被实际使用」（OCR / 导出 / 查看器），用于设置页命中率统计

use rusqlite::Connection;

use crate::Result;

pub const SCHEMA_VERSION: i64 = 4;

/// v4：v2 全部表 + 新表 enhance_hits（增强版被实际使用的次数）
pub const SCHEMA_V4: &str = r#"
CREATE TABLE IF NOT EXISTS vouchers (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    voucher_no    TEXT    NOT NULL UNIQUE,
    voucher_date  TEXT    NOT NULL,
    description   TEXT    NOT NULL DEFAULT '',
    counterparty  TEXT    NOT NULL DEFAULT '',
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
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    voucher_id      INTEGER NOT NULL REFERENCES vouchers(id) ON DELETE CASCADE,
    file_name       TEXT    NOT NULL,
    file_path       TEXT    NOT NULL,
    file_hash       TEXT    NOT NULL,
    file_size       INTEGER NOT NULL DEFAULT 0,
    mime_type       TEXT    NOT NULL DEFAULT 'application/octet-stream',
    ocr_status      TEXT    NOT NULL DEFAULT 'pending'
                    CHECK (ocr_status IN ('pending','running','done','failed')),
    ocr_text        TEXT,
    -- v3：v0.6 文档扫描矫正；NULL = 未增强（原图回退）
    enhanced_path   TEXT,
    enhanced_source TEXT    NOT NULL DEFAULT 'unknown'
                    CHECK (enhanced_source IN ('screenshot','scan','unknown')),
    created_at      TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_attachments_hash ON attachments(file_hash);
CREATE UNIQUE INDEX IF NOT EXISTS idx_attachments_voucher ON attachments(voucher_id);

-- v4：增强版被实际使用的次数（OCR 读取 / 导出 / 查看器打开都算 hit）
CREATE TABLE IF NOT EXISTS enhance_hits (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    attachment_id INTEGER NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    source        TEXT    NOT NULL
                  CHECK (source IN ('ocr','export','viewer')),
    hit_at        TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_enhance_hits_attachment ON enhance_hits(attachment_id);
CREATE INDEX IF NOT EXISTS idx_enhance_hits_hit_at     ON enhance_hits(hit_at);

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

/// v3 库的所有表（升级到 v4 时先整体清除；决策：旧数据不保留，清库重来）
const DROP_ALL_V3: &str = r#"
DROP TABLE IF EXISTS enhance_hits;
DROP TABLE IF EXISTS export_items;
DROP TABLE IF EXISTS export_batches;
DROP TABLE IF EXISTS snapshots;
DROP TABLE IF EXISTS attachments;
DROP TABLE IF EXISTS ocr_cache;
DROP TABLE IF EXISTS config;
DROP TABLE IF EXISTS vouchers;
"#;

/// 初始化/升级 schema。
///
/// v1（多图凭证）→ v2（一图一凭证）：不做数据迁移，v1 表整体删除重建
/// （唯一索引无法在多图存量上建立；清库重来为既定决策）。
/// v2/v3 → v4：同样清库重建（v4 新增 enhance_hits 表；增强版 JPEG 物理文件保留，下次导入同 hash 自动复用）。
pub fn init(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < SCHEMA_VERSION {
        if version >= 1 {
            log::info!("检测到 v{version} 旧库，按「清库重来」策略重建为 v{SCHEMA_VERSION}");
        }
        conn.execute_batch(DROP_ALL_V3)?;
        conn.execute_batch(SCHEMA_V4)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}
