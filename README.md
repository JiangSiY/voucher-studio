# Voucher Studio

凭证整理打印工具 —— Rust + Tauri 2 + Vue 3 跨平台重写版（前身为 PySide6 版 voucher-print v4.3，旧数据不保留，全新空库上线）。

> **2026-11-24 干跑补两道门 + CLI 库路径守卫 + restore 覆盖守卫（Mock，零真实写入）**：`w2-create-dry-run` 输入增 `required_refs`/`required_ref_ids`/`target_voucher_id`，输出每条带 `gate`（`reference`/`target_voucher`），判定仍全部由核心 preflight 给出（未加错误码、未复制规则）。CLI 新增 `existing_db_path()`（`crates/voucher-cli/src/main.rs:89`），排在 `CoreContext::init` 之前，`list-pending`/`c5-fingerprint`/`import-ledger` 缺库即 exit 1 且**不建目录不建库**（后两者输出带 `database` 路径）。`restore --target` 指向已有数据（`voucher.db` 非 0 字节或 `-wal`/`-shm` 存在）时必须显式 `--allow-overwrite`，放行时先把 `voucher.db`+`-wal`+`-shm` 移入 `backups/pre-restore-<UTC>` 再落位——**避免旧 WAL 回放到新库**。全量实跑 **620 passed / 0 failed / 0 ignored**（34 个 test 二进制）；`voucher-cli` **50**（4+5+21+2+6+12）；`voucher-core` 静态 **551**（376 单元 + 175 集成），加壳 3 共 **554**；`services::wps` **177**；前端 **47 文件 247 用例**。边界：仍 `transport=mock`、零真实 WPS 请求、未写生产库、C5 写入阻断维持、G1 未放行；另 S0-02/S0-04 两条证据按事实更正（见开发日志第一百四十六笔）。

> **2026-11-24 W2 三段式写入路径 + 只读状态命令（Mock，零真实写入）**：`WpsCreateFlow` 三段式落地——业务键统一走 `business_key_for`、每条过 doc guard（05/07 复用 `SHEET_DEDUPE_ONLY` 拒写）、只经 `WpsWriteSession` 逐条写（每条独立幂等键、**无批量接口**）、仅成功记录回填 `wps_links`（新增 `WpsStore::insert_link`，`ON CONFLICT(sheet_id,record_id)` 幂等、`voucher_id` 留 NULL）；并列 `create_meter_records` 走 09 门，两守卫互不嵌套。回填失败分支用 `LinkWriter` trait 注入缝补了独立用例（错误不被吞、只发 1 次不重发、其它成功记录不受影响）。新增只读命令 `wps_sync_status`（`crates/voucher-api/src/wps_status.rs`）：`lastSyncAt`/`lastScanComplete`/`pendingCount`/`unknownCount`/`pendingItems`（仅 sheetId/idempotencyKey/code/state，不回显字段值），只查本地 SQLite、不构造客户端、不读凭据、零网络。前端据此把「最后同步时间」与「待处理队列」两处空态换成真实数据（徽标只算 `pendingCount`，`unknownCount` 单独成句）。全量实跑 **590 passed / 0 failed / 0 ignored**；`voucher-core` 静态 **551**（376 单元 + 175 集成），加壳 3 共 **554**；`services::wps` 过滤口径实测 **177**；前端 **47 文件 247 用例**。边界：仍 `transport=mock`、零真实 WPS 请求、未写生产库、C5 写入阻断维持、G1 未放行。

> **2026-11-24 W2 写入预检层（干跑）落地（Mock，零真实写入）**：新增 `crates/voucher-core/src/services/wps_create.rs` 的 `WpsCreateFlow::dry_run`，只复用既有 `W2Sync::preflight` 做请求前判定——空业务键 `WPS_E_BUSINESS_KEY_MISSING`、同批重复业务键 `WPS_E_BATCH_DUPLICATE`、05/07 查重专用 `WPS_E_DEDUPE_ONLY`、01/08/10 仅引用 `WPS_E_REFERENCE_ONLY` 均整批拒绝；不复制规则、不新增错误码、**不提供任何写入口**（C5 写入阻断维持）。新增 6 例（成功干跑、空业务键、同批重复键、05/07、01 引用、源码无写入符号静态判据），并给既有 05/07 阻断用例补四表零增量断言；全量实跑 **574 passed / 0 failed / 0 ignored**；`voucher-core` 静态 **544**（369 单元 + 175 集成），加壳 3 共 **547**；`services::wps` 过滤口径实测 **170**；前端 46 文件 239 用例（t3 前端新增重授权条/查看范围门 2 文件 12 例）。边界：仍 `transport=mock`、未接任何生产命令或桌面入口、零真实 WPS 请求、未写生产库。
>
> **2026-11-24 A2 扫描运行（scan-run）实现完成（授权单待签）**：按设计 v0.3 落地 `P2ScanRun`——运行级只消费一次（`grant_id`）、单逻辑会话（`initialize` ≤2 / `notify` ≤2 / `DELETE` ≤1）、两遍独立全扫并逐表 `sha256` 互比、页数锁死（2/1/8/8/1/1/1）、`P2_E_INCOMPLETE`/`P2_E_DRIFT`/`P2_E_INCONSISTENT` 三类停止码；预算 `P2_SCAN_MAX_READS=74`、`P2_SCAN_MAX_TOTAL_REQUESTS=80`。CLI 新增 `wps-p2-scan`（**无 `--cursor`/`--page`**，外部游标一律拒绝；默认 `offline` 零发网；`https` 需 `--allow-real-endpoint` + `--token-file`；`--check` 不建库）。离线验收 8 例（规格边界/两遍一致/指纹不等/缺页/越界整页拒绝/目标越界零请求/主体缺失不烧授权/主体不符零请求）+ CLI 15 例；**2026-11-24 第二轮补 Q2 约束落码**（`approved_file_id` 目标绑定、`approved_subject` 主体绑定、`check_subject` 先于 `consume`）；全量实跑 **568 passed / 0 failed / 0 ignored**；`voucher-core` 静态 **538**（363 单元 + 175 集成），加壳 3 共 **541**，`services::wps` 口径 **168**；前端 44 文件 227 用例。**授权单 `A2扫描级授权-01.json` 已于 2026-11-24 01:10Z 签署并消费一次**（首跑 `exit 0`，见证据 §5）；原件不含 `approved_file_id`，按 Q2 落码后的规则不可复跑，再跑须新签；扫描证据不构成 C5 ①②③、不解除写入阻断。
>
> **2026-11-23 P2 支持页游标续取（`--cursor`）**：`P2ReadRequest` 的数值 `offset` 换成 `cursor: Option<String>`，请求体 `offset` 原样透传服务端游标（首屏 `None` → `""`），CLI 增 `--cursor <游标>`；此前写死首屏、无法取第二页。核心用例把「非首屏拒绝」改为「游标原样进入请求体」，并补 `page_size` 不符仍零请求。全量实跑 **554 passed / 0 failed / 0 ignored**；`voucher-core` 静态 **530**（363 单元 + 167 集成），加壳 **533**，`services::wps` 口径 **164**；`voucher-cli` **14 例**（新增 `--cursor` 预检不建库）；前端 44 文件 227 用例。边界：单次授权仍只取一页，第二页需另签单并把上一页 `cursor` 传入。
>
> **2026-11-23 P2 只读 CLI 增字段级统计（`--stats`，不落值）（上一轮）**：`wps-p2-read --stats` 在成功输出里附 `stats`——逐字段的**非空计数 + JSON 类型分布 + 类日期计数**，只统计授权内的列，**不回显任何字段值**（note 明写）。用途：回答「金额是数值还是字符串」「日期返回形状」「自增列是否可读」这类 W2 写入映射问题。实现为 CLI 内本地聚合（`field_stats`），核心读取路径未改；新增单元测试 3 例（类型/日期形态/越权列忽略，断言输出不含 `1234.56`、`20260819-001`）。`voucher-cli` 用例 **10 → 13**；全量实跑 **553 passed / 0 failed / 0 ignored**；`voucher-core` 静态 **530**（363 单元 + 167 集成），加壳 **533**，`services::wps` 口径 **164**；前端 44 文件 227 用例。边界：仍只连显式授权的端点、单次授权、窗口/预算照旧；真实副本 stats 需再签一张单。
>
> **2026-11-23 C5 本地辅助证据（指纹；不关闭 C5）（上一轮）**：新增 `services/wps_c5.rs` 与 `voucher-cli c5-fingerprint`——对本地索引做**全字段指纹**（逐表按 `record_id` 排序，`fields_json` 规范化后取 `sha256`），两次比对可发现扫描期间的数据漂移，且只报表号与计数、不回显字段值。实跑（Mock 全量拉取）：`records=3 combined=1666f03db874 sheets=3 stable=true`，人工漂移后 `changed_sheets=[1]`。**边界**：无快照 token，不能证明源端一致，因此**不解除 C5 写入阻断**，真实 C5 仍按决策表 §4 三项归档。全量实跑 **553 passed / 0 failed / 0 ignored**；`voucher-core` 静态 **530**（363 单元 + 167 集成），加壳 **533**，`services::wps` 口径 **164**；前端 44 文件 227 用例。证据：[`docs/验收基线/证据/2026-11-23-C5-指纹/证据.md`](docs/验收基线/证据/2026-11-23-C5-指纹/证据.md)。

> **2026-11-23 P2 分页游标修复（27 vs 50 已定位）**：第二发 CLI 收到 27 条、插件同参数 50 条——根因是 SkillHub 的 `offset` 是**不透明字符串游标**（首屏 `""`），入口却发了数字 `0`。已改为发 `"offset": ""` 并把响应 `data.detail.offset` 作为 `cursor` 输出；新增本机 HTTPS 用例断言 `cursor` 与请求体形状，`services::wps_p2` 39 → **40 例**。全量实跑 **544 passed / 0 failed / 0 ignored**；`voucher-core` 静态 **524**（357 单元 + 167 集成），加壳 **527**，`services::wps` 口径 **158**；前端 44 文件 227 用例。待闭环：再签一发授权单确认 CLI 返回 50 条；Q2（单副本隔离）与真实 429/400006 仍未取得。证据：[`docs/验收基线/证据/2026-11-19-S0/A2授权单-合成专用表首联.md`](docs/验收基线/证据/2026-11-19-S0/A2授权单-合成专用表首联.md) §4.6。

> **2026-11-23 P2 首联真实调用一次 + 响应信封形状修复**：合成表授权单 `A2-SYN-20261123-02`（`fields=["code"]`）已签署并执行**一次**：预检 `ok`，执行 `exit 1` 报 `P2_E_MALFORMED`——真实端点的 `tools/call` 文本是 WPS 信封 `{code,message,data:{detail:{records,offset}}}`，旧 `validate_records` 只认顶层 `records`；该授权已在发网前登记消费，**不可重放**，`DELETE` 未发。用同一 MCP 工具的插件通道对同一合成表复核：只批 `code` 时响应只回 `code` → **Q1（服务端按 `fields` 裁剪）成立**。已修 `validate_records` 接受裸 `records` / `data.records` / `data.detail.records` 三种形状，`wps_p2` 36 → **39 例**（新增信封形状成功与越界整页拒绝）。全量实跑 **543 passed / 0 failed / 0 ignored**；Rust 静态 `voucher-core` **523**（356 单元 + 167 集成），加壳 3 共 **526**，`services::wps` 口径 **157**；前端 44 文件 227 用例。仍未验证：`DELETE` 是否被接受、Q2（单副本隔离）、C5；P2 未整体授权、A2 未重签、真实副本 C1–C6 未执行、S0/G1 未签。证据：[`docs/验收基线/证据/2026-11-19-S0/A2授权单-合成专用表首联.md`](docs/验收基线/证据/2026-11-19-S0/A2授权单-合成专用表首联.md) §4.3。

> **2026-11-23 P2 最小只读 CLI 补真实端点模式（默认关闭）**：`voucher-cli wps-p2-read` 增 `--transport https`，须同时满足 `--allow-real-endpoint` + `--token-file`（缺一退出 2），授权单端点主机非回环时才放行；凭据只从显式文件读一行，不读钥匙串/环境变量/argv，失败即停且错误里不含 token；`--check` 仍只预检（输出 `real_endpoint`/`credential_file`，不建状态库、不含 token）。本轮新增 `voucher-cli` 集成 4 例（守卫三连、空凭据、真实模式预检），`voucher-cli` 共 **10 例**；全量实跑 **540 passed / 0 failed / 0 ignored**；Rust 静态不变 `voucher-core` **520**（353 单元 + 167 集成），加壳 3 共 **523**，`services::wps` 口径 **154**；前端 **44 文件 227 用例**。**仍未发生任何真实调用**：WPS 端是否接受 Bearer/`Mcp-Session-Id`/`tools/call`/`DELETE` 仍待 A2 签署后执行一次；合成表已建成（`file_id` 掩码 `jcpy…MLtD`、`sheet_id=2`、60 行 3 列），首联 `fields` 只批 `["code"]` 以验证 Q1 服务端裁剪。P2 未授权执行、A2 未签、真实副本 C1–C6 未执行、S0/G1 未签。证据：[`docs/验收基线/证据/2026-11-23-P2-CLI/入口证据.md`](docs/验收基线/证据/2026-11-23-P2-CLI/入口证据.md) §8。

> **2026-11-23 数据页入账入口键盘与可访问性小修（verifier 复核后）**：移动卡片与桌面表格的「已入账」按钮补 `@keydown.space.stop`（Space 不再冒泡展开卡片；浏览器里 Space 仍按原生语义在 keyup 触发一次 click，只写一次库）；移动卡片 `div[role=button]` 与桌面表格行补 `aria-expanded`，展开态如实传给辅助技术，未重构成嵌套 button 结构（卡内必须可点入账，是已知取舍）。`src/pages/__tests__/DataPage.test.ts` 增至 7 例（新增：卡片按钮上按 Space 不展开、不调 `get_voucher`、整条链路只写一次库），前端实跑 **44 文件 227 用例**（原 226）。另修正 `docs/开发方案.md` B5 段动作数（两个 → 三个）与开发日志第一百一十六、一百一十七笔字段名（对齐 AGENTS 模板「关键提交（sha + 标题）」）。

> **2026-11-23 数据页补回「人工勾已入账」入口（B5 补收尾）**：B5 换 IA 时随台账页退役的 `set_ledger_booked` 动作已接回数据页——桌面「入账」列与移动卡片标记均可点（`@click.stop` 不误触行展开、逐行防连点、失败走 toast），「待入账」视图勾上后该行自然消失（过滤条件本就是 `!booked`）。前端新增 `src/pages/__tests__/DataPage.test.ts` 6 例（成功只发一次/取消/在途禁用/失败不误改/不展开/移动卡片），实跑 **44 文件 226 用例**（五门全过，build 1.03s）。Rust 计数不变（`voucher-core` 353 单元 + 167 集成 = **520**，加壳 3 共 **523**；本轮零 Rust 改动，`cargo clippy` 与全量 `cargo test` 交验收方，`cargo fmt --all --check` 通过）。挂账 #71（批次撤销 `undo_import_batch` 仍只有命令层）不变。

> **2026-11-23 前端死代码收敛（B5 补收尾）**：补删 `src/pages/LedgerPage.vue`（755 行）及其 10 例测试——路由自 B5 起已是 `/ledger` → `/reconcile`，该页生产无引用；`LedgerImportDialog`（6 例）保留，并把「导入台账 CSV」接进数据页「导入批次」视图（B5 换 IA 时漏接的入口）。前端实跑 **43 文件 220 用例**（原 44 文件 230 用例；五门 `typecheck` / `lint` / `format:check` / `test` / `build` 全过，build 971ms）。Rust 计数不变（`voucher-core` 353 单元 + 167 集成 = **520**，加壳 3 共 **523**；本轮未改 Rust，全量 `cargo test` 未复跑、交由验收方，`cargo fmt --all --check` 通过）。批次撤销仍只有命令层，界面入口见挂账 #71。

> **2026-11-23 P2 跨进程消费登记验收（上一轮）**：`voucher-core` 测试 **520 个**（353 单元 + 167 集成；`wps_p2` 32 → **36 例**——新增独立子进程验收：正常退出后重启、两进程并发、登记提交后发网前中断，均只允许一次消费且零重复请求），加壳 3 共 **523**；全量实跑 **536 passed / 0 failed / 0 ignored**（含新增 `voucher-cli` 集成 6 例：P2 最小只读 CLI 的端点白名单/离线默认/单次授权状态库；须在无文件沙箱的终端执行，见下方 OCR 说明）。前端 **44 文件 230 用例**。`services::wps` 过滤口径 **154**（wps 29 + pull 15 + 09 11 + index 8 + doc guard 7 + wps_w2 21 + credentials 8 + wps_w2_dev 15 + wps_http 4 + wps_p2 36）。**跨进程证据**：子进程复用测试二进制（`current_exe --exact … p2_child_worker`）调用同一 `P2ReadOnlyEntry` 与同一 `SqliteP2State`；A 完成读取 4 次请求后退出，B 同授权得 `P2_E_ALREADY_USED`、新增 0；并发两进程 `ok=1 used=1`、服务端总计 4；登记提交后由测试用同步点暂停、父进程确认 0 请求后终止，重启仍 `P2_E_ALREADY_USED`、请求仍 0，时点由文件标记固定而非 `sleep`。**真实端点/账号主体/服务端只读权限仍未取得**，P2 未授权执行、A2 不签；真实副本 C1–C6 未执行，S0/G1 未签。证据：[`docs/验收基线/证据/2026-11-22-P2-只读入口/入口证据.md`](docs/验收基线/证据/2026-11-22-P2-只读入口/入口证据.md) §3.4。

> **2026-11-23 P2 跨重启消费登记与越界脱敏（上一轮）**：`voucher-core` 测试 **516 个**（349 单元 + 167 集成；`wps_p2` 25 → **32 例**——新增重建入口/并发/发网前中断后不可再启动、状态库不可用零请求、超长窗口与超上限授权零请求、`DELETE` 失败记「未确认」且不追加请求、预算耗尽记「未发送」原因），加壳 3 共 **519**；全量实跑 **526 passed / 0 failed / 0 ignored**（须在无文件沙箱的终端执行，见下方 OCR 说明）。前端 **44 文件 230 用例**。`services::wps` 过滤口径 **150**（wps 29 + pull 15 + 09 11 + index 8 + doc guard 7 + wps_w2 21 + credentials 8 + wps_w2_dev 15 + wps_http 4 + wps_p2 32）。**跨重启失效**：`P2Grant` 增 `grant_id`；`P2Consumption` + 隔离 SQLite `SqliteP2State`（只存授权号/登记时间/掩码副本号/窗口终点），发网前 `INSERT` 原子登记，冲突即 `P2_E_ALREADY_USED`，状态库不可用即 `P2_E_STATE` 本地停止；窗口 ≤15 分钟、整场 ≤8 次在 `preflight` 强制。**越界脱敏**：响应出现未授权字段时只给固定提示 + 错误码 + 表号，不回显字段名/值。**终止状态**：`P2Termination` 区分「已确认 / 未确认（本地已关闭）/ 未发送（原因）」。**真实端点/账号主体/服务端只读权限仍未取得**，P2 未授权执行、A2 不签；真实副本 C1–C6 未执行，S0/G1 未签。证据：[`docs/验收基线/证据/2026-11-22-P2-只读入口/入口证据.md`](docs/验收基线/证据/2026-11-22-P2-只读入口/入口证据.md)。

> **2026-11-22 P2 只读入口第二轮（HTTPS 传输 + 读取预算修正，上一轮）**：`voucher-core` 测试 **503 个**（336 单元 + 167 集成；本轮 `wps_p2` 由 13 例增至 **19 例**——新增 6 例真实 HTTPS：本机测试服务、私 CA 信任、主机名不匹配、重定向目标 0 请求、响应越界整页拒绝、错误目标/写动作零连接），加壳 3 共 **506**；全量实跑 **513 项（511 passed / 2 failed / 0 ignored）**，失败两例为 `ocr_flow` 的 Apple Vision 识别波动，**在干净 HEAD `39421fb` 上同样复现**（日志 `/tmp/vs-p2fix-test.log`、`/tmp/vs-p2fix-test2.log`），与本轮改动无关。前端 **44 文件 230 用例**。`services::wps` 过滤口径 **137**（wps 29 + pull 15 + 09 11 + index 8 + doc guard 7 + wps_w2 21 + credentials 8 + wps_w2_dev 15 + wps_http 4 + wps_p2 19）。**读取预算**：单次逻辑读取最多真实发出 3 次 `dbsheet.list_records`（含会话重建后的重发与超时），修复旧实现「发出 4 次却记 3 次、可再发第 4 次」的偏差；第 4 条成功响应不再被取用。`HttpsP2Transport` 已接线（reqwest 阻塞客户端、`.no_proxy()`、严格证书/主机名校验、不跟随重定向、Bearer 与 `Mcp-Session-Id`），**真实端点/账号主体/服务端只读权限仍未取得**，故 P2 未授权执行、A2 不签；真实副本 C1–C6 未执行，S0/G1 未签。证据：[`docs/验收基线/证据/2026-11-22-P2-只读入口/入口证据.md`](docs/验收基线/证据/2026-11-22-P2-只读入口/入口证据.md)。

> **2026-11-22 W2/S0 本地交付收口（上一轮：结构化反馈 + 回环 HTTP 边界 + 400006/429 + C5 阻断入口）**：`voucher-core` 测试 **497 个**（330 单元 + 167 集成；本轮 `wps_p2` 新增 13 例——P2 只读入口默认关闭、预检零请求、范围派生、TLS/主机名/重定向与重试上限），加壳 3 共 **500**；全量实跑 **507 passed / 0 failed / 0 ignored**（`CENTER_SECRETS_FILE=/tmp/vs-headless-secrets.json RUST_TEST_THREADS=1`；静态 487 ≠ 实跑 494，差在 voucher-api/doctest 目标，**无跳过项**）。前端 **44 文件 230 用例**。`services::wps` 过滤口径 **131**（wps 29 + pull 15 + 09 11 + index 8 + doc guard 7 + wps_w2 21 + credentials 8 + wps_w2_dev 15 + wps_http 4 + wps_p2 13）。W2 批次返回 `authRequired`/`cooldownUntil`：400006 提示重新授权并保留待处理项、429 提示冷却截止时间；`pending`/`unknown` 不自动重发。回环 HTTP 传输仅接受 `127.0.0.1`/`localhost`/`[::1]`，无 TLS/DNS、不跟随重定向；**真实 WPS 端点仍未接线**，真实副本 C1–C6 未执行；S0/G1 未签。桌面 Mock 面板现有 8 个按钮（拉取/正常/超时/缺引用/授权失效 400006/限流 429/C5 阻断/恢复授权），7 个已有走查记录，C5 阻断以进程内入口测试留证（11 表扫描完成、`snapshot_verified=false`、`create=0`、`runs=0/intents=0/scans=11`，正常对照 `create=2`，见 [`docs/验收基线/证据/2026-11-22-C5-入口阻断/`](docs/验收基线/证据/2026-11-22-C5-入口阻断/)）；界面文案 + 临时库行级增量见 [`docs/验收基线/证据/2026-11-22-桌面走查-W2Mock/`](docs/验收基线/证据/2026-11-22-桌面走查-W2Mock/)，两条提示已目视。

> **2026-11-20 S0/W2 凭据链与本地证据（上一轮）**：`voucher-core` 测试 **473 个**（306 单元 + 167 集成；新增 `wps_credentials` 2、`wps::tests` 2、`wps_w2` 3、`wps_pull` 2、`log_retention` 1 等），加壳 3 共 **476**；全量实跑 **481 passed / 0 failed / 0 ignored**（跳过 2 个既有备份测试：`plaintext_options_no_encryption`、`cloud_parts_split_and_compose_roundtrip` 在沙箱内挂起，未计通过；静态 476 ≠ 实跑 481）。前端 **44 文件 230 用例**。`services::wps` 过滤口径 **107**（wps 29 + pull 15 + 09 11 + index 8 + doc guard 7 + wps_w2 21 + credentials 8 + dev 8）。W2 仍仅 Mock/临时库；真实 HTTP/生产写未启；S0 只做本地补证，未签。

> **2026-11-20 W2 桌面 Mock 入口 + S0 补证（启动隔离/凭据门）**：`voucher-core` 测试 **461 个**（294 单元 + 167 集成；本轮新增 12 例：`wps_pull` 2、`wps_w2` 4、`wps_credentials` 6），加壳 3 共 **464**；全量实跑 **471 passed / 0 failed / 0 ignored**（含 voucher-api 4 + doctest 1 + 桌面入口集成 1；静态 464 ≠ 实跑 471）。桌面入口另加 `voucher-api/tests/dev_w2_entry.rs` 1 例、`dev_w2` 启动/守卫 4 例。前端不变 **44 文件 230 用例**。`services::wps` 过滤口径 96（wps 26 + pull 13 + 09 11 + index 8 + doc guard 7 + wps_w2 17 + wps_credentials 6 + wps_w2_dev 8）。W2 桌面入口仅 Mock/临时库，真实 HTTP/生产写未启；S0 只做本地补证，未签。

## 功能特性

- **凭证管理**：编号自动生成（命名规则可配置）、状态流转（草稿/已复核/已导出/已归档）、快照历史、**收支方向**（原「标签」，只用来表达金额正负）
- **图片导入**：拖放/文件选择导入，MD5 自动去重，支持 BMP/JPEG/PNG/WebP，**并支持 PDF**（v0.25 起：多页 PDF 归档为一条附件，记页数，不做 OCR 增强）
- **OCR 识别**：**云端优先、本地兜底**（v0.23 起）——默认走 DeepSeek 视觉 API（`deepseek-flash`，一次调用同时返回全文与日期/对方/金额，准确度基准见 `docs/开发日志.md` 第六十七笔），识别时图片会上传（约 0.003 元/张）；云端不可用（未配 Key / 断网 / 报错）自动回退**本地 Apple Vision**（系统自带、零模型资产、完全离线）；设置页可锁定 `自动 / 云端 / 本地` 三档与云端协议（DeepSeek / 自建 HTTP，兼容旧接口）；识别结果缓存、批量补识别
- **智能记账**：OCR 识别结果结构化抽取（日期/金额/对方账户/商户），凭证表单一键填入
- **批量操作**：批量状态流转、批量编辑（日期/备注/收支方向）、批量导出 PDF
- **待处理驱动**（v0.18 起的主骨架；原称「收件箱驱动」）：首页是**待办流水线**——待识别 → 待核对 → 待打印 → 疑似重复，每项可就地推进；完成的项自动隐藏（不给已完成的事留版面），全部清零时给明确状态
- **全部凭证**（原称「资料库」）：**双行列表**（缩略图 + 摘要 + 金额 / 编号 · 日期 · 状态）+ **右侧编辑器**（选中即看大图与字段，**可就地改全部字段**：日期/摘要/对方/金额/状态/收支方向/备注，另含换图、重跑识别、裁剪、修改历史；双击/回车展开编辑区，⌘⇧E 切换展宽——原独立编辑页已并入此处）；精确筛选（状态 / **收支方向**（下拉：收入·支出·未标）/ 对方 / 金额区间 / 金额未填 / 对方未填 / OCR 状态 / 导出覆盖 / **未被佐证**）与五种排序；跨页选择集与"按筛选全选"；右栏在编辑器下方另有**「相关凭证」**与**「所属事项组」**两块（v0.22 起）
- **台账**（v0.25 起）：把「月度清单 → 建凭证 → 补附件 → 等入账」做成一条流水线。**v0.27 B5 起入口并入数据页的视图 chip（开发中，尚未发版）**（原独立「台账」页退役）：**待补凭证**（有台账号但没图）→ **待入账**（有图、可人工勾「已入账」）→ **导入批次**（每次导入是一批，可回看）。导入向导（选类型 收款 / 付款 / 计提 → 选 CSV → **自动预演** 新建 / 已存在 / 坏行 → 写入）在数据页「导入批次」视图的**「导入台账 CSV」**按钮打开；**批次撤销**仍只有命令层，新 IA 未接界面入口（挂账 #71；撤销只动「未补附件、未导出」的凭证并逐条说明保留原因）；台账号是幂等键，同一笔重复导只命中不重复建。绕过界面也能跑：`crates/voucher-cli` 提供 `import-ledger` 与 `list-pending`（`财务工作流/tools/ledger_to_voucher.py` 走的就是它）
- **凭证关联 / 佐证**（v0.22 起）：有些凭证要另一张凭证来解释——右栏可建立**附件说明**（一张说明凭证解释另一张）与**明细**（一次付款/一笔事项下的多张凭证，用**事项组**承载）两种关系，双向可见、可写说明文本、可解除；列表能筛「**未被佐证**」（既无关联也不在任何组）；新建页可建**无图凭证**（说明 / 备忘 / 原始凭证分割单）。纪律：**关联不带金额、不改任何合计口径**（关联 ≠ 抵消），组的合计只是派生展示值；每个关系建立/解除都写 append-only 审计，删凭证时关联级联清理但**留痕独立存活**
- **任务中心**（v0.21.0 起）：导入 / 关闭前备份 / 生成导出 / 批量识别 / 重建增强这些长任务统一进**内容区左下的全局浮层**，换页不丢、可最小化；有总量画进度条、后端不报进度走不确定态，完成后可一键「打开文件 / 开始核对」（右下角仍留给 toast 结果提示）
- **洞察**：统一时间窗控制器（本月 / 近 3 月 / 近 6 月 / 今年 / 自定义）驱动**月度趋势 + 状态分布 + 月度透视 + 导出覆盖度**；月度透视可按对方/收支方向切换，**每行精确下钻**到「全部凭证」；另有存储占用台账（原图体积走数据库，增强版体积按需计算）
- **图片增强**：扫描件七步增强管线（倾斜矫正→透视矫正→自动裁边→中值去噪→Unsharp 锐化→灰度→自适应二值化，单步失败自动跳过）；增强版用于 OCR 与 PDF 导出，带命中率统计与智能缓存清理
- **搜索**：按编号/摘要/备注/OCR 全文关键词检索
- **导出与打印**：**日报是主线**——表格正文/表头/金额/合计一律 **12pt（小四）**、列宽按 A1（金额**独立预留 40mm**，不再被「对方账户」压到）、摘要超宽折两行、按行高贪心分页（省纸紧凑 / 账本式双模板保留）、图片附录（竖票 3×3 / 横票 2×2）；日报**默认带「相关凭证清单」页**（按关系类型分类小计，页首印"金额不并入合计"）并另存**关联清单 CSV**，单次可关；表格式导出 Excel / CSV 与导出批次历史保留。**年度汇总报表与「单份 PDF」（含卡片风格 / 每行列数 / 预览）均已下线**——打印中心只剩日报一条主线
- **对账**：微信/支付宝账单 CSV 与库内凭证只读对账（同日同金额匹配），差异报告 CSV 导出
- **重复凭证核查**：同日期同金额的疑似重复进入「重复核查」队列，可并排比对（图片 + 字段差异高亮 + 置信度证据）后处置——**标记"不是重复"**（不改动凭证）或**确认重复并删除多余的一份**（二次确认 + 留痕，含被删凭证快照）。已处置的对不再重复提示。注：「重复」在本项目分三义——**内容重复**（同一张图，导入期 MD5 阻断）、**疑似重复**（同日期同金额，本入口）、**编号重复**（编号唯一约束）
- **备份与恢复**：本地 zip 备份（带版本清单，快照 + 内容寻址附件包增量）+ WebDAV 云备份（可选：age 加密，ChaCha20-Poly1305 AEAD 整文件密封）+ 定时/退出前自动备份；恢复走「待恢复文件 + 重启生效」流程，避免运行中替换数据库。**增强版图片属"可重建缓存"、不入备份包**——恢复后界面自动回退原图显示（后端不会下发指向不存在文件的增强路径），需要增强版时可在 设置 → 扫描矫正 一键重建
- **安全**：敏感凭据（WebDAV 密码、云端 OCR API Key）存入系统凭据库（macOS 钥匙串 / Keychain），密钥不明文落盘
- **稳定性**：数据目录文件锁 + 单实例保护，同一目录同一时刻只允许一个实例写入；异常退出后启动健康检查并提供恢复选项

## Center 网页版（v0.26 起；开发中，尚未发版）

同一个 Rust 内核，多了一层**网页服务端**：浏览器就是客户端，桌面版保留为离线增强版。

- **一条命令起服务**：`cargo run -p center-server`（默认 `127.0.0.1:8787`）；首次访问在浏览器里设置访问密码，之后用密码登录（`HttpOnly` 会话 Cookie，30 天）
- **多端**：手机/另一台电脑打开同一个地址即可（经 Tailscale 私网或反向代理），无需安装客户端；桌面版继续用 `npm run tauri dev`
- **上传**：网页里拖放即可（图片/PDF 批量），台账清单 CSV 也从同一个上传口进
- **台账字段**（v0.26）：每张凭证多出「入账要素」——单据类型 / 分类 / 结算账户 / 项目 / 票据类型 / 发票号 / 来源单据号 / 物料；面板实时显示**还缺哪些必填**，并给出按 V5 公式算出的**归档名**
- **按台账改名**：一键把附件改成 `260716-张帅-应付0001-某供应商` 这种台账口径的名字（先落盘后写库，重名自动 `-2` 避让）
- **会计投影**（v0.26）：按月把台账凭证折成借贷分录——收款按日+账户、付款按日+账户、计提按月+分类；导出 **KIS 引入 CSV / 抄单纸 xlsx / V5「凭证分录」JSON**，科目映射可在页面里改
- **三类汇总册**（v0.26 M3）：在「打印与导出」勾几张凭证 → **图片册**（一页一张原图 + 名称 + 信息，末尾清单）/ **发票册**（只收填了票据类型的，字段在上票面在下）/ **PDF 合并**（多份 PDF 原件按顺序并成一份 + 清单）
- **必填闸门**：台账凭证（或已填过入账要素的凭证）没补齐字段不允许复核——复核是"这张能入账"的承诺
- **台账对账**（v0.26 M4）：台账页「对账」页签选一份台账导出 CSV → 逐条比对凭证库，分六类差异：**缺凭证 / 金额不符 / 日期不符 / 对方不符 / 多余凭证 / 编号异常**（可设容差放过尾差），差异表一键导 CSV。只读比对，不改凭证也不改台账；主键是 `ledger_ref`（`cash:`/`doc:` 前缀与裸编号都认）
- **冷备演练**（v0.26 M5）：`bash deploy/drill-coldstandby.sh` —— 备份 → 还原到空目录 → 起第二个实例 → 比对两边凭证条数与金额，全程只用临时目录，不碰真实账本；接管机器上可用 `voucher-cli restore --zip <备份.zip> --target <空目录>`

**部署（用户 2026-11-18 定稿）**：公司那台 Mac 常开做主机（`launchd` 守护 + `tailscale serve` 提供私网 HTTPS），i3 PC 作冷备；**不买云服务器**，成本只有云端 OCR（约 0.003 元/张）。

- 装成常驻服务：`bash deploy/install-macos.sh`（`--no-build` 用已构建二进制、`--uninstall` 卸载保留数据）
- 冷备接管：新机器上 `voucher-cli restore --zip <备份.zip> --target <数据目录>` → 用同一个二进制起服务；接管目标 ≤15 分钟

## 代码审计

- **审计指南**：[`docs/审计指南.md`](docs/审计指南.md)（15 个维度 / 四级严重度 / 闭环三档 L1–L3 / 报告模板）
- **一键自动检查**：`bash scripts/audit.sh`（门禁复用 + 用例数三处口径实测 + 危险模式计数 + 契约守卫存在性 + 文档纪律抽查；`--fast` 跳过门禁，`--strict` 可接 CI）
- **最近一次审计**：[`docs/voucher-studio-audit-2026-09-27.md`](docs/voucher-studio-audit-2026-09-27.md)（P0 0 / P1 6 全部修完 / P2 3 已挂账）

## 文档

- **开发方案**：[`docs/开发方案.md`](docs/开发方案.md)（阶段计划 / 评审门 / 风险登记）
- **UI 设计正本**：[`docs/UI-设计规范-Loon.md`](docs/UI-设计规范-Loon.md)（2026-11-04 起：渐变画布 / 玻璃内容卡 / 令牌 / 无障碍回退）；历史正本 [`docs/UI-设计规范-macOS27.md`](docs/UI-设计规范-macOS27.md)
- **开发日志（唯一过程证据源）**：[`docs/开发日志.md`](docs/开发日志.md)（批次七字段记录 + 文末**挂账表**）
- **术语表（非程序员版）**：[`docs/术语表.md`](docs/术语表.md)（当字典查，每次问到新词就补一条）· 配套 [`docs/学习路线-非程序员版.md`](docs/学习路线-非程序员版.md)
- **方案（v0.22 打印日报与凭证关联）**：[`docs/方案-v2-打印日报与凭证关联（定稿）.md`](docs/方案-v2-打印日报与凭证关联（定稿）.md)（执行契约）· 分析底稿 [`docs/方案-打印日报改进-分析底稿.md`](docs/方案-打印日报改进-分析底稿.md)· 关联详版 [`docs/方案-凭证关联与佐证.md`](docs/方案-凭证关联与佐证.md)· 外部实践 [`docs/调研-凭证关系-外部实践.md`](docs/调研-凭证关系-外部实践.md)
- **真机走查**：[`docs/走查清单.md`](docs/走查清单.md) + [`docs/验收基线/`](docs/验收基线/)（清单1 非 GUI 测试映射 / `清单2-真机验证45项.md` **实际 50 项**（文件名沿旧，2026-09-27 增补 #46–50）/ `清单3-GUI场景32个.md` **实际 36 项**（文件名沿旧，+33 与 U 节 34–36））

## 技术栈

| 层 | 技术 |
|---|---|
| 业务核心 | Rust（`crates/voucher-core`，不依赖 Tauri，可独立测试） |
| 桌面壳 | Tauri 2（单实例、对话框、日志、opener 插件）；命令实现已下沉到 `crates/voucher-api` |
| 网页服务 | `crates/center-server`：hyper 1 + tokio，认证 / RPC / 上传 / 附件 / SPA 托管（单用户密码会话，PBKDF2-SHA256） |
| 本地 OCR | ort 2.0 RC（`=2.0.0-rc.13`；上游暂无 2.x 正式版，故精确 pin，见 `crates/voucher-core/Cargo.toml` 注释）+ RapidOCR PP-OCRv5 模型 |
| 数据库 | SQLite（rusqlite，显式事务 UoW 模式） |
| 前端 | Vue 3 + TypeScript + Vite + Pinia + Vue Router |
| UI | **v0.27 起工作台重构（B1–B5 已落地；开发中，尚未发版——当前发布版本 0.25.0）**：网页端为唯一设计基准，顶部 52px Header + 四个固定页签「工作台 / 数据 / 对账 / 会计投影」，打印导出、设置、备份、重复核查、帮助全部改为弹层；桌面端改用系统原生标题栏，不再为窗口按钮预留内边距；暖色语义令牌（Claude 系强调色 + 中文衬线显示字），正文 15px、输入 16px、字号三档可调；旧侧栏 / 页头 / 底部 tab 已退役。数据页已接真实数据与行内编辑、新建条目强制凭证（B2a/B2b，2026-11-18）；工作台 KPI/图表/待办、对账与五弹层已落地（B3/B4），旧页清场、PWA 壳层与视觉基线已完成（B5）。**自 v0.17.0 起仅支持 macOS**，v0.26 起同一内核也以网页服务形态运行（手机/平板走浏览器）。设计口径见 [`docs/开发方案.md`](docs/开发方案.md) §5.9（B5 落正本；当前取值以 `src/styles/tokens.css` 为准） |

## 环境要求

仅支持 **macOS**（Apple Silicon / arm64）。开发环境：

- Rust 1.75+（stable）+ Xcode Command Line Tools
- Node.js 20+
- **OpenCV 4.10（必须，编译前置；原文为 `brew install opencv`，自 v0.15.1 起改为源码编译固定 4.10）**：`opencv` crate 按**本机已装头文件**生成绑定，代码依赖 4.10 的 API 面（`imgproc::bounding_rect`/`contour_area`/`approx_poly_dp`、`ImreadModes::IMREAD_COLOR`、`cvt_color`/`HoughLines` 参数个数），而 brew 现供 5.x（`opencv@4` 亦已 4.14）上述位置全部变更，直接编译报错。需源码编译 4.10 并让 `PKG_CONFIG_PATH` 指向其 `lib/pkgconfig`；**机器上不要保留 brew 的 opencv**（crate 的 pkg-config 探测优先 `opencv5.pc`，会静默改用 5.x 头文件）
- `libclang`：`opencv` crate 的 bindgen 需要；若 `DYLD_*` 被系统剥离导致找不到，可在 `$HOME/lib` 放符号链接指向 Command Line Tools 的 `libclang.dylib`
- Linux 侧仅用于 CI 的快速门禁（`libopencv-dev` + `clang`/`libclang-dev`），不产出桌面产物

## 开发

```bash
npm install        # 安装前端依赖
npm run tauri dev  # 启动开发模式（自动编译 Rust + 前端热更新）
```

常用命令：

```bash
npm run typecheck          # 前端类型检查
npm run format             # 全量排版（Prettier；提交前跑一次）
npm run format:check       # 只检查排版是否合规（门禁用，CI 同款）
npm run test               # 前端单元测试（Vitest）
cargo test --workspace     # 后端测试（voucher-core：376 单元 + 175 集成，共 551 用例；加 src-tauri 3 用例共 554；另 voucher-cli 集成用例；静态计数）
cargo test --workspace --no-fail-fast -- --include-ignored  # 与 CI 同口径（含 #[ignore] 真跑；单目标失败不中止其余）
cargo clippy --workspace --all-targets  # Lint
cargo fmt --all            # 格式化
```

构建缓存长期会长得很大（682 个依赖 × debug/release/test 三套 profile × 15 个测试目标，`target/` 曾涨到 39 GB）。
定期清理用仓库脚本（默认只预览，`--apply` 才删；只动 `target/`，不碰源码与 `.git`）：

```bash
bash scripts/clean-build.sh                  # 预览体积与将清理的内容
bash scripts/clean-build.sh --apply          # 清 dev 档 + 旧版本发布件（保留 release 依赖缓存）
bash scripts/clean-build.sh --apply --all    # 全量 cargo clean（下次构建全量重编）
bash scripts/clean-build.sh --apply --sweep  # 叠跑 cargo sweep（需 cargo install cargo-sweep）
```

`Cargo.toml` 已对 dev 档减重（`debug = "line-tables-only"` + 第三方依赖 `debug = false`），
`target/debug` 通常只有原来的一半以下。

### 视觉回归（改样式后跑一遍）

改 CSS / 版式后，用它确认没有肉眼没注意到的回归（Playwright + `__TAURI_INTERNALS__` mock，无需真机）：

```bash
npm run dev                          # 先起前端（脚本也会尝试自起 vite:5173）
node scripts/visual/capture.mjs      # 重截 6 个页面 → docs/验收基线/视觉基线/
node scripts/visual/compare.mjs      # 与基线逐像素对比（阈值 0.5%，超了非零退出）
```

基线在 `docs/验收基线/视觉基线/`；每张图对应一个页面（资料库列表/编辑器、洞察、打印中心、设置、帮助）。
**确认要改版式时**才跑 `capture.mjs` 更新基线，并把副本归档到 `docs/验收基线/证据/<日期>-G3-<版本>/`（见该目录 README）。

## 构建

```bash
npm run tauri build    # 产出 .app / .dmg（bundle.targets 已钉为 app+dmg）
bash package-macos.sh  # macOS 必需收尾：OpenCV dylib 收进包内 + 重签，产出 target/release-artifacts/*.dmg
```

macOS 必须走 `package-macos.sh`：Rust 侧链接的 OpenCV dylib 是**绝对路径**引用，`tauri build` 直出的 `.app` 只能在装有同一路径 OpenCV 的机器上启动（拷给别人即 dyld `Library not loaded`）。脚本把 6 个 dylib 收进 `Contents/Frameworks`、引用改为 `@rpath`、补 rpath 并 ad-hoc 重签名。无 Developer ID 证书时签名只能是 ad-hoc、**无法公证**，分发对象首次打开需右键→打开（或 `xattr -dr com.apple.quarantine`）。

安装包自动携带本地 OCR 模型（约 16 MB），首次启动复制到用户数据目录。

### 安装（分发给别人时）

没有 Apple Developer ID，安装包只做了 **ad-hoc 签名、无法公证**，所以第一次打开会被 Gatekeeper 拦下。
给同事或另一台机器装，按下面任一种方式打开：

1. **右键 → 打开**：双击 dmg → 把 `Voucher Studio.app` 拖进「应用程序」→ 在「应用程序」里**右键点图标 → 打开** → 弹窗再点「打开」（只需一次）。
2. **命令行去隔离标记**（适合脚本/远程分发）：

   ```bash
   xattr -dr com.apple.quarantine "/Applications/Voucher Studio.app"
   open "/Applications/Voucher Studio.app"
   ```

打不开时的自检：`codesign --verify --strict "/Applications/Voucher Studio.app"`（ad-hoc 也应通过）、
`otool -L "/Applications/Voucher Studio.app/Contents/MacOS/voucher-studio" | grep -v @rpath`（**不应**出现
`/Users/…` 这类本机绝对路径——出现说明拿的是 v0.22.2 及更早的坏包，请换 v0.22.3+）。

### 发版（一条命令）

```bash
# 前置：先在本版开发日志写条目 + 写 docs/release-notes/<版本>.md（脚本会强制检查）
bash scripts/release.sh 0.22.3            # 预览：只打印步骤，不改任何文件
bash scripts/release.sh 0.22.3 --apply    # 真正发版
```

`scripts/release.sh` 把发版全流程固化成 7 步：预检（干净/在 main/已推送/**锁文件一致性**——`cargo metadata --locked`
+ `npm ci --dry-run`，防依赖漂移/日志与说明就绪）→ 门禁 →
**过期产物清理**（`clean-build.sh --apply`：清 dev 档 + 旧版本 dmg / release-artifacts）→ 版本号 bump 三处 →
`tauri build` + **`package-macos.sh`**（OpenCV dylib 收进包内、改 @rpath、重签、出 dmg）→
**产物自包含硬校验**（主二进制不得引用本机绝对路径 / `Contents/Frameworks` 必须含 6 个 dylib / 引用必须 @rpath /
包内版本号一致 / sha256 校验）→ 提交 + tag + `gh release create` + 下载回来复核（`shasum -c` + `cmp`）+
收尾清理（`--keep-build` 可跳过）。

> 2026-10-18 教训：手工发版连续三次漏了 `package-macos.sh`，发布件在别的机器上会 dyld `Library not loaded`。
> 现在第 6 步会**直接拒绝**这种产物，从流程上堵死。

### 自动更新（维护者）

应用内置「设置 → 通用 → 软件更新 → 检查更新」；打包版会读 GitHub Release 的
`latest.json`，比对版本后自动下载、验签、替换并重启。**开发模式（`tauri dev`）不检查更新**。

维护者需要知道的三件事：

1. **签名私钥**（只在本机 / CI secret，绝不入库）：`~/.tauri/voucher-studio-updater.key`
   ——丢失或泄露都要立刻重新生成并替换 `src-tauri/tauri.conf.json` 里的 `plugins.updater.pubkey`。
   CI 发版时通过仓库 secret `TAURI_SIGNING_PRIVATE_KEY`（或 `TAURI_SIGNING_PRIVATE_KEY_PATH`）注入。
2. **更新包不在 bundler 里生成**：`bundle.createUpdaterArtifacts` 故意设为 `false`。更新包必须由
   `scripts/release.sh` 在 `package-macos.sh` **之后**重新打（`.app.tar.gz` + `.sig`），
   否则会把没修好 dylib 的旧包当成更新发出去（与 2026-10-18 发布缺陷同一根因）。
3. **`latest.json`** 由发版脚本生成并随 Release 上传：`version` / `notes` / `pub_date` /
   `platforms.darwin-aarch64.{signature,url}`。签名与包必须成对更新——只传包不传新 `latest.json`，
   客户端会因验签失败拒绝更新。

## 目录结构

```
├── crates/voucher-core/   # 业务核心（models / db / services / utils）
├── crates/voucher-api/    # 命令层：96 条命令的唯一实现（桌面与网页共用）
├── crates/center-server/  # 网页服务端（认证 / RPC / 上传 / 附件 / SPA）
├── src-tauri/             # Tauri 壳（commands / platform）
├── src/                   # Vue 3 前端（pages / components / stores）
├── assets/ocr-models/     # 本地 OCR 模型（PP-OCRv5）
├── spikes/ocr-poc/        # G1 Spike B 验证工程（24 张合成凭证，报告见 report.json）
└── docs/开发方案.md        # 开发方案（阶段计划 / 评审门 / 风险登记）
```

## 数据存储

- 桌面版（macOS）：`~/Library/Application Support/VoucherStudio`
- 网页服务端：`CENTER_DATA_DIR` 指定的目录（默认同样是上面的路径）；密钥改用 `config/secrets.json`（0600），不再依赖 Keychain

包含：`voucher.db`（SQLite）、`attachments/`（附件图片）、`backups/`（备份）、`exports/`（导出 PDF）、`models/`（OCR 模型）、`logs/`。

## 许可证

[MIT](LICENSE)
