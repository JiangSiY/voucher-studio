# WPS 台账同步：数据库迁移设计（v0.28 修订）

> 状态：方案 A 已批准并实施（W1-T1，2026-11-19）；W1 证据见 `docs/验收基线/证据/2026-11-19-W1/schema.md`。目的：解决方案 §3 的 `SCHEMA_VERSION 5` 与现有「bump 即清库」冲突，给出不丢数据的迁移路径。
> 关联：`docs/开发方案.md` §5.10（待落稿）；W0 报告 `docs/验收基线/证据/2026-11-19-W0/接口记录.md`。

## 1. 现状与冲突

| 项 | 事实 | 证据 |
|---|---|---|
| 当前版本 | `SCHEMA_VERSION = 4` | `crates/voucher-core/src/db/schema.rs:21` |
| 初始化策略 | `if version < SCHEMA_VERSION { DROP_ALL_V3; SCHEMA_V4; user_version=4 }` | 同文件 `init()` `:470-485` |
| 项目纪律 | 自 v5 起新增表/列均「幂等只增，不动 SCHEMA_VERSION」；注释明确「bump 会触发 v<4 的清库分支，把用户数据清空」 | 同文件 `:195`、`:253`、`:318` 等 |
| 方案 §3 | 要求 `SCHEMA_VERSION 5`，新增 `wps_*` 表 | v0.28 修订稿 §3 |

**结论：直接 bump 到 5 = 启动时清空用户库，不是迁移。** 该做法不能上线。

## 2. 三种方案

### 方案 A：保持 v4 + 幂等建表（推荐）

- 不修改 `SCHEMA_VERSION`；在 `schema::init()` 末尾执行 `SCHEMA_WPS`（全部 `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS`）。
- 新库：`SCHEMA_V4` 后执行 `SCHEMA_WPS`，得到完整表。
- 老库（v4）：跳过清库分支，直接补建 `wps_*`；现有 `vouchers/attachments/voucher_ext` 数据完全不动。
- 旧版本应用打开新库：`user_version` 仍为 4，旧版本忽略未知表，可正常回退；同步功能因表存在但代码未启用而不受影响。
- 风险：没有 schema 版本号可表达「WPS 表已建」；用 `wps_*` 是否存在 + `wps_state.schema_version` 自检（见 §5）。

### 方案 B：bump 到 5 + 真正的迁移框架（不推荐本轮）

- 把 `init()` 的「版本低就清库」改为「按版本逐级迁移」，需要：
  - `schema_migrations(version, applied_at)` 表；
  - `v4→v5` 迁移脚本；
  - 失败回滚/重试/降级策略；
  - 所有未来版本遵守迁移，不再清库。
- 优点：版本语义清晰。
- 缺点：改动核心启动路径，影响所有历史库；需要单独一轮回归；本轮 WPS 只是新增表，不需要动老表，收益低、风险高。
- 若未来要做，建议另立方案，不夹在 WPS 同步里。

### 方案 C：bump + 清库（拒绝）

- 等于用户升级即丢数据；违反 `AGENTS.md` 数据安全与 W0 门禁；不讨论。

## 3. 推荐方案 A 的实施细节

### 3.1 DDL 组织

- 新增常量 `SCHEMA_WPS: &str`，包含：
  - `wps_links`、`wps_conflicts`、`wps_tombstones`、`wps_scans`、`wps_schema`、`wps_master`、`wps_sync_runs`、`wps_run_changes`、`wps_run_created`、`wps_state`；
  - 全部 `CREATE TABLE IF NOT EXISTS`；
  - 索引全部 `CREATE INDEX IF NOT EXISTS`；
  - 部分唯一索引：`CREATE UNIQUE INDEX IF NOT EXISTS idx_wps_links_voucher ON wps_links(voucher_id) WHERE voucher_id IS NOT NULL`。
- 在 `schema::init()` 的 `SCHEMA_V15` 之后执行 `conn.execute_batch(SCHEMA_WPS)?`；**不执行** `pragma_update(user_version, 5)`。
- `wps_links.voucher_id` 外键 `REFERENCES vouchers(id) ON DELETE CASCADE`；`wps_conflicts.link_id`、`wps_run_changes.link_id` 外键；建表顺序先父后子。
- `wps_state` 写 `schema_version='wps-1'`，用于运行期自检；不参与 `PRAGMA user_version`。

### 3.2 事务与失败处理

```rust
pub fn init(conn: &Connection) -> Result<()> {
    // ... 现有版本判断/旧迁移 ...
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let r = (|| -> Result<()> {
        conn.execute_batch(SCHEMA_WPS)?;
        conn.execute_batch(
            "INSERT INTO wps_state(key,value) VALUES('schema_version','wps-1')
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        )?;
        Ok(())
    })();
    match r {
        Ok(()) => { conn.execute_batch("COMMIT")?; Ok(()) }
        Err(e) => { let _ = conn.execute_batch("ROLLBACK"); Err(e) }
    }
}
```

- 任一条 DDL 失败 → 回滚；数据库保持 v4；`init()` 返回错误，应用应显示「同步表初始化失败，同步已停用」，不得继续调用 WPS。
- `CREATE TABLE IF NOT EXISTS` 在 SQLite 中可回滚（DDL 事务）；索引同理。
- 不在事务中做数据回填（本轮无需回填）。

### 3.3 迁移前快照

即使方案 A 只是加表，也按「改库先备份」执行：

- 触发条件：DB 已存在、`vouchers` 表有数据、且 `wps_links` 表不存在。
- 做法：在 `schema::init()` 之前，由 `CoreContext::init` 调用 `snapshot_before_wps(&conn, &paths)`：
  - `VACUUM INTO '<backups>/pre-wps-YYYYMMDD-HHMMSS.db'`；
  - 只保留最近 3 份，避免占空间；
  - 失败则中止初始化（不允许在无备份的情况下改库）。
- 新库（无 `vouchers` 数据）不生成快照。
- 快照文件不进入普通备份列表（命名 `pre-wps-*.db`，与 `voucher-backup-*.zip` 区分）。

### 3.4 运行期自检

- `WpsStore::ensure_schema()`：启动/首次同步前检查 `wps_links`、`wps_state`、`wps_run_changes` 存在；缺失返回 `WPS_E_SCHEMA`，同步弹层提示「同步表未初始化，请重启或查看日志」。
- 不自动在建表失败后重试写 WPS；只读仍可用。

## 4. 数据兼容与回退

| 场景 | 行为 |
|---|---|
| 新库 | v4 表 + `wps_*`；`user_version=4`；同步可用 |
| 老库（v4，有数据） | 快照 → 幂等建表 → 数据不动；`user_version=4` |
| 建表失败 | 回滚；应用报错；同步禁用；数据不动 |
| 旧版本应用打开新库 | `user_version=4`，旧代码忽略 `wps_*`；可正常使用旧功能 |
| 降级 | 删除/保留 `wps_*` 均不影响旧版本；保留表不丢数据 |
| 恢复备份 | 恢复的是 v4 库；再次启动会补建 `wps_*`；同步链接丢失，需重新认领（文档写明） |

## 5. 测试与验收

| 用例 | 步骤 | 期望 |
|---|---|---|
| T-MIG-01 新库 | 空目录 `CoreContext::init` | 所有 `wps_*` 存在；`user_version=4`；无快照 |
| T-MIG-02 老库保数据 | 造 v4 库（vouchers/attachments/voucher_ext 若干）→ init | 行数/字段不变；表存在；`user_version=4` |
| T-MIG-03 幂等 | 连续 init 两次 | 无错误、无重复表/索引、数据不变 |
| T-MIG-04 快照 | 有数据老库首次 init | `backups/pre-wps-*.db` 存在且可打开 |
| T-MIG-05 失败回滚 | 注入一条非法 DDL（测试专用） | 回滚；`wps_*` 不存在；`user_version=4`；数据不变 |
| T-MIG-06 外键 | 删除有 `wps_links` 的 voucher | 链接按设计级联/或先墓碑；测试按实现断言 |
| T-MIG-07 旧版本兼容 | 用当前 v4 代码打开含 `wps_*` 的库 | 正常启动；旧功能可用 |
| T-MIG-08 运行期自检 | 删 `wps_state` 后调用 `ensure_schema` | 返回 `WPS_E_SCHEMA`，不调用 WPS |

验收门槛：
- 老库数据行数与关键字段哈希前后一致；
- `PRAGMA user_version` 保持 4；
- 快照存在且能打开；
- 前端/服务端在表缺失时只读降级，不崩溃；
- 全量 `cargo test` 与五门通过。

## 6. 对方案 §3 的修订建议

将 §3「SCHEMA_VERSION 5」改为：

> 保持 `SCHEMA_VERSION = 4`；新增 `SCHEMA_WPS` 幂等建表，随 `schema::init()` 执行；不 bump、不清库、不改老表。迁移前对已有库做 `VACUUM INTO` 快照，失败回滚并禁用同步。`wps_state.schema_version='wps-1'` 用于运行期自检。

这样既满足新增 `wps_*` 表，又不触发清库；与仓库既有「幂等只增」纪律一致。若未来需要真正的版本迁移框架，另立方案。

## 7. 待确认

1. 是否批准方案 A（保持 v4 + 幂等建表 + 迁移前快照）。
2. 迁移前快照保留 3 份是否足够；是否要纳入设置页可见。
3. 旧版本兼容是否要求「删除 wps_* 表后仍可打开」；本设计默认保留表、旧版本忽略。
4. 本轮是否同时把 `docs/开发方案.md` §5.10 落稿，还是等 W1 代码完成后再写。
