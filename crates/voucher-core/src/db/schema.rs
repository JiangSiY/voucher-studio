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

/// v8 附加索引（v0.18 仪表盘批）：只增索引、不改表结构。
///
/// 三条都对应"新指标卡片"的查询路径，此前的列上都**没有**可用索引：
/// - `attachments(ocr_status)`：待识别计数（`ocr_status IN ('pending','failed')`）
/// - `vouchers(status, updated_at)`：陈旧草稿（`status='draft' AND updated_at < ?`）
/// - `export_items(voucher_id)`：导出覆盖度反连接（"从未导出"、每张最近导出时间）
///
/// 幂等 `CREATE INDEX IF NOT EXISTS`，对既有库无损（与 v5/v6 同一模式，不清库）。
/// **不加** `vouchers(amount_cents)` 独占索引：千级数据量下金额区间筛选的全表扫可接受，
/// 先观察真实数据量的代价再决定（挂账）。
pub const SCHEMA_V8: &str = r#"
CREATE INDEX IF NOT EXISTS idx_attachments_ocr_status ON attachments(ocr_status);
CREATE INDEX IF NOT EXISTS idx_vouchers_status_updated ON vouchers(status, updated_at);
CREATE INDEX IF NOT EXISTS idx_export_items_voucher ON export_items(voucher_id);
"#;

/// v9 附加表（v0.22 关联批）：凭证之间的关系、事项组、关联审计。
///
/// 三条纪律（见 `docs/方案-v2-打印日报与凭证关联（定稿）.md`）：
/// - **幂等只增**：`CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS`，
///   对既有库无损；**不动 `SCHEMA_VERSION`**（bump 会触发 v<4 的清库分支，把用户数据清空）；
/// - `voucher_links` / `voucher_group_items` 都带 `ON DELETE CASCADE`——关联随凭证消亡，
///   删凭证不留孤儿（与 `attachments` / `export_items` 同一套语义）；
/// - `voucher_link_events` **刻意不加外键**：它是审计日志，"关联过"这件事必须独立于凭证生命周期
///   存活（与 `dup_decisions` 同理，也是《会计档案管理办法》第八条(三)"建立检索关系"的要求）。
pub const SCHEMA_V9: &str = r#"
CREATE TABLE IF NOT EXISTS voucher_links (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    a_id       INTEGER NOT NULL REFERENCES vouchers(id) ON DELETE CASCADE,
    b_id       INTEGER NOT NULL REFERENCES vouchers(id) ON DELETE CASCADE,
    kind       TEXT    NOT NULL CHECK (kind IN ('explains','detail')),
    note       TEXT    NOT NULL DEFAULT '',
    created_at TEXT    NOT NULL,
    CHECK (a_id <> b_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_voucher_links_pair ON voucher_links(a_id, b_id, kind);
CREATE INDEX IF NOT EXISTS idx_voucher_links_a ON voucher_links(a_id);
CREATE INDEX IF NOT EXISTS idx_voucher_links_b ON voucher_links(b_id);

CREATE TABLE IF NOT EXISTS voucher_groups (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    title      TEXT    NOT NULL,
    note       TEXT    NOT NULL DEFAULT '',
    created_at TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS voucher_group_items (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id   INTEGER NOT NULL REFERENCES voucher_groups(id) ON DELETE CASCADE,
    voucher_id INTEGER NOT NULL REFERENCES vouchers(id) ON DELETE CASCADE,
    sort_order INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_voucher_group_items_uniq
    ON voucher_group_items(group_id, voucher_id);
CREATE INDEX IF NOT EXISTS idx_voucher_group_items_voucher
    ON voucher_group_items(voucher_id);

CREATE TABLE IF NOT EXISTS voucher_link_events (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    link_id INTEGER NOT NULL,
    a_id    INTEGER NOT NULL,
    b_id    INTEGER NOT NULL,
    kind    TEXT    NOT NULL,
    action  TEXT    NOT NULL CHECK (action IN ('linked','unlinked')),
    note    TEXT    NOT NULL DEFAULT '',
    at      TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_voucher_link_events_link ON voucher_link_events(link_id);
CREATE INDEX IF NOT EXISTS idx_voucher_link_events_at ON voucher_link_events(at DESC);
"#;

/// v10 附加表（v0.22.5 组审计）：事项组的建组 / 加成员 / 移出成员 / 删组留痕。
///
/// 为什么单独一张表而不是复用 `voucher_link_events`：那张表的 `link_id / a_id / b_id`
/// 是"两两关系"的形状，事项组是"一个容器 + 成员行"，塞进去三列都会失去意义。
///
/// 它是**审计日志**，所以同样**刻意不加外键**：删组、删凭证都不该把"曾经这样关联过"
/// 抹掉（与 `voucher_link_events` / `dup_decisions` 同一套语义）。
/// 幂等只增，**不动 `SCHEMA_VERSION`**（bump 会触发清库分支）。
pub const SCHEMA_V10: &str = r#"
CREATE TABLE IF NOT EXISTS voucher_group_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id   INTEGER NOT NULL,
    action     TEXT    NOT NULL
               CHECK (action IN ('created','member_added','member_removed','deleted')),
    voucher_id INTEGER,
    note       TEXT    NOT NULL DEFAULT '',
    at         TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_voucher_group_events_group
    ON voucher_group_events(group_id);
CREATE INDEX IF NOT EXISTS idx_voucher_group_events_at
    ON voucher_group_events(at DESC);
"#;

/// v11 附加结构（v0.23 OCR 换代）：**云端结构化要素缓存**。
///
/// 为什么单独建表而不塞进 `ocr_cache`：`ocr_cache` 的键只有图片内容哈希，
/// 存的是「文本」；云端一次返回的是**结构化要素**（日期/金额/对方/商家 + 置信度），
/// 与文本是两种产物、失效条件也不同（换模型/换引擎要各自失效）。
/// 幂等只增，不动 `SCHEMA_VERSION`（bump 会触发清库分支）。
pub const SCHEMA_V11: &str = r#"
CREATE TABLE IF NOT EXISTS ocr_field_cache (
    cache_key   TEXT PRIMARY KEY,
    engine      TEXT NOT NULL,
    model       TEXT NOT NULL,
    fields_json TEXT NOT NULL,
    created_at  TEXT NOT NULL
);
"#;

/// v13 附加结构（台账导入批次）：**每次导入落一条批次**，撤销按批进行。
///
/// 为什么单独建表而不只靠 `vouchers.import_batch_id`：批次要能留住"这次导的是哪个文件、
/// 新建多少张、命中多少张、坏行多少行"——这些是凭证数据本身没有的信息，撤销与回看都靠它。
/// 撤销**不物理删批次行**（只写 `undone_at`），因为"这批曾经存在、后来被撤"本身是审计事实
/// （与 `voucher_link_events` / `dup_decisions` 同一套语义）。
/// 幂等只增，不动 `SCHEMA_VERSION`（bump 会触发清库分支）。
pub const SCHEMA_V13: &str = r#"
CREATE TABLE IF NOT EXISTS import_batches (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at  TEXT    NOT NULL,
    kind        TEXT    NOT NULL CHECK (kind IN ('shoukuan','fukuan','jiti')),
    file_name   TEXT    NOT NULL DEFAULT '',
    file_path   TEXT    NOT NULL DEFAULT '',
    rows_total  INTEGER NOT NULL DEFAULT 0,
    created     INTEGER NOT NULL DEFAULT 0,
    matched     INTEGER NOT NULL DEFAULT 0,
    skipped     INTEGER NOT NULL DEFAULT 0,
    error_rows  INTEGER NOT NULL DEFAULT 0,
    undone_at   TEXT
);
CREATE INDEX IF NOT EXISTS idx_import_batches_created
    ON import_batches(created_at DESC);
"#;

/// v14 附加表（Center 字段对齐，v0.26）：把「原始凭证档案」扩成能与 WPS「项目管理台账 V5」
/// 对上的业务对象——单据类型/分类/业务域/收支方向/票据信息/项目与对方主数据键/归档名/金蝶回写位。
///
/// 为什么是 1:1 扩展表而不是给 `vouchers` 加 20 列：`vouchers` 的行映射（`voucher_repo::COLS`
/// 与 `row_to_voucher`）被列表/搜索/重复核查/报表十几处共用，加列要同步改全部构造点与用例；
/// 扩展表把「台账对齐字段」集中一处，老库零风险，读取时按需 LEFT JOIN。
/// 删凭证/删附件由 `ON DELETE CASCADE` 带走扩展行，不留孤儿。
/// 纪律照旧：**幂等只增、不动 `SCHEMA_VERSION`**（bump 会触发 v<4 的清库分支）。
pub const SCHEMA_V14: &str = r#"
CREATE TABLE IF NOT EXISTS voucher_ext (
    voucher_id       INTEGER PRIMARY KEY REFERENCES vouchers(id) ON DELETE CASCADE,
    doc_type         TEXT    NOT NULL DEFAULT '',
    category         TEXT    NOT NULL DEFAULT '',
    biz_domain       TEXT    NOT NULL DEFAULT '',
    flow_dir         TEXT    NOT NULL DEFAULT '',
    ticket_type      TEXT    NOT NULL DEFAULT '',
    invoice_no       TEXT    NOT NULL DEFAULT '',
    invoice_date     TEXT,
    project_key      TEXT    NOT NULL DEFAULT '',
    project_name     TEXT    NOT NULL DEFAULT '',
    counterparty_key TEXT    NOT NULL DEFAULT '',
    material_name    TEXT    NOT NULL DEFAULT '',
    material_qty     REAL,
    material_unit    TEXT    NOT NULL DEFAULT '',
    archive_name     TEXT    NOT NULL DEFAULT '',
    source_doc_ref   TEXT    NOT NULL DEFAULT '',
    gl_voucher_no    TEXT    NOT NULL DEFAULT '',
    gl_batch         TEXT    NOT NULL DEFAULT '',
    gl_voucher_date  TEXT,
    gl_booked_at     TEXT,
    deleted_at       TEXT,
    updated_at       TEXT    NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_voucher_ext_doc_type ON voucher_ext(doc_type);
CREATE INDEX IF NOT EXISTS idx_voucher_ext_category ON voucher_ext(category);
CREATE INDEX IF NOT EXISTS idx_voucher_ext_project ON voucher_ext(project_key);
CREATE INDEX IF NOT EXISTS idx_voucher_ext_counterparty ON voucher_ext(counterparty_key);
CREATE INDEX IF NOT EXISTS idx_voucher_ext_source ON voucher_ext(source_doc_ref);
CREATE UNIQUE INDEX IF NOT EXISTS idx_voucher_ext_archive
    ON voucher_ext(archive_name) WHERE archive_name <> '';

CREATE TABLE IF NOT EXISTS attachment_ext (
    attachment_id      INTEGER PRIMARY KEY REFERENCES attachments(id) ON DELETE CASCADE,
    original_file_name TEXT    NOT NULL DEFAULT '',
    archive_name       TEXT    NOT NULL DEFAULT '',
    role               TEXT    NOT NULL DEFAULT 'primary'
                       CHECK (role IN ('primary','contract','invoice','receipt')),
    sha256             TEXT    NOT NULL DEFAULT '',
    deleted_at         TEXT
);
CREATE INDEX IF NOT EXISTS idx_attachment_ext_role ON attachment_ext(role);
"#;

/// v15 附加表（Center 会计投影 + 服务端认证，v0.26）：
/// 科目映射、账户映射、会计期间、分录、V5 回写记录、单用户认证、后台任务。
///
/// 为什么放同一段：它们同属「服务端接手桌面版之后」的新域——分录是台账与金蝶之间的桥，
/// 认证与任务队列是 Web 服务能跑起来的前提。幂等只增，不动 `SCHEMA_VERSION`。
pub const SCHEMA_V15: &str = r#"
CREATE TABLE IF NOT EXISTS acct_category_maps (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    category     TEXT    NOT NULL UNIQUE,
    account_code TEXT    NOT NULL,
    account_name TEXT    NOT NULL,
    direction    TEXT    NOT NULL CHECK (direction IN ('借','贷')),
    biz_domain   TEXT    NOT NULL DEFAULT '',
    note         TEXT    NOT NULL DEFAULT '',
    updated_at   TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS acct_account_maps (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    account_key  TEXT    NOT NULL UNIQUE,
    account_code TEXT    NOT NULL,
    account_name TEXT    NOT NULL,
    note         TEXT    NOT NULL DEFAULT '',
    updated_at   TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS gl_periods (
    period     TEXT    PRIMARY KEY,
    start_date TEXT    NOT NULL,
    end_date   TEXT    NOT NULL,
    status     TEXT    NOT NULL DEFAULT 'open' CHECK (status IN ('open','closed')),
    note       TEXT    NOT NULL DEFAULT '',
    updated_at TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS gl_entries (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    period       TEXT    NOT NULL,
    entry_date   TEXT    NOT NULL,
    voucher_no   TEXT    NOT NULL DEFAULT '',
    summary      TEXT    NOT NULL DEFAULT '',
    account_code TEXT    NOT NULL,
    account_name TEXT    NOT NULL DEFAULT '',
    debit_cents  INTEGER NOT NULL DEFAULT 0,
    credit_cents INTEGER NOT NULL DEFAULT 0,
    project_key  TEXT    NOT NULL DEFAULT '',
    source_type  TEXT    NOT NULL DEFAULT '',
    source_ref   TEXT    NOT NULL DEFAULT '',
    status       TEXT    NOT NULL DEFAULT 'draft'
                 CHECK (status IN ('draft','exported','booked')),
    note         TEXT    NOT NULL DEFAULT '',
    created_at   TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_gl_entries_period ON gl_entries(period);
CREATE INDEX IF NOT EXISTS idx_gl_entries_source ON gl_entries(source_type, source_ref);

CREATE TABLE IF NOT EXISTS v5_pushes (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    kind         TEXT    NOT NULL,
    target       TEXT    NOT NULL DEFAULT '',
    payload_json TEXT    NOT NULL DEFAULT '{}',
    status       TEXT    NOT NULL DEFAULT 'pending'
                 CHECK (status IN ('pending','done','failed')),
    error        TEXT    NOT NULL DEFAULT '',
    created_at   TEXT    NOT NULL,
    pushed_at    TEXT
);
CREATE INDEX IF NOT EXISTS idx_v5_pushes_kind ON v5_pushes(kind, created_at DESC);

CREATE TABLE IF NOT EXISTS auth_users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT    NOT NULL UNIQUE,
    password_hash TEXT    NOT NULL,
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS auth_sessions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    token_hash  TEXT    NOT NULL UNIQUE,
    user_id     INTEGER NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
    created_at  TEXT    NOT NULL,
    expires_at  TEXT    NOT NULL,
    last_seen_at TEXT   NOT NULL,
    user_agent  TEXT    NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_auth_sessions_expires ON auth_sessions(expires_at);

CREATE TABLE IF NOT EXISTS jobs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT    NOT NULL,
    status      TEXT    NOT NULL DEFAULT 'queued'
                CHECK (status IN ('queued','running','done','failed')),
    progress    INTEGER NOT NULL DEFAULT 0,
    message     TEXT    NOT NULL DEFAULT '',
    result_json TEXT    NOT NULL DEFAULT '{}',
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status, id DESC);
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
    // v8：仪表盘新指标所需的三条索引（幂等，v0.18 新增；只增索引、不改表结构）
    conn.execute_batch(SCHEMA_V8)?;
    // v9：凭证关联 / 事项组 / 关联审计（幂等只增，v0.22 关联批；不动 SCHEMA_VERSION）
    conn.execute_batch(SCHEMA_V9)?;
    // v10：事项组审计（幂等只增，v0.22.5；同上不动 SCHEMA_VERSION）
    conn.execute_batch(SCHEMA_V10)?;
    // v11：云端结构化要素缓存（幂等只增，v0.23 OCR 换代；同上不动 SCHEMA_VERSION）
    conn.execute_batch(SCHEMA_V11)?;
    // v11：ocr_layouts 补 engine 列——版面由哪个引擎产出必须可辨，否则
    // 「云端识别成功、库里却留着上一次本地版面」时几何配对会吃掉云端结果（要素来源混淆）。
    if !has_column(conn, "ocr_layouts", "engine")? {
        conn.execute_batch(
            "ALTER TABLE ocr_layouts ADD COLUMN engine TEXT NOT NULL DEFAULT 'local'",
        )?;
    }
    // v6：export_batches 补 format 列（ALTER 不幂等，先查列存在）
    if !has_column(conn, "export_batches", "format")? {
        conn.execute_batch(
            "ALTER TABLE export_batches ADD COLUMN format TEXT NOT NULL DEFAULT 'pdf'",
        )?;
    }
    // v12（财务工作流集成）：vouchers 加 ledger_ref 列 + 唯一索引。
    // ledger_ref = 台账流水号/单据号，是台账导入的幂等键（同一笔台账只对应一张凭证）。
    // 幂等只增，不动 `SCHEMA_VERSION`；SQLite 的 `ALTER TABLE ... ADD COLUMN` 不幂等，
    // 故一律先 `has_column` 探测再加。
    if !has_column(conn, "vouchers", "ledger_ref")? {
        conn.execute_batch("ALTER TABLE vouchers ADD COLUMN ledger_ref TEXT")?;
        conn.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_vouchers_ledger_ref ON vouchers(ledger_ref) WHERE ledger_ref IS NOT NULL",
        )?;
    }
    // v12：attachments 加 is_pdf、page_count 列
    if !has_column(conn, "attachments", "is_pdf")? {
        conn.execute_batch("ALTER TABLE attachments ADD COLUMN is_pdf BOOLEAN NOT NULL DEFAULT 0")?;
    }
    if !has_column(conn, "attachments", "page_count")? {
        conn.execute_batch("ALTER TABLE attachments ADD COLUMN page_count INTEGER")?;
    }
    // v13（台账批次）：批次表 + 凭证上的批次指针与「已入账」标记。
    // 批次是撤销的单位（只撤未补附件、未导出的凭证），「已入账」是人工勾的标记——
    // 金蝶凭证号暂不入库（挂账，见 docs/开发日志.md 第七十五笔），故先用时间戳表达"人已确认入账"。
    conn.execute_batch(SCHEMA_V13)?;
    if !has_column(conn, "vouchers", "import_batch_id")? {
        conn.execute_batch("ALTER TABLE vouchers ADD COLUMN import_batch_id INTEGER")?;
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_vouchers_import_batch ON vouchers(import_batch_id)",
        )?;
    }
    if !has_column(conn, "vouchers", "ledger_booked_at")? {
        conn.execute_batch("ALTER TABLE vouchers ADD COLUMN ledger_booked_at TEXT")?;
    }
    // v0.27 工作台重构：条目是否必须有凭证图片。
    // 新条目一律 1（建条目/导入都要求凭证）；迁移把**已有无图条目**回填 0，
    // 保留说明 / 备忘 / 分割单，不因「凭证必填」一刀切被全面阻断。
    if !has_column(conn, "vouchers", "attachment_required")? {
        conn.execute_batch(
            "ALTER TABLE vouchers ADD COLUMN attachment_required INTEGER NOT NULL DEFAULT 0",
        )?;
        conn.execute_batch(
            "UPDATE vouchers SET attachment_required = CASE
                WHEN EXISTS (SELECT 1 FROM attachments a WHERE a.voucher_id = vouchers.id) THEN 1
                ELSE 0 END",
        )?;
    }
    // v14（Center 字段对齐）：台账对齐字段的 1:1 扩展表 + 附件扩展（原名/归档名/角色/校验和）。
    conn.execute_batch(SCHEMA_V14)?;
    // v14 后续补的列：账户（结算方式）。V5 改版后「账户」装的是 银行转账/微信/支付宝/…，
    // 会计投影按「日期+账户」分组要用到它，所以不再是标签而是实列。
    if !has_column(conn, "voucher_ext", "account")? {
        conn.execute_batch("ALTER TABLE voucher_ext ADD COLUMN account TEXT NOT NULL DEFAULT ''")?;
    }
    // v14 后续补的列：电表编号 / 电费月度。V5「6 电费」的附件名是
    // `{日期yyMMdd}-{电表}-抄{电费月度yyMM}-{金额}`，两者都是公式输入，故单独成列。
    if !has_column(conn, "voucher_ext", "meter_no")? {
        conn.execute_batch("ALTER TABLE voucher_ext ADD COLUMN meter_no TEXT NOT NULL DEFAULT ''")?;
    }
    if !has_column(conn, "voucher_ext", "meter_month")? {
        conn.execute_batch(
            "ALTER TABLE voucher_ext ADD COLUMN meter_month TEXT NOT NULL DEFAULT ''",
        )?;
    }
    // v15（Center 会计投影 + 服务端认证）
    conn.execute_batch(SCHEMA_V15)?;
    Ok(())
}

/// 列是否存在（`ALTER TABLE ... ADD COLUMN` 不幂等，加列前先查 `PRAGMA table_info`）
fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        if r.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}
