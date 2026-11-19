# W1-T1 数据库迁移证据（WPS 同步表）

- 日期：2026-11-19
- 任务：按批准的方案 A，保持 `SCHEMA_VERSION=4`，幂等新增 WPS 同步表 + 迁移前快照。
- 关联：`docs/迁移设计-WPS台账-v0.28.md`；W0 报告 `docs/验收基线/证据/2026-11-19-W0/接口记录.md`。

## 1. 变更

| 文件 | 变更 |
|---|---|
| `crates/voucher-core/src/db/schema.rs` | 新增 `SCHEMA_WPS`（11 张表 + 索引；含 W1-T3 的 `wps_idempotency_keys`）；`init()` 末尾事务内 `apply_wps_schema`；新增 `needs_wps_snapshot` / `snapshot_before_wps` / `prune_pre_wps_snapshots`；3 个迁移测试 |
| `crates/voucher-core/src/lib.rs` | `CoreContext::init` 在 `schema::init` 前调用快照；失败中止启动 |

## 2. 表清单

`wps_links`、`wps_tombstones`、`wps_scans`、`wps_conflicts`、`wps_schema`、`wps_master`、`wps_sync_runs`、`wps_run_changes`、`wps_run_created`、`wps_idempotency_keys`、`wps_state`。

- `wps_links`：`UNIQUE(sheet_id, record_id)`；`voucher_id` 部分唯一索引；`idempotency_key` 普通索引；`voucher_id` 外键 `ON DELETE CASCADE`。
- `wps_conflicts`：`link_id` 外键 `ON DELETE CASCADE`；`UNIQUE(link_id, field_id)`。
- `wps_run_changes`：`UNIQUE(run_id, link_id, field_id)`；无外键，保留审计。
- `wps_idempotency_keys`：`idempotency_key` 主键、`run_id`、`record_type`、`local_status`（`pending/succeeded/unknown`）、`remote_record_id`、`created_at`。
- `wps_state`：写入 `schema_version=wps-1`。

## 3. 快照

- 条件：已有库（`vouchers` 表存在且有数据）且 `wps_links` 不存在。
- 方式：`VACUUM INTO backups/pre-wps-YYYYMMDD-HHMMSS-fff.db`。
- 保留最近 3 份；失败则 `CoreContext::init` 返回错误，不继续建表。
- 新库/已有 `wps_links` 的库不生成快照。

## 4. 测试

命令：

```bash
source "$HOME/.cargo/env"
export PKG_CONFIG_PATH="$HOME/.local/opencv-4.10/lib/pkgconfig"
cargo test -p voucher-core --lib db::schema::tests -- --nocapture
```

结果：**3 passed; 0 failed**（194 filtered out）。

| 用例 | 覆盖 | 结果 |
|---|---|---|
| `wps_schema_is_idempotent_and_preserves_data` | 连续两次 init；数据不丢；`user_version=4`；`wps_state=wps-1`；表存在 | 通过 |
| `snapshot_created_for_existing_db_without_wps_tables` | 旧库（仅 SCHEMA_V4）有凭证 → 快照存在且可打开、数据不丢、无 WPS 表 | 通过 |
| `snapshot_skipped_for_new_db` | 新库不生成快照 | 通过 |

静态检查：

```bash
cargo check -p voucher-core
cargo clippy -p voucher-core --all-targets -- -D warnings
```

均通过（仅有既有 `proc-macro-error2` future-incompat 警告，非本次引入）。

## 5. 未覆盖 / 待办

- T-MIG-05 失败回滚：未注入非法 DDL 做自动化测试；当前由事务 + 错误返回保证，后续补测试。
- `WpsStore::ensure_schema` 运行期自检尚未实现（W1-T2）。
- 真实 WPS 读取/写入未调用；S0 未签字，W1 仅 mock/本地。
- 旧版本兼容（当前 v4 代码打开含 `wps_*` 的库）未单独跑；逻辑上 `user_version=4` + 旧代码忽略未知表。

## 6. 结论

W1-T1（schema/快照）**通过**；`SCHEMA_VERSION` 仍为 4，未清库、未改老表。可进入 W1-T2（WpsClient 接口与假 MCP、`wps_state` 自检），真实 WPS 调用仍等 S0。
