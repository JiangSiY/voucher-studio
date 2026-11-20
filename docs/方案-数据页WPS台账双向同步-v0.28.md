# 数据页 ↔ WPS 台账双向同步开发方案（v0.28 修订版·重建稿）

> **重建说明（2026-11-20）**：原方案正文未在仓库/Obsidian 落盘（`git log --all`、工作区、vault 均无）。本文件按 [`docs/方案-销账64-65-66.md`](./方案-销账64-65-66.md) 的「#66 每章重建依据映射表」，**只从已落盘证据重建**，不引用对话记忆，不从对话推导业务规则。每章末的「本章依据」给出可回追的文档与节；没有落盘依据的内容一律标【待补】。
>
> **状态**：重建稿，交技术负责人发起 G0 重审；**不自行宣布通过**。`transport` 仍为 `Mock`，不接真实 HTTP，不进入 W2。
>
> **附录**：[`docs/评审修订清单-2026-11-20.md`](./评审修订清单-2026-11-20.md)（原评审修订清单，现降为本文附录，保留 9 项修正记录与 view_id 定稿）。

## 1. 文档定位

- 本文是 v0.28「数据页 ↔ WPS 台账双向同步」的方案正文重建稿，供 G0 重审、W2 启动前复核。
- 本文不是实现记录；实现状态以各证据文件与 `docs/开发日志.md` 为准。
- 本文与历史评审清单冲突时，以本文正文为准；清单保留为修订过程附录。

## 2. 范围与边界

| 项 | 内容 | 依据 |
|---|---|---|
| 涉及表 | 01 往来对象、02 项目、03 应收、04 应付、06 资金流水、09 抄表、11 出入库；单号表另含 05/07/08/10 的只读去重 | S0 检查表 §1；`实施说明` §10.1 |
| 写入范围 | 当前只做 mock/本地；真实 HTTP、生产写入均关闭 | S0 §4；`W1-T3-阻塞.md` |
| 不做 | W2 创建流程、`wps_links` 回填、真实 HTTP、生产写入 | 本轮边界（`开发日志` 第九十二至九十四笔） |
| 时间分界 | 【待补】「2026-11-01 起双向、之前历史只读」目前只有「11/1 后」作为 `view_id` 固定范围示例（W0-12），**没有落盘的切分规则/字段/例外**，不得据此实现 | W0 接口记录 §2 W0-12 |

## 3. 数据模型与迁移

### 3.1 表清单

`SCHEMA_WPS` 当前 12 张表（`SCHEMA_VERSION` 保持 4，不 DROP/ALTER 已有表、不清库）：

`wps_links`、`wps_tombstones`、`wps_scans`、`wps_conflicts`、`wps_schema`、`wps_master`、`wps_sync_runs`、`wps_run_changes`、`wps_run_created`、`wps_idempotency_keys`、`wps_record_index`、`wps_state`。

- `wps_links`：`UNIQUE(sheet_id, record_id)`；`voucher_id` 部分唯一索引；`idempotency_key` 普通索引；`voucher_id` 外键 `ON DELETE CASCADE`。
- `wps_conflicts`：`link_id` 外键 `ON DELETE CASCADE`；`UNIQUE(link_id, field_id)`。
- `wps_run_changes`：`UNIQUE(run_id, link_id, field_id)`；无外键，保留审计。
- `wps_idempotency_keys`：`idempotency_key` 主键、`run_id`、`record_type`、`local_status`（`pending/succeeded/unknown`）、`remote_record_id`、`created_at`。
- `wps_record_index`：`UNIQUE(sheet_id, record_id)`；`idx_wps_index_key(sheet_id,business_key)`、`idx_wps_index_type(sheet_id,record_type)`。
- `wps_state`：写入 `schema_version=wps-1`。

### 3.2 迁移方式

- 方案 A：保持 `SCHEMA_VERSION=4`，`SCHEMA_WPS` 全部 `CREATE TABLE/INDEX IF NOT EXISTS`，在 `schema::init()` 末尾事务内执行；不 bump、不清库。
- 迁移前快照：已有 `vouchers` 数据且 `wps_links` 不存在时，`VACUUM INTO backups/pre-wps-*.db`，保留最近 3 份；失败中止启动。
- 建表失败：回滚，应用报错并停用同步；不得继续调用 WPS。
- 运行期自检：`WpsStore::ensure_schema` 遍历 `WPS_TABLES`，缺表/版本不符返回 `WPS_E_SCHEMA`。

**本章依据**：[`迁移设计-WPS台账-v0.28.md`](./迁移设计-WPS台账-v0.28.md) §1–§4；[`W1/schema.md`](./验收基线/证据/2026-11-19-W1/schema.md) §1–§3；[`W1/pull-index.md`](./验收基线/证据/2026-11-19-W1/pull-index.md) §2–§3；`crates/voucher-core/src/db/schema.rs`。

## 4. 预检方案（A 分页全表 + 本地索引）

### 4.1 结论

- **A（唯一预检路径）**：分页全表拉取 + 本地 SQLite 索引，按业务键比对。
- **B（不成立）**：`view_id` 过滤确实生效，但视图条件是静态的，MCP 白名单没有建/改视图工具，无法为任意单号动态建视图；B 只能用于固定范围缩窄。
- **view_id 缩窄**：降级为**未验证优化项**，不作为 G0/G1 门禁依赖；需要验证时只用**已存在的视图**做只读拉取，不在应用内新建/修改视图。
- 09 门与 doc guard 一律 `view_id=None` 整表刷新。

### 4.2 成本与页预算

`page_size=500` 时，W0 记录的页预算：02/03/04 约 2 页、06 约 4 页、09 1 页、11 约 15 页。单轮 02–11 合计约 26 页；MCP 单轮可接受请求数仍无官方数据，靠 W1 实测/429 退避。

### 4.3 创建前检查顺序

- 09：key 校验（0 请求）→ 刷新 09 全表 → `complete=false` 或 `skipped>0` → `WPS_E_09_STALE` → 查索引 → `WPS_E_09_DUPLICATE` → 放行。
- 单号表：key 校验（0 请求）→ 刷新对应表 → 同上 → `WPS_E_DOC_KEY/STALE/DUPLICATE`。
- 两者全程只调用 `dbsheet.list_records`，不发送写请求；doc guard 只查单号重复，**不查主数据引用**。

**本章依据**：[`W0/接口记录.md`](./验收基线/证据/2026-11-19-W0/接口记录.md) §2 W0-11/W0-12、§3、§4；[`实施说明`](./实施说明-WPS幂等-v0.28.md) §9.6、§10.1–§10.4；[`W1/wps09.md`](./验收基线/证据/2026-11-19-W1/wps09.md) §1。

## 5. 分页拉取与本地索引

- `pull_sheet(client, store, sheet_id, record_type, view_id, page_size)`：`page_size` 上限 500，页上限 1,000；支持 `records`/`data.records`/`data` 数组，坏记录计入 `skipped`，缺 records 数组返回 `WPS_E_PULL_MALFORMED`。
- 远端/解析/offset 不前进：`finish_scan(complete=false)` 后返回 `Ok(report)`；数据库错误返回 `Err`。
- 正常结束：空页、`has_more=false`、`total` 到达、短页且非 `has_more=true`；达到页上限必须 `complete=false` + `WPS_E_PULL_MAX_PAGES`。
- 索引键与业务键由 `wps_index::business_key_for` 单一真源生成；字段依据见 `财务工作流/设计/项目台账_v2_设计.md`（02 L92、03 L119、04 L137、05 L159、06 L174、07 L192、08 L197、10 L205、11 L210）。

**本章依据**：[`W1/pull.md`](./验收基线/证据/2026-11-19-W1/pull.md) §2–§4；[`W1/pull-index.md`](./验收基线/证据/2026-11-19-W1/pull-index.md) §3–§4；`crates/voucher-core/src/services/wps_pull.rs`、`wps_index.rs`。

## 6. 写入接口与幂等

### 6.1 破坏性变更

- 旧 `create_records(sheet_id, records)` **已删除**，不保留 `#[deprecated]` 包装。
- 新接口以 `KeyedRecord { record, idempotency_key }` 为元素：`create_records(sheet_id, &[KeyedRecord])`；`create_record(sheet_id, record, key)`。缺少 key 的调用无法通过类型构造（另有 `compile_fail` doctest 固定）。
- 写入路径**统一逐条**调用，`build_create_args` 拒绝单次多条；**不调用 WPS 原生批量接口**。
- 仓库内旧签名调用点已全部迁移；无外部调用方（crate 内公共 API，尚无下游 crate）。

### 6.2 幂等语义

- 创建类每条记录一个 UUID，`idempotency_key` 放在 `tools/call` 的 `arguments` 顶层。
- 缺失 key：本地拒绝 `WPS_E_IDEMPOTENCY_MISSING`，0 请求；记录数与 key 数不一致：`WPS_E_IDEMPOTENCY_COUNT_MISMATCH`，0 请求（内部构造器防御）。
- 更新类 key 可选，不强制。
- 本地 `wps_idempotency_keys` 记录 `pending/succeeded/unknown`、`remote_record_id`；成功回填 remote id。
- **WPS 服务端是否执行 idempotency_key 未验证**；该字段只作客户端追溯/去重标识，不构成服务端幂等保证。
- `pending`/`unknown` 不得自动重试写入；需人工确认。

### 6.3 部分失败

逐条写入：第 1 条成功、第 2 条超时 → 第 1 条 `succeeded`、第 2 条 `unknown`、第 3 条 `not_attempted`；不回滚整批，不自动重试。

**本章依据**：[`实施说明`](./实施说明-WPS幂等-v0.28.md) §2、§3、§5、§6、§7；`crates/voucher-core/src/services/wps.rs`；`docs/实施说明-WPS幂等-v0.28.md` §「方案 §4/§6 写入接口说明」。

## 7. 错误码与重试

### 7.1 错误码

| 类别 | 码 |
|---|---|
| 幂等 | `WPS_E_IDEMPOTENCY_MISSING`、`WPS_E_IDEMPOTENCY_COUNT_MISMATCH` |
| 会话/认证 | `WPS_E_SESSION`（HTTP 404）、`WPS_E_AUTH`（400006） |
| 传输 | `WPS_E_RATE`（429）、`WPS_E_TIMEOUT`、`WPS_E_REMOTE`、`WPS_E_PROTOCOL` |
| 结构 | `WPS_E_SCHEMA` |
| 分页 | `WPS_E_PULL_MALFORMED`、`WPS_E_PULL_MAX_PAGES` |
| 09 门 | `WPS_E_09_KEY`、`WPS_E_09_STALE`、`WPS_E_09_DUPLICATE` |
| 单号门 | `WPS_E_DOC_KEY`、`WPS_E_DOC_STALE`、`WPS_E_DOC_DUPLICATE` |

**口径**：已统一为 `WPS_E_PULL_MALFORMED`，无残留；页上限 `WPS_E_PULL_MAX_PAGES`；09 三码；单号门新增 `WPS_E_DOC_*`。

### 7.2 重试

- 只读：总尝试 3 次，1s 起、2×、30s 上限、full jitter；429/5xx 重试；400006 不重试；404 重初始化一次后重试一次。
- 写入：1 次尝试、0 次自动重试；超时/断连原样返回，不重放。
- 09/单号门：刷新失败统一映射 `*_STALE`，文本带 `complete/skipped/error/error_code`，不泄露字段值。
- `view_id` 缩窄未验证，不得作为重试/门禁依据。

**本章依据**：[`实施说明`](./实施说明-WPS幂等-v0.28.md) §2、§4、§8、§9.5、§10.4；[`W1/wps09.md`](./验收基线/证据/2026-11-19-W1/wps09.md) §1、§5；[`W1/pull.md`](./验收基线/证据/2026-11-19-W1/pull.md) §3。

## 8. 同步合并、删除与墓碑

【待补】现有落盘证据只给出表名与部分索引/外键（`wps_tombstones`、`wps_conflicts`、`wps_run_created`），**没有**三向合并规则、冲突判定、删除检测、墓碑抑制重建、未认领行忽略等业务语义的落盘依据。G0 重审前必须由技术负责人补齐以下内容并落盘：

- 本地/远端/base 三向合并与冲突边界；
- 本地删除拦截与墓碑写入顺序；
- WPS 行「未出现」的判定条件（全表扫描、`scans.complete=1`）；
- 虚拟行/认领流程与 `wps_links.voucher_id` 归属。

在补齐前，不得在 W2 实现这些语义。

## 9. 数据页与交互

【待补】现有落盘证据不含数据页列集、虚拟行展示、认领交互、同步弹层、批量操作、移动端布局的定稿；`docs/设计-doc-guard接入W2预检-v0.28.md` 只覆盖 W2 创建入口设计，不能替代本方案。需在 W2 前另立 UI 设计并落盘。

## 10. 调度与运行

【待补】现有证据不含同步触发频率、单写心跳、运行锁、失败重试编排、运行记录保留策略的落盘定稿；`wps_sync_runs` 仅表存在。不得以对话中的 5 分钟/15 分钟等口头数字作为实现依据。

## 11. 配置与开关

| 键 | 取值 | 默认 | 说明 |
|---|---|---|---|
| `wps.transport` | `mock`/`http` | `mock` | `http` 须 S0 门禁通过 |
| `s0_approved` | `false`/`true` | `false` | `http` 时必须为 true，否则启动校验拒绝 |
| `wps.read_retry.*` | 3/1000/2/30000/true | 同左 | 只读重试 |
| `wps.write_retry.attempts` | 1 | 1 | 写入 0 次自动重试 |
| `write_max_attempts` | 1 | 1 | 写入总尝试次数；0 或 >1 拒绝 |
| `write_retry_enabled` | `false` | false | 当前必须 false |

**待补**：`WpsConfig` 的仓库配置载体/启动入口/S0 门禁键名尚未接线；`WpsConfig` 目前只是测试契约。

**本章依据**：[`实施说明`](./实施说明-WPS幂等-v0.28.md) §1；`crates/voucher-core/src/services/wps.rs` `WpsConfig`。

## 12. 安全与 S0

- S0 检查表 12 项（token 钥匙串/不可回读/无明文回退、日志脱敏、权限最小化、400006/429、TLS、备份不含 token、审计、故障演练）**尚未签字**。
- 真实 WPS 读取/写入在 S0 通过前一律关闭，`transport=mock`；W0 仅使用测试副本，生产文件未写入。
- token 只存系统钥匙串；前端永不回读；错误日志只记 sheet/record id、计数、错误码，不记字段值/金额/电话/税号。
- S0 未批准期间的创建前预检只走 A 主线（只读 doc guard/09 门），不依赖 `view_id` 缩窄。

**本章依据**：[`S0/检查表.md`](./验收基线/证据/2026-11-19-S0/检查表.md) §1–§4；[`W0/接口记录.md`](./验收基线/证据/2026-11-19-W0/接口记录.md) §1、§3、§4。

## 13. 测试

| 层 | 证据 |
|---|---|
| 迁移/快照 | `W1/schema.md` 3 例 |
| 本地索引 | `W1/pull-index.md` 7 例 |
| 分页拉取 | `W1/pull.md` 9 例 |
| 09 门 | `W1/wps09.md` 7 例（B4 后 11 例） |
| 单号门 | `W1/wps-client.md`/doc guard 7 例（见开发日志第九十三笔） |
| 幂等/写入 | `W1/wps-client.md` 23 例；字节/compile_fail 见实施说明 §7 |
| 前端 | 数据页契约 `src/pages/__tests__/dataPageContract.test.ts` |

**口径**：Rust 用例数是 `#[test]` 静态登记数，不等于通过数；历史 `421/424` 含 1 例挂账失败 #65，本轮修复后以 README/清单1/开发日志实测为准。前端写「实测」，不写静态 `it(` 计数。

## 14. 批次与交付

| 批次 | 内容 | 状态 |
|---|---|---|
| W1-B1 | 同步表/快照/本地索引 | ✅ 已提交（`68e13b5`） |
| W1-B2 | 分页全表拉取 | ✅ 已提交（`11830c7`） |
| W1-B3 | 09 创建前门 | ✅ 已提交（`0950206`） |
| W1-B4 | 09 四点核对/allow 清理 | ✅ 已提交（`b63ba87`） |
| W1 收口 | doc guard + 评审修正 1–9 | ✅ 已提交（`c0b7d82`） |
| W2 | 创建流程、`wps_links` 回填、真实 HTTP | ⛔ 未启动；G0/S0 未过 |

## 15. G0/G1 门禁与验收

| 门 | 条件 | 当前状态 | 证据 |
|---|---|---|---|
| G0 | W0 条件 GO；filter 预检改为 A；doc guard/09 门只读；G0 重审通过 | **未通过**；W0 条件 GO（仅 mock/本地），A 已定为唯一路径，本文重建待重审 | `W0/接口记录.md` §3–§4；本文 |
| G1 | S0 12 项签字；真实 HTTP 打开 | **未通过**；S0 待签，`transport=mock` | `S0/检查表.md` §2–§3 |
| W1 | B1–B4 + doc guard 本地验收 | **条件收口**；全量门禁见开发日志第九十二至九十四笔 | `W1/*` 证据 |
| W2 | 创建流程/回填/真实写入 | **禁止启动** | 本文 §6、§12 |

**G0 重审材料清单**：

1. 本文重建稿（§4/§6/§7/§12/§15 齐备）；
2. 附录 `docs/评审修订清单-2026-11-20.md`（9 项修正记录）；
3. W0 接口记录（A/B 结论、W0-11/12、条件 GO）；
4. S0 检查表（12 项 + 「S0 未批准落 A」留痕）；
5. W1 证据：`schema.md`、`pull-index.md`、`pull.md`、`wps09.md`、`wps-client.md`；
6. 开发日志第八十九至九十四笔与挂账 #63–#66；
7. 本轮 #64/#65/P1 的修复证据（见开发日志第九十五笔、`audit.sh --strict` 与全量测试输出）。

**仍待确认**：#63 主数据路径（A/B）、#66 本重建稿的评审、S0 签字、G0 重审结论、真实 HTTP 启用。

**本章依据**：`docs/开发日志.md` 第八十九至九十五笔；`docs/验收基线/证据/2026-11-19-W1/*`；`docs/验收基线/证据/2026-11-19-S0/检查表.md`；`docs/验收基线/证据/2026-11-19-W0/接口记录.md`。
