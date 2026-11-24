# doc guard 接入 W2 预检设计（v0.28，只出设计，不写代码）

- 日期：2026-11-20
- 状态：**设计文档；本轮不实现、不接真实 HTTP、不进入 W2 写入**
  - 2026-11-24 补（按 HEAD 代码校正）：`services/wps_create.rs` 已落地只读干跑 `WpsCreateFlow::dry_run`（复用 `W2Sync::preflight`）与三段式写入 `create_doc_records`/`create_meter_records`（`crates/voucher-core/src/services/wps_create.rs:107`、`:149`、`:205`）；干跑命令 `w2-create-dry-run` 已接线（`crates/voucher-cli/src/main.rs:1072`，仅离线路径），但**写入未接任何业务命令**，写入口仍只有 `W2Sync::execute`。干跑命令的契约与边界见 §8。
- 依据：`crates/voucher-core/src/services/wps_doc_guard.rs`（W1 已实现）、`wps_09.rs`、`wps_pull.rs`、`wps_index.rs`、`wps.rs`（`WpsWriteSession` / `wps_links`）；实施说明 §八/§十
- 前置：W1 已收口；#64/#65/#66 截止 W2 启动前；S0/G1 未通过，`transport` 仍 `Mock`

## 1. 创建入口与守卫调用位置

### 1.1 现状（无 W2 创建入口）

- 当前仓库**没有任何业务代码调用** `WpsDocNoCreateGuard` / `Wps09CreateGuard`（`grep -rn` 仅 `services/wps*` 内部；`voucher-api` 只有本地 `create_record_with_attachment` 与 WPS 无关）。
- WPS 写入能力已具备但未接线：`WpsClient::create_record`（逐条 + 幂等键）、`WpsWriteSession::create_records`（逐条写入 + 本地幂等键状态）、`wps_links` 表存在但**无写入方法**（`grep` 确认 `wps.rs` 无 `INSERT INTO wps_links` 方法，只有 `count_links`）。
- 因此 W2 的「创建入口」是**新建**，不是改造现有：在本轮设计中定义入口形态，实现留到 W2。

### 1.2 入口形态：`WpsCreateFlow`（新建，建议）

```rust
// 建议新增 `services/wps_create.rs`（W2 实现），公开：
pub struct WpsCreateFlow<'a, T: WpsTransport> {
    client: &'a WpsClient<T>,
    store: &'a WpsStore<'a>,
    run_id: i64,
}

impl WpsCreateFlow<'_, T> {
    /// 预检 → 写入 → 回填 三段式；返回每条记录的结果。
    pub fn create_doc_records(
        &self,
        spec: &WpsDocNoSpec,
        records: &[WpsCreateInput],   // { fields, idempotency_key }
    ) -> Vec<WpsCreateOutcome>;       // { status, remote_record_id, error }
}
```

- 入口分层：`voucher-api` 命令层（`biz.rs` / 新 `wps.rs` 命令）→ `WpsCreateFlow` → 守卫（只读）→ `WpsWriteSession`。
- 守卫调用位置：**`WpsCreateFlow::create_doc_records` 内部、写入之前**；每条记录先过 `WpsDocNoCreateGuard::ensure_create_allowed`，全部通过才进入写入段。

### 1.3 与 `Wps09CreateGuard` 的关系：**并列，不串行**

- 09 门（`Wps09CreateGuard`）与 doc guard（`WpsDocNoCreateGuard`）**职责正交、互不依赖**：
  - 09 门管 **09 抄表表**（复合业务键 `电表 + 期间`，`sheet_id=9`，模式 `meter_period`）；
  - doc guard 管 **单号表**（02/03/04/06/05/07/08/10/11，单一业务键 `doc_no`，模式 `doc_no`）。
- 同一创建流程如需创建 09 **和**单号表记录（如「先建项目 02 → 建应收 03 → 建抄表 09」），二者**顺序执行**（各自独立预检 + 写入），**不嵌套、不串行依赖**——每个守卫只做自己表的检查，不调用另一个守卫。
- 理由：两守卫共用 `pull_sheet` 与 `find_index_by_business_key`（技术复用面见 §5），但业务判定完全独立；串行化反而引入「B 表预检依赖 A 表写入结果」的耦合，而 09 与单号表之间无此依赖。

### 1.4 三段式组织：「预检 → 写入 → 回填 wps_links」

```
阶段 1  预检（只读）      WpsDocNoCreateGuard::ensure_create_allowed
                          → 整表刷新（pull_sheet）→ complete/skipped → STALE
                          → find_index_by_business_key → DUPLICATE → Ok
                          所有记录预检通过才进入阶段 2（全有或全不进）

阶段 2  写入（逐条）      WpsWriteSession::create_records（已有）
                          逐条 create_record + 独立 idempotency_key；
                          禁止调用 WPS 原生批量接口（冲突 2 裁决，见实施说明 §六）
                          部分失败不回滚整批（现状语义）

阶段 3  回填 wps_links    （W2 新增：wps_links 写入方法）
                          每条成功记录 → INSERT wps_links(
                              sheet_id, record_id=remote_record_id, doc_type=record_type,
                              origin='wps', idempotency_key, state='synced',
                              fields_json/base_json/remote_json=返回 payload, ...)
                          voucher_id 列留 NULL（本地凭证未关联时）；关联由后续认领流程处理
```

- **写入段粒度为逐条（写死）**：阶段 2 内部是**逐条 `create_record` 调用、每条携带独立 `idempotency_key`**；**禁止调用 WPS 原生批量接口**（`build_create_args` 已明确拒绝单次多条，实施说明 §六「粒度冲突」裁决：写入统一逐条，不用于批量）。W2 实现不得回退到批量。
- **阶段 1 只读（含本地索引副作用边界）**：预检阶段允许 `pull_sheet` 产生其固有的**本地索引副作用**（`wps_record_index` UPSERT、`wps_scans` 生命周期记录）——这是刷新机制的组成部分，**不是业务写入**；预检阶段**禁止任何业务表写入**（`wps_links`、幂等键表、墓碑、`voucher` 相关表）。本设计明确「预检只读」的判定标准是**不改任何业务表**，`pull_sheet` 的本地落盘不构成业务写入。
- **阶段 3 由写入路径负责**：回填不是预检的职责；只对 `Succeeded` 记录做。

## 2. 各 sheet 的 WpsDocNoSpec 与 business_key 来源

### 2.1 WpsDocNoSpec 已具备（W1 实现）

| sheet_id | record_type | business_key 字段 | 依据（项目台账_v2_设计.md） | 模式 |
|---|---|---|---|---|
| 2 | `"02"` | `项目编号` | L92 | `doc_no` |
| 3 | `"03"` | `应收单号` | L119 | `doc_no` |
| 4 | `"04"` | `应付单号` | L137 | `doc_no` |
| 5 | `"05"` | `调整编号` | L159 | `doc_no` |
| 6 | `"06"` | `流水号` | L174 | `doc_no` |
| 7 | `"07"` | `发票号` | L192 | `doc_no` |
| 8 | `"08"` | `表号` | L197 | `doc_no` |
| 10 | `"10"` | `SKU` | L205 | `doc_no` |
| 11 | `"11"` | `单号` | L210 | `doc_no` |
| 9 | `"09"` | `电表 Link 首 id + \u{1f} + 期间` | ——（wps_09 既定） | `meter_period` |

- **WpsDocNoSpec 只有 `sheet_id` + `record_type`**（W1 已实现）；`field_name` **不在** Spec 里——因为 `business_key_for(sheet_id, fields)`（`wps_index.rs`）已内含字段名映射，Spec 传 `sheet_id` 即得对应主字段。
- **W2 建议**：不在 Spec 增加 `field_name` 字段（避免与 `business_key_for` 的单一真源重复）；如需显式化，可在 Spec 加 `#[cfg(test)]` 断言锁定 `business_key_for(spec.sheet_id, …)` 行为（已有 `business_key_for_uses_documented_main_fields_for_05_07_08_10` 测试覆盖）。

### 2.2 business_key 来源（单一真源）

- 预检时 guard 不自己拼键；调用方把**待创建的原始 fields** 交给 `business_key_for(sheet_id, &fields)` 得到 `(key, kind)`，再喂给 `find_index_by_business_key`。
- 空键（kind=`none`）**不得用于查询**（既有约束）——调用方在阶段 1 前对空业务键记录直接 `Rejected`（相当于 `WPS_E_DOC_KEY`），不发起请求。
- 写入后回填 `wps_links` 时，`doc_type` 用 `record_type` 字符串（"02"/"03"…），不用数字，以对齐 `wps_record_index.record_type`。

## 3. WPS_E_DOC_* 在 W2 创建流程的用户呈现与重试策略

### 3.1 错误码语义（W1 已定）

| 码 | 触发 | 用户呈现（W2 建议） | 重试策略 |
|---|---|---|---|
| `WPS_E_DOC_KEY` | 单号为空 | 表单红字「单号不能为空」，不发起请求 | 不重试（本地拒绝） |
| `WPS_E_DOC_STALE` | 刷新不完整 / skipped>0 / 索引查询失败 | 弹层/横幅「数据未就绪，请稍后重试」（带 error/error_code 细节入日志，不直接抛给用户） | **指数退避重试 ≤3 次**（沿用只读重试策略）；仍失败给用户「稍后再试」+ 记录 `wps_sync_runs` |
| `WPS_E_DOC_DUPLICATE` | 同单号已存在 | 表单红字「该单号已存在：{record_id 对应单号}」，光标回到单号字段 | 不重试（业务拒绝）；用户换单号或确认后改走「更新」路径（W2 后续） |

### 3.2 呈现载体（W2 设计建议）

- 前端命令结果用结构体 `WpsCreateOutcome { status, error_code, remote_record_id }`，前端按 `error_code` 映射文案，**不把错误码原文直接铺给用户**（S0-04 日志脱敏：字段值不进日志，错误码可进）。
- `WPS_E_DOC_STALE` 的 `error/error_code` 细节只进 `wps_sync_runs` 与日志，不进前端文案。

### 3.3 重试边界

- 预检阶段的 STALE 属于**只读重试**：复用 `WpsRetryPolicy.read_attempts`（现状 3 次），不新增写侧重试。
- 写入阶段的失败**不自动重试**（现状：`create_record` 不重试，`pending/unknown` 需人工确认）——本设计不改变写入语义。
- DUPLICATE 不重试（业务拒绝，重试无意义）。

## 4. 预检只读、不写 wps_links；回填由写入路径负责

- **明确边界**：阶段 1（预检）的只读属性：
  - 允许写：`wps_record_index`（`pull_sheet` 的 UPSERT）、`wps_scans`（`begin_scan/finish_scan`）——这是「刷新本地索引」的定义，属只读刷新的本地落盘，不是业务写入；
  - **禁止写**：`wps_links`、`wps_idempotency_keys`、`wps_tombstones`、`wps_conflicts`、`wps_master`、`wps_run_created`。
- 预检返回「允许/拒绝」后，**不产生任何 `wps_links` 行**；`wps_links` 只在阶段 3 由写入路径（`WpsWriteSession` 回填或新增 `WpsCreateFlow` 回填方法）写入。
- `voucher_id` 默认 NULL：本地凭证与 WPS 记录的认领在 W2 后续（`find_index_by_record_id` 回查）处理，回填不负责认领。

## 5. 与 09 门复用边界

### 5.1 复用面（现状已共享，W2 不破坏）

| 能力 | 提供方 | 09 门 | doc guard | W2 复用建议 |
|---|---|---|---|---|
| 分页拉取 | `wps_pull::pull_sheet` | ✅ | ✅ | 继续共用；**不新建第二个 puller** |
| 扫描生命周期 | `wps_index::begin_scan/finish_scan` | ✅（经 pull_sheet） | ✅（经 pull_sheet） | 继续共用 |
| 索引查询 | `wps_index::find_index_by_business_key` | ✅ | ✅ | 继续共用 |
| 业务键提取 | `wps_index::business_key_for` | ✅ | ✅ | 单一真源 |
| 报告类型 | `wps_pull::WpsPullReport` | ✅ | ✅ | 继续共用 |

- **`WpsPuller` 不存在**：现状没有独立的 `WpsPuller` 结构，`pull_sheet` 是自由函数；两守卫都直接调它。W2 **不需要**新造 `WpsPuller` 抽象（YAGNI），除非出现「一个流程需同时刷新多张表并合并报告」的明确需求，再评估。

### 5.2 scan 生命周期

- 每个 `pull_sheet` 调用内部**自己** begin/end 一个 scan（一对多嵌套的天然边界）：09 门一次刷新 = 1 个 scan，doc guard 一个 sheet 一次刷新 = 1 个 scan。
- 一次创建流程（如「建 02 → 建 03 → 建 09」）会触发 **3 次独立全表刷新 = 3 个 scan**。这是**正确行为**（每张表独立确认最新状态），不是 bug；代价评估见 §5.3。

### 5.3 性能 / 配额影响（W0 页预算，W2 需明确）

| sheet | 记录数（W0 实测） | 页数（page_size=500） | 单次刷新 read 调用 |
|---|---|---|---|
| 02 | 783 | 2 | 1–2 |
| 03 | 763 | 2 | 1–2 |
| 04 | 545 | 2 | 1–2 |
| 06 | 1648 | 4 | 1–4 |
| 09 | 45 | 1 | 1 |
| 11 | 7056 | 15 | 1–15 |

- 单条创建最坏（e.g. 建 11）会拉 15 页；若一次创建 50 条 11（批量），**当前设计每条全量刷新 = 成本不可接受**。
- **W2 设计建议（性能优化项，不阻塞第一版）**：
  - **批内单次刷新（粒度写死）**：`WpsCreateFlow::create_doc_records` 对同一 `spec` 的多条记录，**每张涉及的 sheet 只刷新一次**（一次 `pull_sheet`），然后**所有记录查同一份索引**（每单号一次 `find_index_by_business_key`，本地查询成本低），而非每条一刷。
  - **跨表不做合并刷新**：若一次创建流程涉及 **02/03/04/06 四张表**，是 **四次独立 `pull_sheet`**（每张全表刷新一次），**不是**一次刷新覆盖四张表——`pull_sheet` 以单 sheet 为粒度，scan 一一对应，不存在跨表合并刷新。
  - 进一步：同一 run 内同 sheet 只刷一次（run 级缓存/去重，待 W2 评估）；此属优化项，不改变「每 sheet 一次全量刷新」的语义。
  - 配额提示：mock 阶段无配额；真实 HTTP 后按 W0 的 429 退避策略兜底。

## 6. 本轮范围声明

- **只出设计，不实现**：本文件不新增/修改任何 `.rs`；`WpsCreateFlow`、阶段 3 回填、批内单次刷新均为 W2 待办。
- 不接真实 HTTP（`transport` 仍 `Mock`）、不进入 W2 写入、不改表结构、不改 `wps_09.rs`。
- 待用户裁决后，本设计作为 W2 的实现依据进入开发日志。

## 7. 关联挂账与前置

- #63 主数据创建策略（A/B）待产品负责人确认——**本设计假设主数据已存在**（引用单号表时，主数据表 01/08/10 不在 doc guard 范围，见修订清单）。
- #64/#65/#66 截止 W2 启动前；本设计不依赖其销账。
- G0 重审依赖 #66（方案正文落盘）完成。

## 8. 干跑命令 `w2-create-dry-run`（只读预检可见化，2026-11-24）

> **状态：已接线、未验收。** 命令已落在 `crates/voucher-cli/src/main.rs:1072`（`w2_create_dry_run`，分派 `:1284`；用法行 `:46`），只跑 Mock/离线路径：传输层是「恒报错」的 `OfflineDryRunTransport`（`:988-1013`），配置写死 `W2Config::mock_write()`（`:1107`）。三段式**写入**仍未接任何业务命令；`wps_links` 的生产写入点有两处——`WpsStore::insert_link`（`services/wps.rs:1067`，`state='synced'`，三段式回填）与 `W2Sync` 内部 `write_link`（`services/wps_w2.rs:1068`，`state=linked|unclaimed`，原 W2 路径经 `W2Sync::execute` 调用），两者均受 C5 与 G1 约束。
> 本节 `main.rs` 行号取自落纸时的工作树；该文件同轮由后端改动，行号可能后移——检索以符号名（`w2_create_dry_run`、`OfflineDryRunTransport`、`parse_dry_run_requests`）为准。
> 定位一句话：干跑是「第 1 段（预检）的可见化」，**不是**「写入的彩排」，也不是写入口。§1.1 记录的是 2026-11-20 设计时点现状，当前口径以本节为准。

### 8.1 现状与定位

| 项 | 状态 | 依据 |
|---|---|---|
| 库层干跑 | 已落地：`dry_run(&W2Config, &[W2CreateRequest]) -> Result<WpsCreateDryRun>`，内部只调 `W2Sync::preflight` | `crates/voucher-core/src/services/wps_create.rs:107-147` |
| 三段式写入 | 已落地 `create_doc_records`/`create_meter_records` | `wps_create.rs:149`、`:205` |
| 干跑命令 | **已接线**：`voucher-cli w2-create-dry-run --db … --input …` | `crates/voucher-cli/src/main.rs:1072`、`:1284`、`:46` |
| 写入业务命令 | **未接线**：无 CLI 子命令、无 Tauri 命令 | `grep` 无 `execute` 调用方；§15.2「尚未接任何业务命令」 |
| 传输 | 双闸门：`OfflineDryRunTransport` 运行期恒 `Err`；命令段源码级静态测试禁网络符号 | `main.rs:988-1013`、`:1145-1176` |

### 8.2 命令形状

```
voucher-cli w2-create-dry-run --db <sqlite 路径> --input <输入.json> [--allow-existing-db]
```

| 参数 | 必填 | 语义 |
|---|---|---|
| `--db <file>` | 是 | 本地 SQLite。**默认只接受「不存在或 0 字节」的路径**：不存在则新建并跑既有迁移；已存在且非空 → exit 2，且**根本不打开该文件**（路径判定前置，因为 `open` 本身会改写 journal 模式）。预检读 `wps_record_index`、`wps_master`、09 单号索引、`wps_idempotency_keys` 与冷却键 `wps.rate_limit_until` |
| `--allow-existing-db` | 否 | 显式放行既有库：允许 open，但**跳过** `schema::init`，改以 `WpsStore::ensure_schema()` 只查 `sqlite_master`/`wps_state`；缺表即 exit 1（不自动建表），齐全则只读试跑 |
| `--input <file>` | 是 | 单表一批新单，UTF-8 JSON（§8.3）；解析在打开数据库之前完成，形状错即退出 2、不建库 |
| `--mode` / `--json` / 传输参数 | — | **均不支持**：配置写死 `mock_write()`，输出固定单行 JSON。不要按「有 `--json` 开关」去写脚本 |

### 8.3 输入 JSON 契约（单表一批）

根对象固定两个键（`main.rs:1014-1063`）：

```json
{ "sheet_id": 3,
  "records": [ { "fields": { "应收单号": "YS-001", "金额": 100 },
                 "idempotency_key": "idem-1",
                 "business_date": "2026-11-20" } ] }
```

| 键 | 必填 | 校验/语义 |
|---|---|---|
| `sheet_id` | 是 | 整数；缺 → 「输入缺少整数 sheet_id」退出 2 |
| `records` | 是 | 非空数组；缺或空 → 「输入 records 缺失或为空」退出 2 |
| `records[].fields` | 是 | 必须为对象；否则「第 N 条记录缺少对象字段 fields」退出 2。业务键由 `business_key_for` 派生（05/07 → `WPS_E_DEDUPE_ONLY`，01/08/10 → `WPS_E_REFERENCE_ONLY`，缺失 → `WPS_E_BUSINESS_KEY_MISSING`） |
| `records[].idempotency_key` | 是 | trim 后非空；否则退出 2。库内已存在 → `WPS_E_IDEMPOTENCY_STATE`（拒绝自动重试） |
| `records[].business_date` | 02/03/04/06/11 必填 | 显式给出、不猜录入日；缺失 → `WPS_E_DATE_MISSING`；早于 2026-11-01 → `WPS_E_LEGACY_READONLY`；非法 → `WPS_E_DATE_INVALID` |
| `records[].period` | 09 必填 | `YYYY-MM`；缺失时按 `fields["期间"]` 兜底（`wps_w2.rs:253-261`） |
| `record_type` | — | **不可由输入指定**：由 `doc_spec_for_sheet(sheet_id)` 推导，其余表按两位数字兜底（`main.rs:1031-1034`） |
| `required_refs` / `required_ref_ids` / `target_voucher_id` | — | **不解析**，恒为空。因此干跑**测不到** 01/08/10 引用门（`WPS_E_REF_MISSING`/`WPS_E_REF_STALE`）与目标凭证门（`WPS_E_TARGET_MISSING`） |

### 8.4 输出 JSON 契约（单行）

```json
{"ok":true,"command":"w2-create-dry-run","transport":"offline","accepted":false,"sheet_id":3,
 "outcomes":[{"idempotency_key":"idem-1","status":"rejected","code":"WPS_E_DEDUPE_ONLY","message":"…"}]}
```

| 字段 | 类型 | 语义 |
|---|---|---|
| `ok` | bool | 命令正常执行完即为 `true`，与是否放行无关 |
| `command` | string | 固定 `w2-create-dry-run` |
| `transport` | string | 固定 `offline` |
| `accepted` | bool | 预检结论：`true`＝整批放行，`false`＝整批拒绝 |
| `sheet_id` | number | 回显输入表号 |
| `outcomes[].idempotency_key` | string | 回显输入条目 |
| `outcomes[].status` | string | `would_create` 或 `rejected`（**snake_case**，勿按 DTO 的 camelCase 反推） |
| `outcomes[].code` / `message` | string? | 仅 `rejected` 有值 |

- `accepted=true` ⇒ 每条 `would_create`、`code/message` 为 `null`；条数与顺序等于输入。
- `accepted=false` ⇒ **每条都是同一个** `code`/`message`（`wps_create.rs:126-140` 整批复制预检结论）——**看不到哪一条先触发**，`W2Reject.index`（`wps_w2.rs:264-279`）未进入输出。
- 不含业务键、行号、远端记录 ID、链接 ID（干跑没有这些产物）。

### 8.5 退出码语义

| 退出码 | 含义 | 触发点（`main.rs`） |
|---|---|---|
| `0` | 干跑执行完成：`accepted=true` **或** `accepted=false` | `:1135`——「拒绝是业务结果，仍 exit 0」；判定必须读 `accepted`/`code`，不得用退出码区分放行与拒绝 |
| `1` | 业务/IO 失败：打开数据库失败、初始化表结构失败、预检返回 `Err` | `:1097`、`:1100`、`:1109` |
| `2` | 参数或输入形状错：缺 `--db`/`--input`、读文件失败、JSON 非法、`sheet_id`/`records`/`fields`/`idempotency_key` 不合格、**`--db` 指向既有非空库且未给 `--allow-existing-db`** | `w2_create_dry_run` 内的路径判定与输入解析分支 |

退出码口径与 CLI 顶部约定一致（`main.rs:14`：0 成功／1 业务错误／2 参数错误）。

### 8.6 它只做预检（三条硬约束）

- **不产生 `wps_write_intents`**：`insert_intent` 只在写入段出现（`wps_w2.rs:1014-1045`），命令段无调用。
- **不写 `wps_links`**：`write_link`/`insert_link` 只在写入段与回填分支（`wps_w2.rs:1068`、`wps_create.rs:149`）。
- **发不出网**：`OfflineDryRunTransport::initialize`/`call` 恒返回错误（`main.rs:990-1013`），配置写死 `mock_write()`，命令不接受任何端点、token、传输参数。

可验证判据（四条同时成立才叫「零写入」）：

1. `OfflineDryRunTransport` 被调用即失败；源码级静态测试禁止命令段出现 `reqwest`/`ureq`/`https://`/`http://`/`HttpsP2Transport`/`P2Transport`（`main.rs:1145-1176`）；
2. `wps_write_intents`/`wps_links`/`wps_sync_runs`/`wps_idempotency_keys` 四表行数前后不变；
3. 命令段静态无业务表 `INSERT`/`UPDATE`、无 `start_run`/`set_rate_limit_cooldown`。注意 `--db` 不存在时会建库建表（DDL 属初始化，**不**算写入业务数据），所以判据只能断言业务表零增量，不能主张「零 DDL」；
4. 拒绝分支先于任何写入准备：预检在任何写表动作之前返回。

### 8.7 它**不能**证明什么

| 不能证明 | 原因 |
|---|---|
| 真实 WPS 端点会接受这批记录 | 零请求；端点/认证头/会话/DELETE/响应形状全未触及 |
| 服务端按 `fields` 裁剪 | 属服务端能力（Q1 未答，见 70-官方核验清单 §1） |
| 写入阻断与字段一致性（C5） | 无快照、无 create 轨迹、无一致性判定；C5 不在 S0 签署范围内 |
| 写入会成功 | 放行只是必要条件；提交时重新预检且远端可变 |
| 哪一条先触发拒绝 | 整批同一码（§8.4） |
| 引用门与目标凭证门 | 输入不解析 `required_refs`/`required_ref_ids`/`target_voucher_id`（§8.3），这几道守卫在干跑里恒为空 |
| 入口权限门的拒绝文案 | 配置写死 `mock_write()`，`pull-only` 分支不可达 |

### 8.8 与三段式全链路的关系

| 段 | 内容 | 干跑命令覆盖 | G1 前可达性 |
|---|---|---|---|
| 第 1 段 预检 | 入口权限、日期分界、业务键、批内/历史重复、必填引用、目标凭证、冷却 | ✅ 主体可见（引用门/目标门见 §8.3 限制） | 可达（只读） |
| 第 2 段 写入 | `create_doc_records`/`create_meter_records` → `WpsWriteSession` 逐条（无批量） | ❌ 不触及 | 不可达：无业务命令 + G1 未放行 |
| 第 3 段 回填 | `wps_links`、`PendingBackfill`、`recover_link` | ❌ 不触及 | 不可达：依赖第 2 段产物 |

一句话：干跑是第 1 段的可见化，第 2/3 段在 G1 前没有入口。

### 8.9 使用纪律

- 只在临时或 Mock SQLite、合成数据下运行。生产库保护已补（2026-11-24 t14）：默认拒绝既有非空库且不打开它；`--allow-existing-db` 只放行 open、不建表。故「指向生产库」的误用不再可能悄悄发生，但仍**不得**主动把 `--db` 指向生产库。
- 只在离线路径使用；不得为它增加端点/token/传输参数，也不得把 `OfflineDryRunTransport` 换成真实传输。
- 输出仅用于开发自检与走查；**不得**写入验收证据当作「写入已验证」，也不得据此改签任何 S0/G1 结论。
- 用例数不以本文为准：见 `docs/开发日志.md` 第一百四十五笔。

### 8.10 本地形状实测（2026-11-24，临时库，非验收）

`target/debug/voucher-cli` 在 `/tmp/w2dry/` 临时目录、合成输入、离线传输下实跑，逐条对照 §8.3–§8.5：

| 场景 | 命令要点 | 结果 | 退出码 |
|---|---|---|---|
| 缺参 | 无 `--db`/`--input` | stderr「错误：w2-create-dry-run 需要 --db 与 --input」 | `2` |
| 合法批次（03 表） | `fields{"应收单号":"YS-DRY-001"}` / `business_date 2026-11-20` | `accepted=true`、单条 `would_create`、`code/message` 为 `null` | `0` |
| 05 表（只查重） | `sheet_id 5` | `accepted=false`、`rejected`、`WPS_E_DEDUPE_ONLY`（「表 5 首批仅全表查重，不生成业务行」） | `0` |
| 非法 JSON | `not-json` | stderr「输入不是合法 JSON」 | `2` |

同一轮跑完查临时库：`wps_write_intents`/`wps_links`/`wps_sync_runs`/`wps_idempotency_keys` 四表均 `0` 行，与 §8.6 判据 2 一致。**这是形状自检，不是验收证据**：不构成写入验证、不涉及真实端点与 C5。
