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

- 分页响应缺 records 数组统一返回 **`WPS_E_PULL_MALFORMED`**（`WpsError::PullMalformed`）；`WPS_E_PULL_PAYLOAD` 已停用，代码/测试/证据不再出现。
- 达到页上限不是自然结束：`PullReport.complete=false`、`error_code=WPS_E_PULL_MAX_PAGES`；只有空页/`has_more=false`/`total` 到达/短页且非 `has_more=true` 才 `complete=true`。
- `WpsPullReport`（`PullReport` 别名）公开 `complete`、`skipped`、`error`、`error_code`；`complete=true` 时后两者为空。
- 09 创建前门 `Wps09CreateGuard::ensure_create_allowed` 固定顺序：key 校验（0 请求）→ `pull_sheet(9, "09", None)` → `complete=false` 或 `skipped>0` → `WPS_E_09_STALE`（文本带 `error`/`error_code`）→ 查索引 → `WPS_E_09_DUPLICATE` → `Ok`。全程不调用写入。
- 09 拒绝码：`WPS_E_09_KEY`、`WPS_E_09_STALE`、`WPS_E_09_DUPLICATE`。
