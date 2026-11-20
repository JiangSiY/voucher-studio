# W1-B3 09 创建前门（只读）证据

- 日期：2026-11-19
- 范围：B3，新增 `services/wps_09.rs` 的 `Wps09CreateGuard::ensure_create_allowed`
- 不含：WPS 写入、真实 HTTP、W4 回滚/冲突；S0/G1 未通过，`transport` 仍为 `Mock`。

## 1. 判定顺序

固定为：

1. `meter_record_id` / `period` 去空格后为空 → `WPS_E_09_KEY`，**0 次请求**（不 initialize、不 list）。
2. `pull_sheet(9, "09", None, 500)` 强制刷新整张 09 表。
3. `report.complete == false` 或 `report.skipped > 0` → `WPS_E_09_STALE`，错误文本带 `error` 与 `error_code`。
4. 查本地索引 `find_index_by_business_key(9, "{meter}\u{1f}{period}")`；有命中 → `WPS_E_09_DUPLICATE`。
5. 全部通过 → `Ok(())`。

全程不调用 `create_records` / `create_record`；测试同时断言 mock 的 action 只有 `dbsheet.list_records`。

## 2. 前置修正（B3 前）

| 项 | 处理 |
|---|---|
| 错误码二选一 | 统一为 `WPS_E_PULL_MALFORMED`；`WpsError::PullMalformed` 的 `code()` 即该值；`WPS_E_PULL_PAYLOAD` 已从代码/测试/证据/日志移除 |
| `max_pages` 语义 | 达到上限仍可能继续 → `complete=false` + `error_code=WPS_E_PULL_MAX_PAGES`；只有自然结束才 `complete=true`。新增 `pull_sheet_marks_complete_false_when_max_pages_reached` |
| `skipped` | `PullReport.skipped` 公开；09 门在 `skipped>0` 时直接 STALE，不查索引、不放行；新增坏记录 skipped 测试 |
| `error/error_code` | `PullReport` 新增 `error_code: Option<String>`；09 门错误文本包含 `complete/skipped/error/error_code` |
| B1 字段依据 | `business_key_for` 对 05/07/08/10 使用设计文档主字段，依据 `财务工作流/设计/项目台账_v2_设计.md` L159「调整编号」、L192「发票号」、L197「表号」、L205「SKU」；新增 `business_key_for_uses_documented_main_fields_for_05_07_08_10` 锁定 |

## 3. 接口

```rust
pub enum Wps09Error {
    Key(String),       // WPS_E_09_KEY
    Stale(String),     // WPS_E_09_STALE
    Duplicate(String), // WPS_E_09_DUPLICATE
}

impl Wps09Error { pub fn code(&self) -> &'static str; }

pub struct Wps09CreateGuard;
impl Wps09CreateGuard {
    pub fn ensure_create_allowed<T: WpsTransport>(
        client: &WpsClient<T>,
        store: &WpsStore<'_>,
        meter_record_id: &str,
        period: &str,
    ) -> Result<(), Wps09Error>;
}
```

## 4. 测试

命令：

```bash
cargo test -p voucher-core --lib services::wps_09
cargo test -p voucher-core --lib services::wps
cargo clippy -p voucher-core --all-targets -- -D warnings
cargo fmt --all --check
```

结果：

- `services::wps_09`：**7 passed / 0 failed**
  - `missing_key_rejects_without_any_request`：空电表/期间 → `WPS_E_09_KEY`，initialize 0、calls 空
  - `complete_empty_pull_allows_create_and_never_writes`：09 空表 → Ok，只有 `dbsheet.list_records`
  - `incomplete_refresh_rejects_stale_with_readable_error`：429 重试耗尽 → `WPS_E_09_STALE`，文本含 `error_code=WPS_E_RATE`
  - `skipped_record_in_complete_pull_rejects_stale`：complete=true + skipped=1 → STALE，不查索引
  - `duplicate_same_meter_period_rejects_duplicate`：同表同期 → `WPS_E_09_DUPLICATE`，无 create
  - `different_period_is_not_duplicate`：同表不同期 → 放行
  - `malformed_payload_is_stale_with_readable_error_code`：缺 records → STALE，文本含 `WPS_E_PULL_MALFORMED`
- `services::wps`（含 index/pull/guard）：**47 passed / 0 failed**
- clippy/fmt 通过。

## 5. 门禁状态

| 项 | 状态 |
|---|---|
| B3 判定顺序/错误码 | ✅ |
| 09 skipped/complete=false 拒绝 | ✅ |
| 无写请求 | ✅ 测试断言 action 仅 list_records |
| 真实 HTTP | ❌ 关闭（S0/G1 未通过） |
| W4 | ❌ 未进入 |

结论：B3 只读 09 门通过本地 mock 验收，停在 B3，不进入 B4。
