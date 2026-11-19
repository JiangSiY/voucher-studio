# W1-T2：WpsClient 边界、MCP 会话 mock 与同步表骨架

- 日期：2026-11-19
- 范围：仅 mock/本地；未调用真实 WPS；未改 `SCHEMA_VERSION`（仍为 4）；未清库。
- 方案依据：v0.28 §7（MCP 会话/错误码）、§15 G1（S0 前仅只读/mock）、W1-T2。

## 一、三项评审结论

| 项 | 结论 | 理由 | 最终落地 |
|---|---|---|---|
| 1. `WpsClient` 类型化接口 | **采纳（最小 JSON 包装）** | W0 已实测动作名与参数；返回类型若定领域结构会固化未评审字段。先返回 `Value`，字段解析留 W1 映射层 | `get_schema(sheet_id)`、`list_records(sheet_id, view_id, offset, page_size)`、`create_records(sheet_id, &[KeyedRecord])`、`create_record(sheet_id, record, key)`、`update_records(sheet_id, records)`，均 `Result<Value, WpsError>`；旧 `create_records(sheet_id, records)` 已删除（破坏性变更） |
| 2. MCP 会话初始化 | **采纳（限定范围）**：本次做 mock 会话 + SSE 解析；重试/退避不做 | `initialize`/session 头/SSE 是方案 §7 已定义流程；重试时序涉及真实 HTTP，未实测，留 W1-T3 | `WpsTransport::{initialize, call(session_id, action, args)}`；`WpsClient::{initialize, session_id, call}`；`parse_mcp_payload` |
| 3. `CoreError::Wps` 载体 | **不采纳（本轮）** | 新增全局枚举变体会影响 API 错误映射与桌面壳；`WpsError` 已在同步层稳定表达，W2 接 API 时再评估 | 保留 `CoreError::Invalid("WPS_E_SCHEMA: …")`；同步层用 `WpsError::Schema` |

## 二、已实现

| 项 | 实现 |
|---|---|
| 传输层边界 | `WpsTransport::initialize` + `call(session_id, action, args)` |
| 类型化客户端 | `WpsClient::{initialize, session_id, get_schema, list_records, create_records, create_record, update_records, call}`；`WpsWriteSession` 负责逐条写入与本地状态 |
| 动作常量 | `dbsheet.get_schema` / `list_records` / `create_records` / `update_records` |
| 错误分类 | `WpsError::{Schema,AuthExpired,RateLimited,Timeout,Remote,SessionExpired,IdempotencyMissing,IdempotencyCountMismatch,Protocol}` → `WPS_E_*` |
| 400006 | `from_mcp_error({"code":400006}) → AuthExpired` |
| 429 | `from_http_status(429) → RateLimited` |
| 超时 | `WpsError::Timeout` |
| 运行期自检 | `WpsStore::ensure_schema()` 检查 11 表 + `wps_state.schema_version='wps-1'`；缺失/不符 → `WPS_E_SCHEMA` |
| 表骨架 | `state` 读写、`start_run`/`finish_run`、`count_links` |
| SSE | `parse_mcp_payload` 支持纯 JSON 与 `data:` 行 |
| 假 MCP | `MockTransport`：session id、响应队列、调用记录（session/action/args） |

## 三、未实现/待确认

1. **真实 HTTP 传输**：未实现 `initialize` 的 HTTP 头解析、`tools/call` 的 SSE 流式读取；当前只有 mock 与纯函数解析。
2. **重试/退避策略**：W1-T3 已实现只读重试（3 次/1s/2x/30s/full jitter）与写入 0 次重试；真实 HTTP 传输仍阻塞（见 `W1-T3-阻塞.md`）。
3. **`content[0].text` 二次解析**：当前 `parse_mcp_payload` 只解析外层 JSON/SSE；MCP `content[0].text` 内的业务 JSON 由后续映射层解析。
4. **`WPS_E_SCHEMA` 的 API 映射**：暂用 `CoreError::Invalid`，W2 接 API 时再决定是否新增变体。

## 四、测试

命令：`cargo test -p voucher-core --lib services::wps`

```
running 8 tests
test services::wps::tests::mock_transport_maps_400006_429_and_timeout ... ok
test services::wps::tests::uninitialized_call_returns_protocol_error ... ok
test services::wps::tests::typed_methods_wrap_actions_and_args ... ok
test services::wps::tests::parse_mcp_payload_accepts_plain_json_and_sse ... ok
test services::wps::tests::ensure_schema_ok_after_init ... ok
test services::wps::tests::state_roundtrip_and_run_lifecycle ... ok
test services::wps::tests::ensure_schema_missing_table_returns_wps_e_schema ... ok
test services::wps::tests::ensure_schema_version_mismatch_returns_wps_e_schema ... ok

test result: ok. 8 passed; 0 failed
```

静态检查：

- `cargo check -p voucher-core` → 通过
- `cargo clippy -p voucher-core --all-targets -- -D warnings` → 通过
- `cargo fmt --all --check` → 通过

## 五、自检

- 未改 `SCHEMA_VERSION`（4）；未清库；未改老表。
- 未调用真实 WPS；未写生产文件。
- 未提交代码（等 W1 小批收口统一写日志/提交）。
- 本次新增代码与三项评审结论一致；未把未评审的台账字段结构写进公共 API。


## 六、W1-T3 定稿落地（2026-11-19）

- 新增 `WpsRetryPolicy`：只读 3 次、1s 起、2x、30s 上限、full jitter；写入 `write_attempts=1`（0 次自动重试）。
- 新增 `WPS_E_IDEMPOTENCY_MISSING`；`WpsConfig` 提供 `transport`、`s0_approved`、`write_max_attempts`、`write_retry_enabled` 与启动校验。
- **破坏性变更**：旧 `create_records(sheet_id, records)` 已删除；新 `create_records(sheet_id, &[KeyedRecord])` 以 `KeyedRecord { record, idempotency_key }` 强制每条记录带 key，`create_record` 单条发送、顶层 `idempotency_key`。仓库内旧调用点已全部迁移，无外部调用方。
- 写入路径统一逐条调用；`build_create_args` 明确拒绝单次多条记录；`WpsWriteSession` 逐条写本地 `wps_idempotency_keys`，部分失败不回滚，后续记录 `NotAttempted`。
- 新增 `WPS_E_IDEMPOTENCY_COUNT_MISMATCH`、`WPS_E_SESSION`；只读 404 重初始化一次，400006 不重试。
- 新增 `wps_idempotency_keys` 表与 `WpsStore::{insert_idempotency_key, mark_idempotency_key, idempotency_key}`。
- 测试从 8 例增至 23 例：缺失 key 0 请求、计数不一致 0 请求、写入超时不重试、只读 429/5xx 三次、404 重初始化、update 无 key 放行、逐条独立 key、3 条逐条成功/第 2 条超时部分失败、配置门禁、幂等键往返、传输模式契约、响应 id 提取、单条写入耗时基线。
- 编译期负例：`create_records` 的 `compile_fail` doctest 固定“缺 key 调用无法编译”。

命令：`cargo test -p voucher-core --lib services::wps` → **23 passed; 0 failed**；`cargo test -p voucher-core --doc` → **1 passed（compile_fail）**。
