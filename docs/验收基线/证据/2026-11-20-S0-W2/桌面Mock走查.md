# 桌面开发入口 Mock 走查（2026-11-20）

> 范围：本地 debug 桌面 + `transport=Mock` + 临时 SQLite + 合成数据。**未连接真实 WPS、未发 HTTP、未操作生产库、未部署。** 本记录不构成 S0/G1 通过或 G0 验收结论。

## 1. 启动与隔离

```bash
cd /Users/apple/Downloads/开发/voucher-studio
export VOUCHER_DATA_DIR="$(mktemp -d "${TMPDIR:-/tmp}/voucher-w2-mock.XXXXXX")"
test -d "$VOUCHER_DATA_DIR"
export VOUCHER_W2_MOCK=1
. "$HOME/.cargo/env"
export PKG_CONFIG_PATH="$HOME/.local/opencv-4.10/lib/pkgconfig"
npm run tauri dev
```

约束：

- 启动时先校验 `VOUCHER_DATA_DIR`：必须显式、已存在、规范化后位于系统临时根内、且不等于临时根；外指符号链接、空路径、缺路径都在**创建目录/待恢复/打开库之前**退出，不回落默认目录。
- 仅 debug 构建 + `VOUCHER_W2_MOCK=1` + 上述临时目录时入口可用；普通运行、生产目录、网页端会收到 `dev_only`。
- 命令只接受 `action=pull|write|auth_recover`、`scenario=normal|timeout_second|missing_reference|auth_expired|rate_limited`，不接受 file_id 或数据目录；界面无法把目标换成真实文件/生产库。
- 桌面数据页 →「开发本地模拟同步（W2 Mock）」面板。

### 1.1 启动隔离实测

无效路径（缺失目录）不建目录、不回落，直接退出：

```bash
python3 - <<'PY'
import os, subprocess, tempfile
base=tempfile.mkdtemp(prefix="w2-bad-start.")
missing=os.path.join(base,"missing")
env=dict(os.environ, VOUCHER_W2_MOCK="1", VOUCHER_DATA_DIR=missing)
p=subprocess.run(["./target/debug/voucher-studio"], env=env, capture_output=True, text=True, timeout=20)
print("rc", p.returncode, "exists", os.path.exists(missing))
print(p.stderr.strip())
PY
# 实测：rc 2；W2 Mock 启动隔离校验失败：VOUCHER_DATA_DIR 不可解析…；exists False
```

有效路径（第 1 节 `mktemp` 命令）启动日志应出现：
`W2 Mock 启动隔离校验通过：/private/var/.../voucher-w2-mock.XXXXXX`，随后才创建目录/打开库。

## 2. 最短走查步骤

1. 点「拉取 11 表（只读）」：应显示 `mode=pull`、数据目录、11 表完整；不回写业务数据。
2. 点「模拟创建（正常）」：逐条 2 笔均为「成功」；「保存关联」显示「已保存（未认领）」；「正式入账」均为「未入账」。
3. 点「模拟创建（第二笔超时）」：第 1 笔成功、第 2 笔结果不明、第 3 笔未尝试；「本批已按停止规则终止」。
4. 点「模拟创建（缺引用）」：单笔整批拒绝，0 写入。
5. 核对临时库：

```bash
DB="$VOUCHER_DATA_DIR/voucher.db"
sqlite3 "$DB" "select sequence,state,coalesce(remote_record_id,'') from wps_write_intents order by sequence;"
sqlite3 "$DB" "select sheet_id,record_id,state from wps_links order by id;"
sqlite3 "$DB" "select id,status,pid from wps_sync_runs order by id;"
```

正常/超时后预期：

- `wps_write_intents`：正常 2 条 `linked`；超时 2 条（`linked`、`unknown`），第 3 笔无意图。
- `wps_links`：正常/超时成功笔为 `unclaimed`，不自动 `posted`。
- 重启应用后再次点击同一场景：幂等键已存在，整批拒绝；不重发。

## 2.1 人工点击实测（2026-11-22 完成）

**执行方式**：开发模式启动桌面版（debug + `VOUCHER_W2_MOCK=1` + 临时库），由**用户本人**在数据页点击面板按钮；助手负责驱动窗口、截图与读取临时库。助手侧无法模拟点击——`osascript` 对 System Events 仍返回 `execution error: … 发生权限违例。 (-10004)`（Apple Events/Accessibility 均被 macOS 拒绝），故点击动作由人完成，截图为助手在点击后立即抓取。

**本轮临时库**（均在系统临时根内，未触生产目录）：

| 临时库 | 用途 |
|---|---|
| `/private/var/folders/.../voucher-w2-mock.B4oelv` | 第 1 轮：拉取 11 表、正常、第二笔超时、缺引用 |
| `/private/var/folders/.../voucher-w2-mock.Fazdb5` | 第 2 轮补拍：正常、第二笔超时（另捕获重复运行拒绝） |

**复跑命令**（`--config` 只覆盖本次启动的 `devUrl`，不改仓库文件；临时目录必须建在 `$TMPDIR` 下，建在 `/tmp` 会被启动校验以 `不在系统临时根内` 拒绝并 `exit 2`）：

```bash
cd /Users/apple/Downloads/开发/voucher-studio
export VOUCHER_DATA_DIR="$(mktemp -d "${TMPDIR:-/tmp}/voucher-w2-mock.XXXXXX")"
test -d "$VOUCHER_DATA_DIR"
export VOUCHER_W2_MOCK=1
. "$HOME/.cargo/env"
export PKG_CONFIG_PATH="$HOME/.local/opencv-4.10/lib/pkgconfig"
npm run tauri dev -- --config '{"build":{"devUrl":"http://localhost:5173/#/data"}}'
```

**实测结果（界面文案 + 库增量）**：

| 步骤 | 界面实测 | 临时库增量 |
|---|---|---|
| 拉取 11 表（只读） | 表头 `mode=pull · 数据目录 … · 拉取 完整（11 表）`；下方业务列表仍为「当前视图没有条目」 | `wps_scans` 44 行 = 11 表 × 4 页，`complete=1`、`error` 全空；无业务写入 |
| 模拟创建（正常） | 两行均「成功」「已保存（未认领）」「未入账」，错误列 `—` | `wps_sync_runs#1 status=ok scanned=2 created=2 errors=0`；intent 2 条 `linked`；`wps_links` +2、state=`unclaimed` |
| 模拟创建（第二笔超时） | 1「成功」/ 2「结果不明」+ `WPS_E_TIMEOUT: WPS 请求超时` / 3「未尝试」；表尾「本批已按停止规则终止。」见 [`02-第二笔超时-批次结果.jpg`](../2026-11-22-桌面走查-W2Mock/02-第二笔超时-批次结果.jpg) | `wps_sync_runs#2 status=partial scanned=3 created=1 conflicts=1 errors=1`；intent `AR-DEV-T1=linked`、`AR-DEV-T2=unknown`；T3 无 intent |
| 模拟创建（缺引用） | 单行「整批拒绝」「远端记录 —」「未保存关联」「未入账」+ `WPS_E_REF_STALE: 引用原记录不存在或已失效: 表 10 / dev-10-missing`；表尾「本批已按停止规则终止。」见 [`03-缺引用-整批拒绝.jpg`](../2026-11-22-桌面走查-W2Mock/03-缺引用-整批拒绝.jpg) | **零本地业务写入**：无新 run / intent / link；仅新增 11 条 pull 扫描（1 页/表，`complete=1`） |
| 重复点击「正常」（附加） | 两行「整批拒绝」+ `WPS_E_IDEMPOTENCY_STATE: 幂等键已存在，拒绝自动重试`；表尾「终止。」见 [`04-重复运行-幂等键拒绝.jpg`](../2026-11-22-桌面走查-W2Mock/04-重复运行-幂等键拒绝.jpg)（窗口左缘越界，幂等键列未入镜，行号 01/02 与库内键一一对应） | 无新 run / intent / link；`wps_idempotency_keys` 仍只有 `AR-DEV-001`/`AR-DEV-002` |

**重启后不自动重发（实测）**：第 2 轮启动复用的是仍含 `partial` 批次的临时库，重启后 `wps_sync_runs#2` 仍为 `partial`、`AR-DEV-T2` 仍为 `unknown`，未出现任何自动 `create` 调用或新 run 行——与「结果不明项只保留、不自动重发」的约定一致。

**仍未覆盖**：`authRequired`/`cooldownUntil` 两条提示的**界面渲染**本轮没有触发——dev 面板只有 pull / normal / timeout / missing_reference 四个入口，没有 400006/429 场景按钮，面板也无法注入这两种错误；这两条的界面显示仍只有单测（`services::wps_w2` 断言 `auth_required=true`、`cooldown_until` 前缀）与 DTO 核对，须记为缺口（见 §5），不得写成已目视验收。

截图目录：[`docs/验收基线/证据/2026-11-22-桌面走查-W2Mock/`](../2026-11-22-桌面走查-W2Mock/)（`01-数据页-W2Mock面板.jpg` 为点击前的面板状态）。

## 2.2 上轮人工点击记录（2026-11-20）

2026-11-20 桌面开发版已启动成功：日志出现 `W2 Mock 启动隔离校验通过：/private/var/.../voucher-w2-mock.oyWR0R`，窗口已显示工作台（截图：[`桌面Mock窗口截图.png`](./桌面Mock窗口截图.png)）。但当前执行环境的 `osascript` 访问 Tauri 原生窗口被 macOS 拒绝（返回“osascript 不允许辅助访问”），尝试将窗口置前并使用 System Events 点击均失败；**未完成人工点击目视，保留“待人工验收”**。截图只证明窗口启动，不当作同步流程已验收证据。可复跑命令见第 1 节；若在目标 Mac 上执行，按第 2 节顺序点击并记录：

- 每步的 `mode`、`dataDir`、11 表完整性；
- `wps_write_intents` 与 `wps_links` 的增量；
- 重启应用后再次点击同场景，确认幂等键存在、整批拒绝、不重发。

## 2.3 400006 / 429 场景与目视验收（2026-11-22 完成）

2026-11-22 给面板补了三个入口：「模拟创建（授权失效 400006）」「模拟创建（限流 429）」「恢复授权（本地合成）」。三者都走同一个 `DevW2Transport` 与 `W2Sync::execute`：错误由 transport 返回 `WpsError::AuthExpired`/`WpsError::RateLimited`，不是前端硬造；面板另外显示本次调用的 Mock 请求计数（`list`/`create` 与 create 幂等键），用来证明「零请求」。

两个前置阻断在**拉取之前**短路：合成授权标记（`wps_state.dev_w2_auth_expired=1`）存在、或 `wps.rate_limit_until` 仍在冷却期时，本次调用直接返回整批拒绝并填 `authRequired`/`cooldownUntil`，transport 请求数为 0，也不产生 run 行。

**本轮临时库**（各自独立，均在系统临时根内）：

| 临时库 | 用途 |
|---|---|
| `/private/var/folders/.../voucher-w2-mock.3qbkJ6` | 400006 全流程：超时造 unknown → 授权失效 → 未恢复再点 → 恢复 → 正常 |
| `/private/var/folders/.../voucher-w2-mock.2avUi6` | 单独捕获 400006 批次本身的界面 |
| `/private/var/folders/.../voucher-w2-mock.MJBfjX` | 429：限流批次 → 冷却期内再点 |

**实测 1：400006（授权失效）**

| 步骤 | 界面实测 | 临时库增量 |
|---|---|---|
| 模拟创建（第二笔超时） | 1「成功」/ 2「结果不明」+ `WPS_E_TIMEOUT` / 3「未尝试」 | run#1 `partial`；`AR-DEV-T1=linked dev-03-1`、`AR-DEV-T2=unknown`（作为后续「已有 unknown 记录」） |
| 模拟创建（授权失效 400006） | 红条「WPS 授权已失效：请重新授权。当前待处理项已保留，恢复后不会自动重发。」；①`AR-DEV-A1`「结果不明」+ `WPS_E_AUTH: 授权失效，已停止批量写入；该条保留待处理`；②`AR-DEV-A2`「未尝试」；表尾「本批已按停止规则终止。」；表头 `Mock 请求 12（list 11 / create 1）· create 键 AR-DEV-A1`。见 [`05-授权失效400006-批次结果.jpg`](../2026-11-22-桌面走查-W2Mock/05-授权失效400006-批次结果.jpg) | run#2 `partial scanned=2 created=0 conflicts=1 errors=1`；intent 仅 `AR-DEV-A1=unknown`（A2 无 intent）；`wps_idempotency_keys.AR-DEV-A1=pending`；`wps_state.dev_w2_auth_expired=1` |
| 模拟创建（正常）——授权未恢复 | 同一红条；表头 `拉取 已跳过（前置阻断，零请求） · Mock 请求 0（list 0 / create 0）`；两行「整批拒绝」+ `WPS_E_AUTH`；见 [`06-授权未恢复-零请求被挡.jpg`](../2026-11-22-桌面走查-W2Mock/06-授权未恢复-零请求被挡.jpg) | **无新 run / intent / link**；`wps_scans` 与键状态不变（旧 `AR-DEV-T2` 仍 `unknown`） |
| 恢复授权（本地合成） | 面板回到可写状态（`mode=auth_recover`） | `wps_state.dev_w2_auth_expired=0`；无 transport 请求 |
| 模拟创建（正常）——已恢复 | 两行「成功」「已保存（未认领）」「未入账」；表头 `拉取 完整（11 表） · Mock 请求 13（list 11 / create 2）· create 键 AR-DEV-001、AR-DEV-002`；见 [`07-恢复授权后-新批次未重发旧键.jpg`](../2026-11-22-桌面走查-W2Mock/07-恢复授权后-新批次未重发旧键.jpg) | run#3 `ok scanned=2 created=2`；新增 intent 仅 `AR-DEV-001`/`AR-DEV-002`（`linked`）；`AR-DEV-T2` 仍 `unknown`、`AR-DEV-A1` 仍 `unknown` 且键仍 `pending`——**旧记录 create 增量为 0、未被标成成功** |

**实测 2：429（限流）**（独立库，冷却状态不跨场景）

| 步骤 | 界面实测 | 临时库增量 |
|---|---|---|
| 模拟创建（限流 429） | 红条「WPS 限流：冷却至 2026-11-22T11:17:48Z，冷却结束前不会发起新批次。」；①`AR-DEV-R1`「结果不明」+ `WPS_E_RATE: 限流，已停止批量写入并冷却至 2026-11-22T11:17:48Z`；②「未尝试」；表头 `Mock 请求 12（list 11 / create 1）· create 键 AR-DEV-R1`；见 [`08-限流429-冷却截止.jpg`](../2026-11-22-桌面走查-W2Mock/08-限流429-冷却截止.jpg) | run#1 `partial scanned=2 created=0 conflicts=1 errors=1`；`AR-DEV-R1=unknown`、键 `pending`；`wps.rate_limit_until=2026-11-22T11:17:48Z` |
| 冷却期内再点（正常） | 红条仍是同一截止时刻；表头 `拉取 已跳过（前置阻断，零请求） · Mock 请求 0（list 0 / create 0）`；两行「整批拒绝」+ 同一 `WPS_E_RATE` 文案；见 [`09-冷却期内再次点击-零请求.jpg`](../2026-11-22-桌面走查-W2Mock/09-冷却期内再次点击-零请求.jpg) | `wps_sync_runs` 仍 1、`wps_write_intents` 仍 1、`wps_links` 0、`wps_scans` 仍 11——**零新请求、零新 run** |

**截止时刻三处一致**：页面红条、行内 `WPS_E_RATE` 文案、库内 `wps.rate_limit_until` 均为 `2026-11-22T11:17:48Z`（RFC3339、秒级、UTC），与批次返回的 `cooldownUntil` 同值。

**库核对命令**：

```bash
DB="$VOUCHER_DATA_DIR/voucher.db"
sqlite3 "$DB" "select id,status,scanned,created,conflicts,errors from wps_sync_runs order by id;"
sqlite3 "$DB" "select business_key,state,coalesce(error,'') from wps_write_intents order by id;"
sqlite3 "$DB" "select idempotency_key,local_status from wps_idempotency_keys order by rowid;"
sqlite3 "$DB" "select key,value from wps_state where key in ('dev_w2_auth_expired','wps.rate_limit_until');"
```

**边界**：以上是**合成** 400006/429（transport 直接返回错误码），不是真实 WPS 演练；「恢复授权」是本地清标记，不是真实 OAuth/重授权流程。`authRequired`/`cooldownUntil` 两条提示已完成目视验收；重授权 UI、队列续传、红点仍未实现。

## 3. 自动化证据

```bash
cargo test -p voucher-core --lib --offline services::wps_w2_dev
# 13 passed / 0 failed（2026-11-22 新增 5 例：auth 停工保 pending、auth 未恢复零请求、恢复后新批次不重发旧键、429 冷却与返回值一致、429 二次点击零请求）

cargo test -p voucher-api --test dev_w2_entry --offline
# 1 passed / 0 failed：守卫 → pull → 400006 → 未恢复零请求 → auth_recover → 正常 → 超时 → 缺引用 → 429 → 冷却期零请求

cargo test -p voucher-api --lib --offline dev_w2::tests
# 4 passed：启动校验（空路径/临时根/外指符号链接/有效目录）；守卫拒绝；status 文本稳定

cargo test -p voucher-core --lib --offline services::wps_http
# 4 passed（2026-11-22 新增）：回环白名单、Bearer 注入、缺/过期凭据零请求、不跟随重定向、错误体不回显
```

2026-11-22 复跑：`services::wps_w2` 34 passed（含 `wps_w2_dev` 13）、`dev_w2_entry` 1 passed、`dev_w2::tests` 4 passed；全量（`CENTER_SECRETS_FILE=/tmp/vs-headless-secrets.json RUST_TEST_THREADS=1`）`cargo test --workspace --no-fail-fast --offline -- --include-ignored` → **492 passed / 0 failed / 0 ignored**；WPS 家族过滤口径 116（wps 29 + pull 15 + 09 11 + index 8 + doc guard 7 + wps_w2 21 + wps_w2_dev 13 + credentials 8 + wps_http 4）。

## 4. C1–C6 本地 Mock 覆盖

| 用例 | 本地 Mock 已覆盖 | 缺口 |
|---|---|---|
| C1 日期 | `classify_date` 覆盖 2026-10-31 旧、2026-11-01 新、空/非法 NeedsReview；`classify_period` 覆盖 09 YYYY-MM 与非法值 | 06 到账日/02/03/04/11 各表真实字段值、`2026-10-31` 到账但 11 月补录等业务数据未造 |
| C2 旧资料 | `preflight` 对 01/08/10 引用做存在性校验；缺引用场景整批拒绝；资料零新增（引用只读） | 同名不同 ID、分界日前失效资料、真实关联目标未测 |
| C3 拉取恢复 | `wps_pull` staged：不完整/坏页/游标回退保留旧索引并阻断写；空页假 `has_more` 有测试；`dev_pull` 11 表全历史 | 扫描中插入/删除/更新、同键字段冲突、总数不变场景未测；真实远端重扫未执行 |
| C4 重复与互斥 | 同批 `(sheet,key)` 重复、历史索引重复、二次运行幂等键拒绝、`preflight` 0 请求；01/08/10、05/07 范围拒绝；第二页历史重复整批拒绝；`entry_owner=Wps` 或 `exclusive=false` 时 `WPS_E_ENTRY_PERMISSION` | 非首页重复已覆盖“第二页重复 ID”；录入权互斥只覆盖配置标志，真实跨端互斥凭据/并发录入仍未测 |
| C5 分页与变化 | `wps_pull` 覆盖取页失败、坏行、游标不前进、空页假 `has_more`、页上限、跨页重复 ID、扫描中 `total` 变化、记录 `remote_modified_at` 晚于扫描起点；失败保留旧索引并阻断写；`c5_field_change_with_unique_ids_is_not_snapshot_verified` 证明“总数不变、ID 不重复、字段变化”无法被接口证明，`pull_all_history_strict` 因此 `writes_blocked` | 字段级快照无 token，严格路径只能阻断、不能证明；多页真实副本未执行 |
| C6 写入恢复 | 首笔成功/第二笔超时/第三笔未尝试；意图先落库；回填失败→pending_backfill；恢复需证据、不重发；二次运行拒绝；新增“远端已落库但响应丢失→重启 preflight 拒绝、0 create、补链不重发” | 请求后进程崩溃、真实重启、首次查询未见远端记录的补偿查询仍未执行 |

## 5. 边界与签审提示

- 本走查只证明**桌面命令入口 + Mock 执行器**可用；真实 HTTPS transport、真实 token、真实 WPS 权限、真实副本写入均未启用；`LoopbackHttpTransport` 只用于本地回环伪服务，不是生产传输。
- `posted` 本轮始终为 false；「正式入账」未发生，符合 Mock 边界。
- 六组真实副本 C1–C6 继续登记为未执行；需 G0/S0/G1 与副本专项授权后另跑，范围草案见 [真实副本授权范围草案](./真实副本授权范围草案.md)。
- S0-01/02/03 已有 WPS 专用 keyring-only 入口与合成后端测试，但未写真实 token、未演练目标机钥匙串；不能由本轮补证通过，见 [凭据链核查](./凭据链核查.md)。
- **点击由人执行**：助手侧 `osascript`/System Events 被 macOS 拒绝（`-10004`），点击动作无法自动化；本轮界面证据是「人点击 + 助手即时截图 + 临时库行级核对」的组合，重跑同样需要人工点击（命令见 §2.1）。
- **提示条已目视（2026-11-22 补做）**：面板新增 400006/429/恢复授权三个入口后，`authRequired`/`cooldownUntil` 两条提示均已实机渲染并截图，零请求行为与库增量见 §2.3；仍属合成错误，不是真实 WPS 演练。
- **截图瑕疵**：`04-重复运行-幂等键拒绝.jpg` 为整屏截图，窗口左缘越界导致「幂等键」列未入镜；错误列与行号清晰，且可与库内 `wps_idempotency_keys` 对账，不影响该步结论。
