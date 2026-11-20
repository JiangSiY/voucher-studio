# WPS 幂等与重试定稿（v0.28 W1-T3）

- 日期：2026-11-19
- 范围：`transport=mock` 客户端底座；不含真实 HTTP 传输、不含真实 WPS 读取。
- 依据：v0.28 §4/§6/§7；第 3、5 项待决问题定稿（2026-11-19）。

## 一、配置契约

| 配置键 | 取值 | 默认 | 说明 |
|---|---|---|---|
| `wps.transport` | `mock` \| `http` | `mock` | `http` 仅在 S0 显式门禁通过后启用；门禁键名/载体（待补充） |
| `wps.read_retry.attempts` | `3` | `3` | 只读总尝试次数（首次 + 2 次重试） |
| `wps.read_retry.initial_ms` | `1000` | `1000` | 初始退避 |
| `wps.read_retry.multiplier` | `2` | `2` | 指数倍数 |
| `wps.read_retry.max_ms` | `30000` | `30000` | 退避上限 |
| `wps.read_retry.full_jitter` | `true` | `true` | 0–2s 抖动（实现为 0..base 的 full jitter） |
| `wps.write_retry.attempts` | `1` | `1` | 写入默认 0 次自动重试；仅受控测试证明服务端幂等后才允许 1 次重试 |
| `s0_approved` | `false` \| `true` | `false` | `transport=http` 时必须为 `true`；否则启动校验拒绝 |
| `write_max_attempts` | `1` | `1` | 写入总尝试次数；配置为 0 或大于 1（重试关闭时）拒绝 |
| `write_retry_enabled` | `false` \| `true` | `false` | 写入重试开关；当前必须为 `false`，未开放 |

> 已落地 `WpsConfig`（`crates/voucher-core/src/services/wps.rs`）：`transport`、`s0_approved`、`write_max_attempts`、`write_retry_enabled`，`WpsConfig::validate()` 在 `http && !s0_approved`、写入重试开启或次数矛盾时拒绝。**配置载体尚未接线**（待补充：从仓库配置/X 读取 `WpsConfig` 的入口与 S0 门禁键名）。

## 二、错误码

- `WPS_E_IDEMPOTENCY_MISSING`：创建类写入缺少 `idempotency_key`；**本地拒绝，0 次请求**。
- `WPS_E_IDEMPOTENCY_COUNT_MISMATCH`：记录数与幂等键数不一致；内部构造器防御，**本地拒绝，0 次请求**。
- `WPS_E_SESSION`：MCP 会话过期（HTTP 404）；只读路径重初始化一次，写入路径不自动重放。
- 既有 `WPS_E_AUTH` / `WPS_E_RATE` / `WPS_E_TIMEOUT` / `WPS_E_REMOTE` / `WPS_E_PROTOCOL` / `WPS_E_SCHEMA` 不变。

## 三、幂等契约

- 创建类：每条记录一个 UUID；`idempotency_key` 放在 `tools/call` 的 `arguments` 顶层。
- 更新类：可选，不强制；缺失不拒绝。
- 缺失（创建）：本地拒绝，返回 `WPS_E_IDEMPOTENCY_MISSING`，不发起 HTTP。
- **服务端幂等未验证**：WPS 是否识别并执行 `idempotency_key` 尚未验证；该字段当前仅作为客户端追溯与去重标识，不构成服务端幂等保证。
- 本地记录：`wps_idempotency_keys` 保存 `idempotency_key`、`run_id`、`record_type`、`local_status`（`pending` / `succeeded` / `unknown`）、`remote_record_id`、`created_at`；成功回填 `remote_record_id`。
- `pending` / `unknown` 不得自动重试写入；需人工确认。

## 四、写入重试

- 写入（create/update/batch）默认 **1 次尝试，0 次自动重试**。
- 超时/断连原样返回错误，由上层决策；不自动重放。
- 仅当后续受控测试证明服务端幂等生效且请求携带 key 时，写入最多开放 1 次重试；当前不开放。

## 五、客户端 API 落地

| 方法 | 行为 |
|---|---|
| `create_records(sheet_id, &[KeyedRecord])` | **新签名**；每条记录必须带 `record` + `idempotency_key`，逐条调用；旧 `create_records(records)` 已删除 |
| `create_record(sheet_id, record, idempotency_key)` | 单条创建；key 置于 `arguments` 顶层；写入不重试 |
| `WpsWriteSession::create_records(sheet_id, record_type, &[KeyedRecord])` | 逐条写入 + 本地幂等键状态；部分失败不回滚整批 |
| `update_records(sheet_id, records)` | 更新；不强制 key |
| `update_records_with_key(sheet_id, records, Option<&str>)` | 可选 key；无 key 不拒绝 |
| `get_schema` / `list_records` | 只读；429/5xx/超时按 3 次策略重试；400006 不重试；404 重初始化一次 |

### 方案 §4/§6 写入接口说明（破坏性变更）

- 旧接口 `create_records(sheet_id, records)` **已删除**，不保留 `#[deprecated]` 包装。
- 新接口以 `KeyedRecord` 为元素：`KeyedRecord { record, idempotency_key }`，每条记录必须携带 key，缺失无法通过类型构造。
- 调用方迁移：把 `&[Value]` 改为 `&[KeyedRecord::new(record, key)]`；需要逐条持久化状态时使用 `WpsWriteSession`。
- **破坏性变更**：仓库内旧签名的调用点已全部迁移（`grep` 零残留）；**无外部调用方**（crate 内公共 API 仅供本仓库使用，尚无下游 crate）。
- 写入路径**不调用 WPS 原生批量接口**；`build_create_args` 明确拒绝单次多条记录，`create_record` 每次只发 1 条。

## 六、边界与冲突

- **粒度冲突（已裁决）**：`idempotency_key` 在 `arguments` 顶层只能承载 1 个 key；写入路径统一逐条调用，单次 `tools/call` 只含 1 条记录；WPS 原生批量创建不用于写入。若后续确需批量，必须先验证批量 payload 中记录级 key 的可识别性，作为独立优化项。
- `wps.transport` 的 S0 门禁字段/载体仍未接线（待补充）；`WpsConfig::validate()` 已提供 `http && !s0_approved` 拒绝逻辑，但没有从仓库配置读取 `WpsConfig` 的入口。
- 真实 HTTP 传输、token 刷新仍未实现（W1-T3 真实传输阻塞项，见 `W1-T3-阻塞.md`）；404 重初始化已在 mock 客户端契约与测试中落地。

## 七、测试契约

| 覆盖点 | 用例 |
|---|---|
| 缺失 key 0 请求 | `create_without_key_rejects_locally_without_request` |
| 写入超时不自动重试 | `write_timeout_is_not_retried` |
| 只读 429/5xx 重试 3 次 | `read_retries_429_and_5xx_three_attempts` |
| update 无 key 不拒绝 | `update_without_key_is_allowed` |
| 批量每条记录独立 key | `batch_each_record_uses_distinct_keys` |
| 幂等键本地记录 | `idempotency_keys_roundtrip` |
| 传输模式契约 | `transport_mode_contract` |
| 长度不一致 0 请求 | `count_mismatch_rejects_locally_without_request` |
| 配置门禁 | `config_contract_blocks_http_without_s0_and_write_retry` |
| 404 重初始化一次 | `read_404_reinitializes_session_once` |
| 逐条写入 3 条全成功 + remote id | `writer_creates_each_record_and_persists_remote_ids` |
| 第 2 条超时部分失败 | `writer_partial_failure_stops_and_marks_unknown` |
| 空 key 0 请求 | `writer_rejects_empty_key_without_request` |
| 响应 id 提取 | `extract_remote_record_id_accepts_known_shapes` |
| 单条写入耗时基线 | `single_record_write_latency_baseline`（mock，100 次均值约 33µs） |
| 缺 key 编译失败 | doctest `compile_fail`（`create_records` 文档，1 例） |

命令：`cargo test -p voucher-core --lib services::wps` → **23 passed**；`cargo test -p voucher-core --doc` → **1 passed（compile_fail）**。
## 八、分页与 09 门错误码（B2/B3 统一）

- 分页响应缺 records 数组统一返回 **`WPS_E_PULL_MALFORMED`**（`WpsError::PullMalformed`）；`WPS_E_PULL_PAYLOAD` 已停用且**无残留**，代码/测试/证据不再出现。
- 达到页上限不是自然结束：`PullReport.complete=false`、`error_code=WPS_E_PULL_MAX_PAGES`；只有空页/`has_more=false`/`total` 到达/短页且非 `has_more=true` 才 `complete=true`。页上限错误码 **`WPS_E_PULL_MAX_PAGES`**。
- `WpsPullReport`（`PullReport` 别名）公开 `complete`、`skipped`、`error`、`error_code`；`complete=true` 时后两者为空。
- 09 创建前门 `Wps09CreateGuard::ensure_create_allowed` 固定顺序：key 校验（0 请求）→ `pull_sheet(9, "09", None)` → `complete=false` 或 `skipped>0` → `WPS_E_09_STALE`（文本带 `error`/`error_code`）→ 查索引 → `WPS_E_09_DUPLICATE` → `Ok`。全程不调用写入。
- 09 拒绝码：`WPS_E_09_KEY`、`WPS_E_09_STALE`、`WPS_E_09_DUPLICATE`（**09 三码**）。
- **WPS_E_DOC_\***（doc guard 新增，见 §十）：单号去重门错误码，`WPS_E_DOC_KEY` / `WPS_E_DOC_STALE` / `WPS_E_DOC_DUPLICATE`。错误码总口径：**已统一为 `WPS_E_PULL_MALFORMED`，无残留；页上限 `WPS_E_PULL_MAX_PAGES`；09 三码；新增 `WPS_E_DOC_*`**。

### 9.6 视图条款（2026-11-20 评审修正）

- doc guard / 09 门**不做视图依赖**：预检一律走「分页全表 + 本地索引」（A 路径），`view_id` 传 `None`。
- `view_id` 缩窄（固定范围如「11/1 后」）在 **S0/G1 未验证前降级为未验证优化项**，不作为 G0/G1 门禁依赖；**A 是唯一预检路径**。
- 不在应用内新建/修改任何视图（MCP 白名单无建/改视图工具，W0-12）；需要验证固定视图时只用**已存在的视图**做只读拉取。


## 九、B4 四点核对（2026-11-20）

### 9.1 09 刷新签名与 view_id

`pull_sheet` 的签名为：

```rust
pull_sheet(
    client: &WpsClient<T>,
    store: &WpsStore<'_>,
    sheet_id: i64,
    record_type: &str,
    view_id: Option<&str>,
    page_size: usize,
) -> Result<WpsPullReport>
```

09 门调用 `pull_sheet(client, store, 9, "09", None, 500)`：第 3 个数据参数是 `sheet_id=9`，第 4 个是 `record_type="09"`，`view_id` 明确为 `None`，页大小 500。测试 `guard_full_refresh_has_no_view` 直接读取 `MockTransport` 收到的 `arguments`，断言 `sheet_id=9`、`offset=0`、`page_size=500`，且 `arguments.get("view_id").is_none()`。

### 9.2 期间格式

期间只接受 `YYYY-MM`（4 位数字 + `-` + 01–12）。`2026-13`、`2026/11`、`2026-1`、`2026-00`、`2026-1a`、前后带空格等均在本地返回 `WPS_E_09_KEY`，`initialize_calls=0`、`calls` 为空；`2026-01`、`2026-12` 放行。用例：`invalid_period_formats_reject_without_any_request`、`valid_period_formats_are_accepted`。

### 9.3 UPSERT 写库失败

`pull_sheet` 在索引发布/本地 DB 失败时先 `finish_scan(false, error)`，然后返回 **`Err`**（不是 `Ok(complete=false)`）；`Wps09CreateGuard` 把任何 `pull_sheet` 错误（含本地 DB 错误）统一映射为 **`WPS_E_09_STALE`**，文本带「09 刷新本地失败」，不会漏成裸 DB error。用例 `db_upsert_failure_maps_to_stale_not_bare_db_error` 通过删除 `wps_record_index` 表注入该路径。

### 9.4 剩余 `dead_code` allow

- `crates/voucher-core/src/services/wps_index.rs`：仅 `find_index_by_record_id` 保留 `#[allow(dead_code)] // W2 预检接入；B2 仅写索引`；接入 W2 按 record id 回查后移除。
- `count_index` 已改为 `#[cfg(test)]`，不再用 allow；`find_index_by_business_key` 的 allow 已移除（B3 09 门已使用）。
- `wps_pull.rs`、`wps_09.rs` 的模块级 `#![allow(dead_code)]` 已移除；B2/B3 已由 `pull_sheet`/`Wps09CreateGuard` 实际引用。

### 9.5 B4 验证原始结果

```text
cargo test -p voucher-core --lib services::wps
test result: ok. 51 passed; 0 failed; 0 ignored; 0 measured; 197 filtered out

cargo test -p voucher-core --lib services::wps_pull
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 239 filtered out

cargo test -p voucher-core --lib services::wps_09
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 237 filtered out

cargo test -p voucher-core --lib services::wps_index
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 240 filtered out

cargo check -p voucher-studio      # Finished
cargo clippy -p voucher-core --all-targets -- -D warnings  # Finished
cargo fmt --all --check            # 通过（无输出）
cargo test -p voucher-core --doc   # 1 passed（compile_fail）
```

说明：`wps_09` 由 7 增至 11（B4 补 4 例）；`wps_index` 为 8（B3 字段依据 1 例已包含，B4 未增删）；`services::wps` 相应由 47 增至 51。Rust 静态计数：voucher-core **414**（248 单元 + 166 集成），加壳 3 = **417**；前端仍 44 文件 228 用例。

## 十、doc guard（单号去重创建前门，2026-11-20 评审后实现）

### 10.1 职责与边界

- `WpsDocNoCreateGuard::ensure_create_allowed` 是 **02/03/04/06/05/07/08/10/11 等单号表**的创建前只读去重门：先 `pull_sheet(sheet, record_type, None)` 整表刷新，再查本地索引 `find_index_by_business_key`，命中同单号即拒绝。
- 判定顺序（与 09 门对称）：key 校验（0 请求）→ 整表刷新 → `complete=false` 或 `skipped>0` → `WPS_E_DOC_STALE` → 查索引 → `WPS_E_DOC_DUPLICATE` → `Ok`。全程不调用写入。
- **边界声明（评审修正）**：doc guard **仅检查单号重复**，**不检查主数据引用是否存在**（如 02 的客户 Link、03/04 的项目 Link）。主数据路径的处理（01/08/10 是否在应用内新建、阻塞并提示去 WPS 建、或增加新建）由审批问题 5 的修订单独处理，与 doc guard 解耦。
- **sheet_id 来源（评审修正）**：现有代码**没有** 02/03/04/06 的 sheet_id 常量或配置（09 门与 `wps_index` 均为硬编码 `9`/表号）。定来源为 `WpsDocNoSpec` 常量（见 10.2）+ `wps_pull`/`wps_index` 的参数贯穿；**不在 guard 内硬编码**，`WpsDocNoSpec` 即常量/配置承载点。

### 10.2 接口

```rust
pub struct WpsDocNoSpec { pub sheet_id: i64, pub record_type: &'static str }
pub const DOC_SPEC_02: WpsDocNoSpec = WpsDocNoSpec { sheet_id: 2, record_type: "02" };
pub const DOC_SPEC_03: WpsDocNoSpec = WpsDocNoSpec { sheet_id: 3, record_type: "03" };
pub const DOC_SPEC_04: WpsDocNoSpec = WpsDocNoSpec { sheet_id: 4, record_type: "04" };
pub const DOC_SPEC_06: WpsDocNoSpec = WpsDocNoSpec { sheet_id: 6, record_type: "06" };
// 05/07/08/10/11 同理（表号按业务键字段依据）

pub enum WpsDocNoError {
    Key(String),       // WPS_E_DOC_KEY
    Stale(String),     // WPS_E_DOC_STALE
    Duplicate(String), // WPS_E_DOC_DUPLICATE
}

pub struct WpsDocNoCreateGuard;
impl WpsDocNoCreateGuard {
    pub fn ensure_create_allowed<T: WpsTransport>(
        client: &WpsClient<T>,
        store: &WpsStore<'_>,
        spec: &WpsDocNoSpec,
        doc_no: &str,
    ) -> Result<(), WpsDocNoError>;
}
```

### 10.3 测试（与 09 门对称）

| 用例 | 覆盖 |
|---|---|
| `empty_doc_no_rejects_without_any_request` | 空单号 → `WPS_E_DOC_KEY`，initialize 0、calls 空 |
| `complete_empty_pull_allows_create_and_never_writes` | 空表 → Ok，只有 `dbsheet.list_records` |
| `incomplete_refresh_rejects_stale_with_readable_error` | 429 重试耗尽 → `WPS_E_DOC_STALE`，文本含 error_code |
| `skipped_record_in_complete_pull_rejects_stale` | complete=true + skipped=1 → STALE（**评审补**，与 09 对称） |
| `duplicate_same_doc_no_rejects_duplicate` | 同单号 → `WPS_E_DOC_DUPLICATE`，无写请求 |
| `different_doc_no_is_not_duplicate` | 不同单号 → 放行 |
| `guard_never_writes` | 全程 action 仅 `dbsheet.list_records` |

### 10.4 错误码口径（评审修正汇总）

- **已统一为 `WPS_E_PULL_MALFORMED`，无残留**（`WPS_E_PULL_PAYLOAD` 不再出现）；
- 页上限 **`WPS_E_PULL_MAX_PAGES`**；09 三码（`WPS_E_09_KEY` / `WPS_E_09_STALE` / `WPS_E_09_DUPLICATE`）；
- 新增 **`WPS_E_DOC_*`**（`WPS_E_DOC_KEY` / `WPS_E_DOC_STALE` / `WPS_E_DOC_DUPLICATE`）。

### 10.5 主数据路径（审批问题 5，待产品负责人确认）

- 01（往来对象）/08（电表）/10（物料）的主数据：**不**在应用内新建；创建前预检只覆盖单号表（doc guard），主数据引用缺失时阻塞并提示去 WPS 建，或由增加新建（备选）处理；
- 具体选「阻塞并提示」还是「增加新建」**待产品负责人确认**，已列入 `docs/评审修订清单-2026-11-20.md` 修订点；doc guard 本期只做单号重复检查，不扩展主数据创建。

## 11. W2 Mock 应用层链路（2026-11-20，本地）

`services/wps_w2.rs` 是 W1 只读组件的业务编排层，只在 `transport=Mock` 下运行；不接真实 HTTP、不操作生产文件。入口：

```rust
pub fn run_mock_batch<T: WpsTransport>(
    store: &WpsStore<'_>,
    transport: T,
    requests: &[W2CreateRequest],
) -> Result<W2BatchReport>;
```

固定 `W2Config::mock_write()`（显式测试配置，不读生产配置）；内部仍可拆成 `W2Sync::pull_all_history`、`preflight`、`execute`、`status`、`recover_link`。

### 11.1 顺序

1. `pull_all_history`：11 张表各一条 `pull_sheet(..., view_id=None, 500)` 全历史分页链；任一表 `complete=false` 或 `skipped>0` → `writes_blocked=true`。
2. `preflight`：入口权限（默认 PullOnly 拒绝写）、空批、01/08/10 只读、05/07 只查重、02/03/04/06/09/11 才可建、日期/期间分界、业务键非空、批内 `(sheet_id,key)` 重复、历史索引重复、幂等键已存在、必填引用和目标凭证存在。单号/09 的历史重复判定复用 `WpsDocNoCreateGuard::ensure_index_unique` / `Wps09CreateGuard::ensure_index_unique`，只查已发布索引，不重复刷新整表。
3. `execute`：预检失败整批 Rejected、0 请求；逐条先写 `wps_write_intents`（payload hash）再写 `wps_idempotency_keys`，然后 `create_record`；成功回填 `wps_links`；链接失败→`pending_backfill`；远端超时→`unknown`；停止后后续 `not_attempted`，不重试、不回滚。
4. `recover_link`：必须有远端记录 id + 归属证据；只补本地链，不再发创建请求。

### 11.2 日期与表范围

- 分界 `2026-11-01`（含）；旧记录仅历史查重/引用，不进入新建。
- 01/08/10 只引用 WPS 原记录；05/07 只读查重；02/03/04/06/09/11 才可进入写入候选。
- 日期支持 `YYYY-MM-DD`、`YYYY/MM/DD`、RFC3339（Asia/Shanghai）；09 仅 `YYYY-MM`；缺失/非法/歧义 → `NeedsReview`。
- 03 约定收款日、04 约定付款日、06 收款取实际到账日/付款取实际扣款日（字段映射待真实副本核验）。

### 11.3 索引发布

`wps_pull` 不再逐页写索引：页结果先 `staged` 累积；只有 `complete` 且无坏页、游标连续、结束信号一致时才调用 `wps_index::publish_scan`，在同一事务内 upsert 本轮索引并把 `wps_scans.complete=1`。失败/坏页/游标异常/空页假 `has_more`/达页上限均不发布，保留上一轮有效索引并阻断写入。

### 11.4 桌面开发入口

- `voucher-api::dev_w2::run` 提供进程内 W2 开发入口；`src-tauri/src/commands/dev_w2.rs` 暴露 `dev_w2_mock_sync`，`src/pages/DataPage.vue` 在 `import.meta.env.DEV && isTauri` 时显示「开发本地模拟同步」面板。
- 守卫：debug 构建 + `VOUCHER_W2_MOCK=1` + 非空 `VOUCHER_DATA_DIR` 且位于系统临时目录；命令参数只有 `action`（pull/write）与 `scenario`，不接受 file_id 或数据目录，无法把目标改成真实文件/生产库。
- 普通运行、生产目录、网页端均返回 `dev_only`；入口只走 `DevW2Transport` 合成数据，不读取 WPS 凭据、不发 HTTP。

### 11.5 已知边界

- 桌面入口仍是开发态 Mock，不是生产接线；`MockTransport` 仅在测试构建使用。
- `wps_write_intents` 已落库；`wps_links` 回填成功/失败均有测试，但真实副本 C1–C6 未执行。
- S0-01/02/03/05/09/10/11/12 仍依赖实现/目标环境/授权；本文件不替代 S0/G1 签字。
