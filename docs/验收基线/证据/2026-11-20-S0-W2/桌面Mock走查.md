# 桌面开发入口 Mock 走查（2026-11-20）

> 范围：本地 debug 桌面 + `transport=Mock` + 临时 SQLite + 合成数据。**未连接真实 WPS、未发 HTTP、未操作生产库、未部署。** 本记录不构成 S0/G1 通过或 G0 验收结论。

## 1. 启动与隔离

```bash
cd /Users/apple/Downloads/开发/voucher-studio
export VOUCHER_DATA_DIR="$(mktemp -d /var/folders/…/voucher-w2-mock.XXXXXX)"
export VOUCHER_W2_MOCK=1
. "$HOME/.cargo/env"
export PKG_CONFIG_PATH="$HOME/.local/opencv-4.10/lib/pkgconfig"
npm run tauri dev
```

约束：

- 仅 debug 构建 + `VOUCHER_W2_MOCK=1` + `VOUCHER_DATA_DIR` 位于系统临时目录时入口可用；普通运行、生产目录、网页端会收到 `dev_only`。
- 命令只接受 `action=pull|write`、`scenario=normal|timeout_second|missing_reference`，不接受 file_id 或数据目录；界面无法把目标换成真实文件/生产库。
- 桌面数据页 →「开发本地模拟同步（W2 Mock）」面板。

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

## 3. 自动化证据

```bash
cargo test -p voucher-core --lib --offline services::wps_w2_dev
# 8 passed / 0 failed

cargo test -p voucher-api --test dev_w2_entry --offline
# 1 passed / 0 failed：pull + normal + timeout_second + missing_reference

cargo test -p voucher-api --lib --offline dev_w2::tests
# 2 passed：守卫拒绝非 debug/非 flag/非临时目录；status 文本稳定
```

## 4. C1–C6 本地 Mock 覆盖

| 用例 | 本地 Mock 已覆盖 | 缺口 |
|---|---|---|
| C1 日期 | `classify_date` 覆盖 2026-10-31 旧、2026-11-01 新、空/非法 NeedsReview；`classify_period` 覆盖 09 YYYY-MM 与非法值 | 06 到账日/02/03/04/11 各表真实字段值、`2026-10-31` 到账但 11 月补录等业务数据未造 |
| C2 旧资料 | `preflight` 对 01/08/10 引用做存在性校验；缺引用场景整批拒绝；资料零新增（引用只读） | 同名不同 ID、分界日前失效资料、真实关联目标未测 |
| C3 拉取恢复 | `wps_pull` staged：不完整/坏页/游标回退保留旧索引并阻断写；空页假 `has_more` 有测试；`dev_pull` 11 表全历史 | 扫描中插入/删除/更新、同键字段冲突、总数不变场景未测；真实远端重扫未执行 |
| C4 重复与互斥 | 同批 `(sheet,key)` 重复、历史索引重复、二次运行幂等键拒绝、`preflight` 0 请求；01/08/10、05/07 范围拒绝 | 非首页重复、多候选、无互斥凭据并发录入、跨端互斥未测 |
| C5 分页与变化 | `wps_pull` 覆盖取页失败、坏行、游标不前进、空页假 `has_more`、页上限；失败保留旧索引 | 多页表、扫描中增删改、同总数变化未测；真实副本未执行 |
| C6 写入恢复 | 首笔成功/第二笔超时/第三笔未尝试；意图先落库；回填失败→pending_backfill；恢复需证据、不重发；二次运行拒绝 | 远端已落库但响应超时（含首次查询未见）、请求后崩溃、真实重启恢复未执行 |

## 5. 边界与签审提示

- 本走查只证明**桌面命令入口 + Mock 执行器**可用；真实 HTTP transport、真实 token、真实 WPS 权限、真实副本写入均未启用。
- `posted` 本轮始终为 false；「正式入账」未发生，符合 Mock 边界。
- 六组真实副本 C1–C6 继续登记为未执行；需 G0/S0/G1 与副本专项授权后另跑。
- S0-01/02/03 无 WPS 凭据链，不能由本轮补证通过；见 [凭据链核查](./凭据链核查.md)。
