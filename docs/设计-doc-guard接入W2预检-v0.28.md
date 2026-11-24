# doc guard 接入 W2 预检设计（v0.28，只出设计，不写代码）

- 日期：2026-11-20
- 状态：**设计文档；本轮不实现、不接真实 HTTP、不进入 W2 写入**
  - 2026-11-24 补：设计中的写入三段式**仍未实现**；`services/wps_create.rs` 只落地了只读干跑 `WpsCreateFlow::dry_run`（复用 `W2Sync::preflight`），不提供 `create_doc_records`，写入口仍只有 `W2Sync::execute`。
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
