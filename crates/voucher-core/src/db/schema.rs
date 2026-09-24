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

/// v5 附加表：OCR 版面数据（每行文本 + 置信度 + 坐标框，JSON 存储）。
/// 与 v4 主体 schema 解耦：`CREATE TABLE IF NOT EXISTS` 幂等执行，
/// 对已存在的 v4 库无损新增（不清库、不动 attachments）。
pub const SCHEMA_LAYOUT: &str = r#"
CREATE TABLE IF NOT EXISTS ocr_layouts (
    attachment_id INTEGER PRIMARY KEY REFERENCES attachments(id) ON DELETE CASCADE,
    lines         TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL
);
"#;

/// v6 附加结构（v0.10/v0.11）：疑似重复检测索引 + 会计期间锁。
/// 幂等执行；`export_batches.format` 列与 `period_locks` 表见 `init` 中的 Rust 侧补齐。
pub const SCHEMA_V6: &str = r#"
-- 疑似重复检测（同日期 + 同金额分组扫描）
CREATE INDEX IF NOT EXISTS idx_vouchers_dup ON vouchers(voucher_date, amount_cents);
-- 会计期间锁（v0.11：按月封账；period = YYYY-MM）
CREATE TABLE IF NOT EXISTS period_locks (
    period    TEXT PRIMARY KEY,
    locked_at TEXT NOT NULL
);
"#;

/// v7 附加表（v0.16）：疑似重复的**处置记录**。
///
/// 背景：语义层"疑似重复"（同日期 + 同金额）自 v0.10 起只检测不处理——结果每次重算、
/// 无任何持久化，于是面板/提示永远重现，用户无处处置（2026-09-23 用户报"很鸡肋"）。
/// 本表记录**逐对判定**：同一对只存一行（`a_id < b_id` 归一化 + 唯一索引）。
///
/// **刻意不加外键**：这是审计日志。若加 `ON DELETE CASCADE`，删掉被删凭证会把留痕一起
/// 冲掉——而"确认重复后删掉多余那一份"恰恰是本表要留痕的动作（`removed_payload` 存
/// 被删凭证的完整 JSON + 附件清单）。删除是本项目不可恢复的操作（快照随凭证级联删除），
/// 所以留痕必须独立于凭证生命周期。
pub const SCHEMA_V7: &str = r#"
CREATE TABLE IF NOT EXISTS dup_decisions (
    id              INTEGER PRIMARY KEY,
    a_id            INTEGER NOT NULL,
    b_id            INTEGER NOT NULL,
    decision        TEXT    NOT NULL CHECK (decision IN ('not_duplicate','duplicate_removed')),
    keep_id         INTEGER,
    removed_id      INTEGER,
    removed_payload TEXT,
    note            TEXT    NOT NULL DEFAULT '',
    created_at      TEXT    NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_dup_decisions_pair ON dup_decisions(a_id, b_id);
CREATE INDEX IF NOT EXISTS idx_dup_decisions_created ON dup_decisions(created_at DESC);
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
    // v5：OCR 版面表（幂等，新旧库统一补齐；旧附件无 layout，识别时回退纯文本抽取）
    conn.execute_batch(SCHEMA_LAYOUT)?;
    // v6：疑似重复索引 + 期间锁表（幂等）；v0.9 已发布、用户已有数据，只增不改
    conn.execute_batch(SCHEMA_V6)?;
    // v7：疑似重复处置记录表（幂等，v0.16 新增；同上，只增不改）
    conn.execute_batch(SCHEMA_V7)?;
    // v6：export_batches 补 format 列（ALTER 不幂等，先查列存在）
    let has_format: bool = {
        let mut stmt = conn.prepare("PRAGMA table_info(export_batches)")?;
        let mut rows = stmt.query([])?;
        let mut found = false;
        while let Some(r) = rows.next()? {
            if r.get::<_, String>(1)? == "format" {
                found = true;
                break;
            }
        }
        found
    };
    if !has_format {
        conn.execute_batch(
            "ALTER TABLE export_batches ADD COLUMN format TEXT NOT NULL DEFAULT 'pdf'",
        )?;
    }
    Ok(())
}
