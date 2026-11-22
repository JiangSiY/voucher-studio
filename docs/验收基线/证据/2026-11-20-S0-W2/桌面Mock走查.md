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
- 命令只接受 `action=pull|write`、`scenario=normal|timeout_second|missing_reference`，不接受 file_id 或数据目录；界面无法把目标换成真实文件/生产库。
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

## 2.1 人工点击状态（2026-11-22 复核）

- **实测范围**：2026-11-22 本轮只做进程内测试与静态核对，**未启动新窗口、未新增截图、未点击任何实际按钮**；面板按钮的真实点击与渲染仍为「待人工验收」，不得引用本轮测试替代。
- **截图与测试分开留痕**：窗口启动截图只有 2026-11-20 的 [`桌面Mock窗口截图.png`](./桌面Mock窗口截图.png)，仅证明窗口出现；`authRequired`/`cooldownUntil` 提示条的实际显示、按钮点击后的界面变化无截图证据。
- **结构化反馈**：本轮在 `src/pages/DataPage.vue` 增加两条提示（需重新授权、冷却至某时刻），数据来自 `DevW2Batch` 的 `authRequired`/`cooldownUntil`；字段与 Rust DTO 已核对，但界面渲染未目视确认。
- **按钮行为**：拉取、模拟创建（正常/第二笔超时/缺引用）由 `wps_w2_dev` 8 例、`dev_w2` 4 例与 `dev_w2_entry` 1 例在进程内覆盖，**不等于**桌面点击验收。

## 2.2 上轮人工点击记录（2026-11-20）

2026-11-20 桌面开发版已启动成功：日志出现 `W2 Mock 启动隔离校验通过：/private/var/.../voucher-w2-mock.oyWR0R`，窗口已显示工作台（截图：[`桌面Mock窗口截图.png`](./桌面Mock窗口截图.png)）。但当前执行环境的 `osascript` 访问 Tauri 原生窗口被 macOS 拒绝（返回“osascript 不允许辅助访问”），尝试将窗口置前并使用 System Events 点击均失败；**未完成人工点击目视，保留“待人工验收”**。截图只证明窗口启动，不当作同步流程已验收证据。可复跑命令见第 1 节；若在目标 Mac 上执行，按第 2 节顺序点击并记录：

- 每步的 `mode`、`dataDir`、11 表完整性；
- `wps_write_intents` 与 `wps_links` 的增量；
- 重启应用后再次点击同场景，确认幂等键存在、整批拒绝、不重发。

## 3. 自动化证据

```bash
cargo test -p voucher-core --lib --offline services::wps_w2_dev
# 8 passed / 0 failed

cargo test -p voucher-api --test dev_w2_entry --offline
# 1 passed / 0 failed：pull + normal + timeout_second + missing_reference

cargo test -p voucher-api --lib --offline dev_w2::tests
# 4 passed：启动校验（空路径/临时根/外指符号链接/有效目录）；守卫拒绝；status 文本稳定

cargo test -p voucher-core --lib --offline services::wps_http
# 4 passed（2026-11-22 新增）：回环白名单、Bearer 注入、缺/过期凭据零请求、不跟随重定向、错误体不回显
```

2026-11-22 复跑：`services::wps_w2` 29 passed（含 `wps_w2_dev` 8）、`dev_w2_entry` 1 passed、`dev_w2::tests` 4 passed；全量 `cargo test --workspace --no-fail-fast --offline -- --include-ignored` → 487 passed / 0 failed / 0 ignored。

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
