# W1-B2 分页全表拉取证据（v0.28）

日期：2026-11-19
范围：`voucher-core` 只读分页拉取（`services::wps_pull`），不接真实 HTTP、不含 09 创建门、不含写入，不进入 B3。

## 1. 交付内容

| 文件 | 变更 |
|---|---|
| `crates/voucher-core/src/services/wps_pull.rs` | 新增：`PullPage` / `PullReport` / `parse_list_page` / `pull_sheet` / 分页解析辅助 |
| `crates/voucher-core/src/services/wps_index.rs` | 移除模块级 `#![allow(dead_code)]`；`find_*` / `count_index` 改为具体项 allow（W2 预检接入）；删除未用 `TABLE` 常量 |
| `crates/voucher-core/src/services/mod.rs` | 注册 `pub mod wps_pull;` |

## 2. 接口

```rust
pub fn parse_list_page(
    sheet_id: i64,
    record_type: &str,
    payload: &Value,
) -> Result<PullPage, WpsError>;

pub fn pull_sheet<T: WpsTransport>(
    client: &WpsClient<T>,
    store: &WpsStore<'_>,
    sheet_id: i64,
    record_type: &str,
    view_id: Option<&str>,
    page_size: usize,
) -> Result<PullReport>;
```

- `page_size` 上限 `MAX_PAGE_SIZE = 500`；分页上限 `MAX_PULL_PAGES = 1_000`，防止 offset 异常导致无限循环。
- 支持 `records`、`data.records`、`data` 数组；`has_more`/`hasMore`、`total`/`total_count`、`next_offset`/`nextOffset` 两种命名。
- 坏记录（缺 id、`fields` 非对象、记录非对象）计入 `skipped`，不阻断整页；响应缺少 records 数组才返回 `WPS_E_PULL_MALFORMED`。
- 远端/解析/offset 不前进 → `finish_scan(complete=false, error=...)`，报告 `complete=false`，返回 `Ok(report)`；数据库错误才返回 `Err`。
- 正常终止条件：空页、`has_more=false`、`total` 到达、`has_more != true` 且短页、`MAX_PULL_PAGES`。
- 索引行 `pulled_at` 用 UTC RFC3339（秒精度）；`fields_json` 保存原字段对象，不丢字段。

## 3. 测试

命令：

```bash
cargo test -p voucher-core --lib services::wps_pull
cargo test -p voucher-core --lib services::wps
```

结果：

- `services::wps_pull`：**9 passed / 0 failed**
  - `parse_page_extracts_rows_and_page_metadata`
  - `parse_page_skips_bad_records_without_failing`
  - `parse_page_accepts_data_array_and_camel_case_has_more`
  - `parse_page_rejects_payload_without_records`
  - `pull_sheet_pages_until_short_page_and_marks_scan_complete`
  - `pull_sheet_marks_complete_false_when_max_pages_reached`
  - `pull_sheet_marks_scan_incomplete_on_offset_regression`
  - `pull_sheet_marks_scan_incomplete_on_remote_error`
  - `pull_sheet_marks_scan_incomplete_on_payload_error`
- `services::wps`（含 index/guard）：**47 passed / 0 failed**。
- `cargo clippy -p voucher-core --all-targets -- -D warnings`、`cargo fmt --all --check` 通过。

## 4. 覆盖与缺口

已覆盖：
- 正常分页到短页、scan `complete=1`、`finished_at` 写入；
- 远端 429（只读重试耗尽）→ scan 不完整 + 错误落库；
- payload 缺 records → scan 不完整；
- offset 回退 → 拒绝并标记不完整；
- 页上限 → `complete=false` + `WPS_E_PULL_MAX_PAGES`，不误标完成；
- 坏记录跳过、`data` 数组/camelCase 元数据、缺 records 报错。

未覆盖（后续阶段）：
- 真实 HTTP/MCP 传输（S0/G1 未通过，`transport` 仍默认 `Mock`）；
- 09 创建前全表刷新（B3 门，本批不做）；
- 视图 `view_id` 的有效性（W0 已验证存在性，B2 只透传）；
- ~~`upsert_index_rows` 数据库失败路径的注入测试~~（B4 已在 `wps09.md` 的 `db_upsert_failure_maps_to_stale_not_bare_db_error` 覆盖）；
- 50 万级/多页真实压测（W0 页预算：02/03/04 约 2 页、06 约 4 页、09 1 页、11 约 15 页）。

## 5. 门禁状态

| 项 | 状态 |
|---|---|
| B2 分页用例 | ✅ 9/9 |
| `services::wps` 回归 | ✅ 47/47 |
| clippy / fmt | ✅ |
| S0 / G1 | ❌ 仍阻断 |
| 真实 HTTP | ❌ 关闭 |
| filter 预检 A/B | ❌ 仍阻断（W0 结论：A + view 缩窄） |
| 09 创建前刷新 | ✅ B3 实现，B4 补无 view_id/期间格式/失败归一 |
| W2 写入 | ❌ 未进入 |

## 6. 交叉引用

- 09 门与 B4 四点：`docs/验收基线/证据/2026-11-19-W1/wps09.md`
- 索引/扫描记录：`docs/验收基线/证据/2026-11-19-W1/pull-index.md`
- 实施口径：`docs/实施说明-WPS幂等-v0.28.md` §八、§九

结论：B2 只读分页与索引写入路径通过本地 mock 验收；B3/B4 已在其上完成 09 门与核对；不接真实 HTTP，不碰写入。
