# W1-B1 本地记录索引（只读）证据

- 日期：2026-11-19
- 范围：B1，仅 schema 追加 + `services/wps_index.rs` + 索引测试
- 不含：`wps_pull`、`wps_09`、分页解析、W2 写入、真实 HTTP
- 工作树：未提交（`wps.rs`、`schema.rs`、`services/mod.rs` 修改；`wps_index.rs` 新增）

## 1. 访问器路径核对

`Database` 的实际接口：

```rust
pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T>
pub fn with_tx<T>(&self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T>
```

- `Database` 内部为 `Mutex<Connection>`，**没有**返回 `&Connection` 的访问器，也没有 `unchecked_transaction()`；
- 直接返回 `&Connection` 需要持有 guard，无法安全提供，因此**未新增 `WpsStore::conn()`**，也未改 `db` 字段可见性；
- 在 `WpsStore` 加薄包装：

```rust
// crates/voucher-core/src/services/wps.rs:714
pub(crate) fn database(&self) -> &Database {
    self.db
}
```

`ensure_schema` 改用 `self.database().with_conn(...)`；`wps_index.rs` 的全部读写均走 `with_conn`/`with_tx`。

## 2. `ensure_schema` 核对

- `WPS_TABLES` 从 `[&str; 11]` 改为 `[&str; 12]`，加入 `wps_record_index`；
- `ensure_schema` 遍历 `WPS_TABLES`（不是硬编码 11），逐项检查 `sqlite_master`，缺表返回 `WPS_E_SCHEMA`；
- 版本自检仍为 `wps_state.schema_version='wps-1'`；
- `S0` 门禁未签字，真实 HTTP 未启用；本次不触碰 `wps.transport` 默认值（`Mock`）。

## 3. Schema 追加

`crates/voucher-core/src/db/schema.rs` 的 `SCHEMA_WPS` 在 `wps_idempotency_keys` 之后追加：

```sql
CREATE TABLE IF NOT EXISTS wps_record_index (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    sheet_id           INTEGER NOT NULL,
    record_id          TEXT    NOT NULL,
    record_type        TEXT    NOT NULL,
    business_key       TEXT    NOT NULL,
    business_key_kind  TEXT    NOT NULL,
    remote_modified_at TEXT,
    fields_json        TEXT    NOT NULL,
    pulled_at          TEXT    NOT NULL,
    UNIQUE(sheet_id, record_id)
);
CREATE INDEX IF NOT EXISTS idx_wps_index_key
    ON wps_record_index(sheet_id, business_key);
CREATE INDEX IF NOT EXISTS idx_wps_index_type
    ON wps_record_index(sheet_id, record_type);
```

- `SCHEMA_VERSION` 保持 4，不 DROP/ALTER 已有表、不清库；
- `WPS_TABLES` 11 → 12。

## 4. 接口

`crates/voucher-core/src/services/wps_index.rs`：

```rust
pub struct WpsIndexRow { /* sheet_id, record_id, record_type, business_key,
                             business_key_kind, remote_modified_at, fields_json, pulled_at */ }

impl WpsIndexRow {
    pub fn new(...) -> Self;
}

impl<'a> WpsStore<'a> {
    pub(crate) fn begin_scan(&self, sheet_id: i64, view_id: Option<&str>) -> Result<i64>;
    pub(crate) fn finish_scan(&self, scan_id: i64, pages: usize, complete: bool, error: Option<&str>) -> Result<()>;
    pub(crate) fn upsert_index_rows(&self, rows: &[WpsIndexRow]) -> Result<()>;
    pub(crate) fn find_index_by_business_key(&self, sheet_id: i64, key: &str) -> Result<Vec<WpsIndexRow>>;
    pub(crate) fn find_index_by_record_id(&self, sheet_id: i64, record_id: &str) -> Result<Option<WpsIndexRow>>;
    pub(crate) fn count_index(&self, sheet_id: i64) -> Result<usize>;
}

pub fn business_key_for(sheet_id: i64, fields: &Value) -> (String, &'static str);
```

- `business_key_for`：09 为 `电表 Link 首 id + \u{1f} + 期间`；02/03/04/05/06/07/08/10/11 取对应单号；空/缺失返回 `("", "none")`，调用方不得用空键查询；
- 字段名依据 `财务工作流/设计/项目台账_v2_设计.md`：02 项目「项目编号」L92、03 应收「应收单号」L119、04 应付「应付单号」L137、05 结算调整「调整编号」L159、06 资金流水「流水号」L174、07 发票「发票号」L192、08 电表「表号」L197、10 物料「SKU」L205、11 出入库「单号」L210；均为对应表主字段，不是推断；
- Link 支持数组、`{linkedRecordIds}`、`{linkedRecordId}`、裸字符串；
- `upsert_index_rows` 使用 `UNIQUE(sheet_id, record_id)` UPSERT；同一业务键的重复记录保留多行（`find_index_by_business_key` 返回全部），与“WPS 允许重复单号/09 同表同期”一致；
- `finish_scan` 只在自然结束传 `complete=true`；失败保留 `error` 文本；
- **一致性防御（本批选择返回错误，不用 debug_assert）**：`complete=true` 且 `error` 非空时直接返回 `CoreError::Invalid("WPS_E_SCAN_INCONSISTENT: …")`，不写 `finished_at`；B3 的 09 门仍保留 `report.complete == false` 直接 `STALE` 的第二道检查。

## 5. 测试

```bash
cargo test -p voucher-core --lib services::wps_index
# running 7 tests; test result: ok. 7 passed; 0 failed; 220 filtered out

cargo test -p voucher-core --lib services::wps
# test result: ok. 30 passed; 0 failed; 197 filtered out   (23 旧 + 7 新)
```

新增 7 例：

| 用例 | 覆盖 |
|---|---|
| `begin_and_finish_scan_roundtrip` | scan 插入/结束、pages/complete/finished_at |
| `finish_scan_records_error_and_incomplete` | 失败 `complete=false` + error 文本；另断言 `complete=true && error` 被拒绝且不写 `finished_at` |
| `index_upsert_is_idempotent_and_updates_fields` | 同 record_id 二次 UPSERT 覆盖 fields，不增行 |
| `index_find_by_business_key_returns_duplicates` | 同业务键多行全部返回 |
| `index_09_composite_key_hits_only_same_period` | 09 同电表不同期间不串命中 |
| `empty_doc_no_is_marked_none_and_not_used_as_key` | 空单号标 `none`，不被真实键命中 |
| `business_key_for_reads_doc_no_and_meter_period` | 单号 trim、09 复合键、缺电表返回 none |

## 6. 构建与静态检查

```bash
cargo build -p voucher-core
# Finished `dev` profile

cargo check -p voucher-studio
# Finished

cargo fmt --all --check
# no output

cargo clippy -p voucher-core --all-targets -- -D warnings
# Finished
```

过程中修复：`wps_index` 测试内 `format!("B1A\u{1f}2026-11")` 触发 `clippy::useless_format`，已改为 `"…".to_string()`。

## 7. 门禁状态

| 项 | 状态 |
|---|---|
| G0 客户端工程契约 | 已提交（`c44972d`、`593d5ca`） |
| G0 真正阻断 | **不变**：filter 预检 A/B、09 创建前全表刷新 |
| S0 / G1 | 仍阻断；真实 HTTP 关闭 |
| B1 | 索引+扫描记录完成，仅本地/mock，未进入 W2 |
| 旧 `create_records` 单参 | `git grep` 零残留 |

## 8. 未覆盖/待补充

- 本次不含 `wps_pull` 分页解析，也不含 `wps_09` 创建前刷新门；
- `wps_record_index` 目前无生产调用（模块级 `#![allow(dead_code)]`），已登记挂账 #60：B2 接入 `wps_pull` 后改为具体项 allow 并逐步移除；
- `Database` 无 `&Connection` 访问器的情况已在 `W1-T3-阻塞.md`/本文件记录，后续若需要跨模块引用连接，统一走 `with_conn`/`with_tx`；
- 未跑全量 `cargo test`（已知 `backup::crypto` 长跑 #58），仅按任务跑相关过滤。
