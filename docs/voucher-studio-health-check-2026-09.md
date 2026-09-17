# Voucher Studio 全面健康体检报告

> 版本：v0.13.1 ｜ 体检日期：2026-09-09 ｜ 代码规模：Rust 约 18k 行 + 前端 8.5k 行 + Tauri 命令层 1450 行
> 参检：架构评审（高见远）· 代码质量（寇豆码 ×2 独立评估）· 测试评估（严过关）｜ 汇总：齐活林
> 方式：全量静态扫描 + 抽样取证 + 前端用例实测（未改动任何源码）

> ⚠️ **校正注记（2026-09-17 追加，正文不改）**：本报告是 **v0.13.1 时点快照**。截至 v0.14.0（2026-09-17）的收口情况——Q-1/Q-2/Q-3/Q-4/Q-5/Q-6/Q-8/Q-11/Q-12/Q-18/Q-27/Q-28/Q-30 与 5 处「假绿」多已修复（v-model 单向流、AEAD age 信封、zip-slip、assetProtocol 收窄、WebDAV https、ESLint、clippy 全 workspace、契约快照测试、archive 清理等）；**同族新增一处缺陷**：WebDAV 回环判定 userinfo 绕过（Q-3 的残留面），2026-09-17 发现并修复（`services/backup/webdav.rs` + 5 反例回归）。用例数 178 → 现值 **212**（121 单元 + 91 集成）。**仍成立**：ort 仍 pin `=2.0.0-rc.13`、Prettier 仍未引入。逐条对照见 `docs/voucher-studio-status-audit-2026-09-17.md` §五。

---

## 〇、TL;DR（结论先行）

**这是一套下限很高、架构纪律良好的代码库，但"保障网"存在三块结构性空白，且有 2 处可 panic 的实现细节。**

| 结论 | 说明 |
|---|---|
| ✅ 架构最硬的资产 | `voucher-core` 对 Tauri **零依赖**（全仓仅 1 处注释提及），178 个测试可脱离 GUI 独立验证 |
| ✅ 安全基本面扎实 | SQL 100% 参数化、凭据 100% 走系统 keyring、生产代码 0 个 `panic!`、前端 `any` 零命中 |
| 🔥 最急的两颗雷 | `webdav.rs:480` 用 `to_lowercase()` 索引切原串 → 非 ASCII 响应**可能 panic**；`unit_of_work.rs` 锁中毒 → **全应用数据库永久不可用** |
| ⚠️ 债在"壳层"不在"核心层" | 命令层越层直连 DB（15 处）、错误类型被抹成字符串（38 处） |
| ❌ 三块保障空白 | Tauri 命令桥 65 个命令 **0 测试**；WebDAV 780 行 **0 联网用例**；前端 26 个组件/页面 **0 测试** |
| ❌ 两个外部可利用面 | WebDAV 允许 http 明文传密码（9 处）；云端 OCR `api_url` 未校验 scheme，Bearer Key 明文外传 |

**综合健康度：B**（架构 B+ ｜ 代码质量 B / 72–83 ｜ 测试 C+）

---

## 一、评分卡

| 维度 | 评级 | 核心依据 |
|---|---|---|
| **分层清晰度** | B+ | 三层职责无歧义，依赖严格单向无环；扣分于命令层 15 处越层 |
| **核心层解耦** | A− | `voucher-core` 零 Tauri 依赖，178 测试独立背书 |
| **扩展性** | B− | OCR 无 trait（字符串 enum）；导出格式扣分项比初判轻——版式已按函数拆分（`generate_daily` 仅 12 行分流器），实质只是表引擎重复 |
| **错误处理** | B | `CoreError` 13 变体质量好；扣分于边界压成 `String`、`Invalid` 变体被滥用 126 次 |
| **安全** | B+ | SQL/凭据/注入面达标；扣分于 2 处明文传输面、`assetProtocol` 全开、路径校验不全 |
| **依赖健康** | A− | 依赖零冗余；扣分于 `ort` 锁定 RC 版、前端无 ESLint |
| **测试覆盖** | C+ | 后端 178 用例质量高；前端 25 用例、命令层 0、组件 0 |
| **并发健壮性** | C | 0 个并发用例，7 处锁中毒面 |

---

## 二、架构维度（高见远）

### 依赖核验
- ✅ **无环**：`FE → SH → CORE → 外部` 严格单向，`src-tauri` 单向依赖 core，反向零引用
- ✅ **核心层自证清白**：`grep -rn "tauri" crates/voucher-core/` 仅命中 1 行**注释**
- ❌ **一处越层**：`src-tauri` 命令层 15 处直连 `db::repos`，且 repo 类型被当作前后端契约（DB schema 变更会直接破坏前端）

### 核心数据流（设计亮点）
- **OCR**：以 `file_hash` 为缓存键（同图永不重复识别）；版面数据单独落 `ocr_layouts` 支持几何配对；低置信度字段 UI 显式标注——**人机协同的正确取舍**
- **增强管线**：`opt_step`「单步失败跳过 + trace 记录」是教科书级稳健设计；双输出（彩色给 PDF、二值给 OCR）切中要害
- **备份**：`backup_guard` Mutex 串行化（注释点明 VACUUM 临时路径冲突 + zip 秒级时间戳碰撞）、`drill_backup` 演练、`warnings` 显式化部分成功语义——**本仓设计最完整的子系统**

### 架构问题
| # | 级别 | 问题 |
|---|---|---|
| A-1 | **P0** | 命令层 15 处越层直连 repos + repo 类型泄漏为契约 |
| A-2 | **P0** | OCR 引擎无 trait，用字符串 enum；云端无版面被隐式降级，前端置信度含义不一致 |
| A-3 | P1 | 事务边界不完整（增强路径写入在事务外→孤儿增强文件）；全仓仅 10 处 `with_tx` |
| A-4 | P1 | `Result<_, String>` 扁平化，前端无法按错误类型分支 |
| A-5 | P1 | 配置用裸 `String` 枚举（theme/template/engine/card_style） |
| A-6 | P1 | 前后端事件为弱契约字符串，散落 `main.rs` + 10+ `.vue` |
| A-7 | P2 | `voucher_repo` 515 行偏大；`period`/`reports` 横切未下沉 |
| A-8 | P2 | 前端无业务 store，loading/error/data 三态每页重复 |
| A-9 | P2 | `examples/` 混入一次性调试脚本，`webdav_e2e` 放错位置 |
| A-10 | P1 | `pdf.rs` **两组复制粘贴的表引擎**：`draw_table_head`(709-739) vs `draw_yearly_head`(1119-1147) 尾部 10 行逐字相同；`draw_table_row`(742-806) vs `draw_yearly_row`(1150-1229) 共用同一基线公式 `top + row_h - 1.9`、同一套填充色切换、同一条 `top + row_h - 0.3` 分隔线。**列偏移不一致**：日报侧用命名常量 `TCOL_NO/TCOL_DESC/TCOL_PARTY`，年度侧用裸 magic number `MARGIN + 30/50/70/92/114`。→ 抽 `TableRenderer` 列定义数据驱动，消约 150 行重复；**不单独立项**，下次有 PDF 排版需求时顺手做 |

---

## 三、代码质量维度（寇豆码 ×2 独立评估）

> 两位工程师独立扫描，评分 **72/B−** 与 **83/B**，差异源于权重侧重（一位重安全/可用性/依赖，一位重重复/前端工程化），**问题清单高度互补**，以下为合并去重后的并集。

### 🟢 实测达标项（下限很高，两位一致确认）
- 生产代码 **0 个** `panic!` / `unreachable!` / `todo!` / `FIXME`（全库 0 处标记，干净）
- **SQL 100% 参数化**：82 处 SQL 全 `?N` 占位符；`escape_like`（`voucher_repo.rs:388-392`）正确转义 `\ % _` 且 SQL 侧声明 `ESCAPE '\'`——**转义闭环完整，无注入面**
- **凭据真走 keyring**：`utils/crypto.rs` 全程 OS 原生凭据库，配置只存 `password_set: bool`，日志无密码字段
- **备份加密取舍正确**：`crypto.rs:29-43` 开启加密但无密码时**报错而非静默降明文**
- **`unsafe` 仅 1 处生产代码**：`file_lock.rs:62` 的 `libc::flock`（另 3 处在 `#[cfg(test)]` 内）
- **前端零妥协**：`as any` / `@ts-ignore` / `eslint-disable` **全 0 命中**，props 100% TS 泛型化
- **依赖零冗余**：22 个 Rust 依赖 + 7 个前端运行时依赖全部有引用
- **无命令注入面**：全库 0 处 `Command::new`，文件选择全走 Rust 侧原生对话框
- `unwrap()` 全库 124 处命中看似吓人，但**108 处在测试代码内**，生产路径仅 16 处（`src-tauri` 生产代码为 0）

### 🔴 Top 问题清单

| # | 文件:行号 | 问题 | 级别 |
|---|---|---|---|
| Q-1 | `backup/webdav.rs:480-496` | `to_lowercase()` 后在 lower 上求索引，却用 `&body[..]` 切原串。Unicode（如 `İ`）大小写转换会改变字符数 → 索引错位 → **可能切在非字符边界 panic**。且 PROPFIND 响应是手写 XML 解析 | **P1（可 panic）** |
| Q-2 | `db/unit_of_work.rs:25,31` + `lib.rs:91,97,108,118` + `ocr.rs:153`（7 处） | **Mutex 中毒后全应用数据库永久不可用**且无恢复路径。注意 `lib.rs:103` 已用 `unwrap_or_else(\|p\| p.into_inner())` 正确处理，**此处不一致** | **P1（可用性）** |
| Q-3 | `services/ocr.rs:169-190` | 云端 OCR `api_url` **未校验 scheme**，填 `http://` 时 `bearer_auth(key)` API Key **明文外传** | **P1（安全）** |
| Q-4 | `backup/webdav.rs`（9 处 `basic_auth`） | 无 scheme 校验，http 下密码 Base64 明文过网 | **P1（安全）** |
| Q-5 | `SettingsPage.vue:34-70` ↔ `config.rs:46-240` | `AppConfig` 默认值**两份真源且已分歧**：`naming.template` 前端 `{date}-{seq}` vs Rust `{date-}-{n}`；`print.title` 前端「凭证日报」vs Rust「原始凭证」 | **P1（正确性）** |
| Q-6 | `backup/restore.rs:203-207` | zip-slip 只查 `..`，**未拒绝绝对路径条目** | **P1（安全）** |
| Q-7 | `src-tauri/commands/*.rs`（38 处，`mod.rs:38`） | `CoreError` 13 变体被压成裸字符串，前端无法差异化处理 | **P1** |
| Q-8 | `tauri.conf.json:30-33` | `assetProtocol.scope: ["**"]` 资源协议作用域全开 | **P1（安全）** |
| Q-9 | `CoreError::Invalid` 被用 **126 次** | 其中 `enhance_geom` 32 + `enhance_pixel` 12 全是 OpenCV 错误手动包装样板，**缺 `Cv(#[from] opencv::Error)` 变体** | **P1（重构收益高）** |
| Q-10 | `crates/voucher-core/Cargo.toml:34` | `ort = "=2.0.0-rc.13"` 锁定 RC 预发布版，无法收补丁/安全更新 | **P1** |
| Q-11 | `package.json` | **无 ESLint / Prettier / eslint-plugin-vue** —— 8.5k 行前端完全无静态检查（CI 前端 job 只有 typecheck + vitest + build） | **P1（工程化）** |
| Q-12 | `reports.rs:70-95` ↔ `319-346` | 「月份前缀 + 状态透视」SQL 与 `HashMap<String,[i64;6]>` 构建**整体复制两份**，且 `[i64;6]` 裸数组按 `d[0]..d[5]` 取值（magic index） | P1 |
| Q-13 | `enhance_cache.rs:452-460` | 两个空壳占位函数，用 `#[allow(dead_code)]` 掩盖真正未使用的 import（拿 lint 抑制当创可贴） | P1 |
| Q-14 | `EnhanceTab.vue:86,132` | 用原生 `confirm()`，而项目已有统一 `ConfirmDialog.vue`（5 处使用）→ UI 一致性破坏 | P1 |
| Q-15 | `BackupTab.vue:31-35` ↔ `SecretsTab.vue:16-20` | 密钥保存逻辑（判空→setSecret→清空 draft→try/catch）**逐行复制** | P1 |
| Q-16 | `enhance.rs:157-165` ↔ `297-305` | `PipelineOptions{7 字段}` 映射代码复制两份，缺 `impl From<&EnhanceConfig>` | P1 |
| Q-17 | `enhance_geom.rs:31-42/118-129/252-265` | 灰度化块复制 3 份；且混用 `src.clone()` 与 `try_clone()`（**语义不一致**） | P1 |
| Q-18 | 23 处 `v-model="cfg.xxx"` | **直接双向绑定 props 对象**，违反单向数据流（配套：`JSON.parse(JSON.stringify())` 深拷贝 hack） | P1 |
| Q-19 | `pdf.rs:709-739` vs `1119-1147`、`742-806` vs `1150-1229` | 表引擎两套复制粘贴（约 150 行），列偏移 30/50/70/92/114 为裸 magic number | P2 |
| Q-20 | 路径穿越校验不全 | `attachment_abs()` 约 20 处调用，**仅 7 处**前置 `is_inside_attachments()` 校验 | P2 |
| Q-21 | `commands/mod.rs:69` `open_path(path: String)` | 直接交系统 opener，**无任何白名单/规范化校验** | P2 |
| Q-22 | `App.vue:66-109` | 8 个串行 `await listen(...)` 全在 try 之外，任一 reject 导致后续监听全不注册 | P2 |
| Q-23 | `v-model` 配套 + 未处理 async | `RestorePage:180`、`PrintCenterPage:233`、`CommandPalette:107`、`VoucherFormPage:468` 等 fire-and-forget 无 `.catch` | P2 |
| Q-24 | `enhance_hit_repo.rs:107` | 未知枚举 `_ => log::warn!` 静默降级（与测试"假绿"同源） | P2 |
| Q-25 | `crypto.rs:65-66` | 用 `msg.contains("password")` **字符串匹配**判断是否加密条目，依赖 zip 库英文文案，换版本即失效 | P2 |
| Q-26 | `voucher_repo.rs:201` | `((page-1)*page_size) as i64`，`u32` 乘法极大值时 debug 溢出 panic | P2 |
| Q-27 | `crypto.rs:18-27` | WinZip AES-256 **非 AEAD、无 MAC**，密文可篡改 | P2 |
| Q-28 | CI（`.github/workflows/ci.yml`） | **clippy 只覆盖 `voucher-core`**，`src-tauri` 14 个命令文件不在 lint 范围 | P2 |
| Q-29 | DashboardPage 1023 行 / VoucherFormPage 957 行 / EnhanceTab 749 行 | 组件臃肿（含手写 SVG 柱状图 ~40 个顶层函数） | P2 |
| Q-30 | 根目录 10 个 `.archive-*` 归档目录 | 约 1MB 历史杂物在版本库内 | P2 |
| Q-31 | `db/unit_of_work.rs:25,31`（`with_tx`/`with_conn`） | **持 std Mutex 调用闭包**，闭包内再次取连接**直接死锁**（std Mutex 非重入），编译器不报错。当前 10 处用法安全（repo 只接 `&Connection`/`&Transaction`），属**隐性不变量、零防护**；而 P0-① 的 15 处越层直连正把「直接持连接」变成常规写法，放大踩中概率。**效力边界（勿过度承诺）**：拟加的 `thread_local` 护栏**只能检出同线程重入**（拦得住"闭包内再调 `with_conn`"，现实约 99%）；**拦不住**闭包内 `spawn_blocking` 到线程 B 再取连接（A 持锁等 B、B 等 A 锁，且 B 的 `thread_local` 干净）。彻底堵死需类型层让闭包拿不到 `&CoreContext`，大改，记技术债本轮不做。**终版实施规格见附录 A** | **P1（架构防护）** |

### ⚠️ 对架构结论的实测修正（重要）
架构师报告称「`pdf.rs` 中 `generate_daily` 约 700 行巨型函数」**与代码不符**。实测函数边界：

| 函数 | 起止 | 实际长度 |
|---|---|---|
| `generate` | 94→246 | 152 行 |
| `generate_daily` | 326→338 | **12 行**（薄封装，分流 archive/compact） |
| `generate_daily_compact` | 426→550 | 121 行 |
| `draw_cover_page` | 609→709 | 100 行 |
| `draw_card` | 847→948 | 101 行 |

**债务真实形态**：1364 行里塞了 20+ 个 60~150 行绘制函数（常量化做得好：30+ 命名常量）+ **两组复制粘贴表引擎**，而非"一个 700 行巨函数"。
→ **重构方案随之改变**：不是"拆巨型函数"，而是**抽列表排版引擎 + 列定义数据驱动**。
→ 补充实测：**除 pdf.rs 外全项目无 >200 行函数**（最长 `enhance_geom::dewarp` 125 行）。**pdf.rs 之外的复杂度债被高估了。**

### 🌟 值得肯定的做法（重构时勿破坏）
1. 凭据 100% 走 OS 原生凭据库，配置只存 `password_set` 布尔
2. 备份加密「宁可失败也不静默降明文」
3. 全库 0 处外部进程调用，文件选择一律 Rust 侧原生对话框（绕开 WebView 桥竞态）
4. `unwrap/expect` 严格限制在测试代码（124 处中 108 处）
5. `pdf.rs` 版面常量化（30+ 命名常量）是本项目最好的常量管理范例，**应推广到 `enhance_*`**（后者全是裸字面量）
6. 前端零 `any`、100% 类型化 props、605 行集中式 typed API 层
7. CI 有 `clippy -D warnings`——这是当前代码没有大规模坏味道的**根本原因**

---

## 四、测试覆盖维度（严过关）

### 分布（实测）
| 层 | 用例数 | 被测规模 | 实测 |
|---|---|---|---|
| voucher-core 单元 | 95 | 13.4k 行 | 静态统计 |
| voucher-core 集成 | 83 | — | 静态统计 |
| **src-tauri（65 命令）** | **0** | 1450 行 | — |
| **前端** | **25**（4 文件） | 7098 行 | **vitest 全绿 2.29s** |

**覆盖估算**：voucher-core 行覆盖 **55–65%**（分支偏低）｜ src-tauri **0%** ｜ 前端约 **15%**
README 宣称「120+」属实且保守，实际 **178**。

### ✅ 覆盖充分
OCR 字段/版面/校验（38 单元，含非法日历日期 2月30日 拒绝、多金额放弃）｜增强管线（17 单元 + 24 e2e，7 个 `*_empty_errors`）｜备份本地/加密/保留（含**篡改校验和拒绝**、异盘不可写仅告警）｜崩溃恢复 marker 状态机｜凭证主链路（集成 131 处引用）

### ❌ 覆盖空白（真正风险）
| 模块 | 规模 | 用例 |
|---|---|---|
| Tauri 命令桥 | 65 命令 / 1450 行 | **0** |
| WebDAV 网络链路 | 780 行 | 5 个**纯函数**（联网函数 0） |
| 前端 api/client.ts | 605 行 / 70 invoke | **0** |
| 前端 13 组件 + 8 页面 | — | **0** |
| db/repos | 1183 行 / 9 个仓库 | 仅 `period_repo` 1 个 |
| db/unit_of_work `with_tx` | 6 处多表写入依赖 | **0** |
| utils/config + paths | 390 行 | **0** |

### 异常路径缺口
| 场景 | 现状 |
|---|---|
| 数据库损坏 / 异常数据 | ❌ 0 用例（`grep -i corrupt/malformed/truncat` 全 0） |
| 并发写入 / 文件锁冲突 | ❌ **全库 0 个并发用例**，`backup_guard` 互斥零验证 |
| WebDAV 备份失败 | ❌ 0 自动化（无 mock、无故障注入） |
| 定时备份触发链路 | ⚠️ 仅纯函数 `is_due`，`spawn_backup_scheduler` 0 用例 |
| 事务回滚一致性 | ❌ 0 用例 |
| Unicode / 空格 / 超长路径 | ❌ 0 用例 |
| 增强管线异常输入 | ✅ 充分 |
| 空数据库启动 | ✅ 有 |

### 🔴 质量隐患：5 处"假绿"
`ocr_flow.rs:67/73/109/115/119`、`voucher_flow.rs:201/1038/1682` 用 `eprintln!("跳过…"); return Ok(())` **静默返回**而非 `#[ignore]`——在缺模型/字体环境下通过数具有欺骗性。

### 回归高危区（改动次数 × 测试稀少）
| 文件 | 改动 | 测试 |
|---|---|---|
| `src/api/client.ts` | 24 次 | 0 |
| `src-tauri/src/main.rs` | 24 次 | 0 |
| `src-tauri/src/commands/*` | 20 次 | 0 |
| `utils/config.rs` | 12 次 | 0 单元 |
| `VoucherFormPage.vue` / `SettingsPage.vue` | 各 16 次 | 0 |

---

## 五、交叉验证：多角色互证与事实更正

> 本次体检最有价值的部分——四份报告独立产出，结论互相对齐，且**修正了 2 处前序判断**。

| 事实 | 印证情况 |
|---|---|
| **微信监听已移除** | 架构师（提交 `50d2adb`，`reconcile.rs` 仅剩只读 CSV 对账）与 QA（全库 grep 无 `notify`/`watcher`）**独立确认一致** → 非测试缺口，从风险清单剔除 |
| **增强管线是 7 步非 5 步** | 架构师发现（`enhance.rs:1-15`，v0.6.1 加入中值去噪 + Unsharp 锐化），README 未回写 |
| **pdf.rs 债务形态** | 寇豆码**实测修正**架构师结论：非 700 行巨函数（实为 12 行薄封装），真实债务是复制粘贴表引擎 → **重构方案改变** |
| **两位工程师的核心结论一致** | 均确认：SQL 全参数化、凭据走 keyring、零 `any`、依赖零冗余、`unit_of_work` 锁中毒、webdav 无 HTTPS、`ort` 锁 RC、生产代码 0 `panic!` |
| **"假绿"同源** | QA 发现测试静默跳过（`return Ok(())`）；寇豆码发现 `enhance_hit_repo.rs:107` 未知枚举静默降级 —— **同一类问题的两个面** |
| **验收基线文档失真** | QA 发现 `docs/验收基线/清单1` 仍写「45 用例」，实际 178；H 节 4 条微信用例随功能移除仍标 ✅ |

---

## 六、统一改进清单（按成本收益比排序）

### 🥇 第一批：止血速赢（合计 ≈ 2 人日）
> 消除**可 panic 点** + **3 个外部可利用安全面** + **最严重的可用性故障模式**

| 动作 | 位置 | 成本 | 收益 |
|---|---|---|---|
| 1. 修 `webdav.rs:480-496` 字符边界 | 用 `body.get(..)` 或 `char_indices` 搜索 | 0.3 人日 | **极高**：消除唯一已知可 panic 点 |
| 2. 锁中毒修复 **+ RAII 重入护栏（合并项·勿拆）** | `unit_of_work.rs:25,31` 等，改 `unwrap_or_else(\|e\| e.into_inner())`（对齐 `lib.rs:103`）**并同步 `log::error!`**（中毒意味上一持锁者 panic 过，连接可能残留未提交事务，静默恢复会让故障延后以「cannot start a transaction within a transaction」在别处爆发）；② `with_tx`/`with_conn` 加 `# Panics` 文档注释（零成本，明示"闭包内禁止再次取连接"）；③ 重入护栏**必须用 RAII（Drop）**——`unit_of_work.rs:32/33/34` 有三处 `?` 早退点，若在函数尾部 `set(false)` 会被跳过，`HOLDING` 永久卡 true 后**每次调用都误报重入**（护栏反打死正常路径）。**三者是同一改动的三半，拆分排期会产生"只改一半"的中间态** | 0.5–0.6 人日 | **极高**：把"崩一次=软件废掉"降为"崩一次=重试"，并把隐性不变量固化为显式约束 |
| 3. WebDAV 强制 HTTPS | `webdav.rs` `webdav_target()` (540) | 0.2 人日 | **高** |
| 4. 云端 OCR `api_url` 强制 https | `ocr.rs:169` | 0.2 人日 | **高**：Bearer Key 明文外传 |
| 5. 收窄 `assetProtocol.scope` | `tauri.conf.json:32` | 0.1 人日 | **高** |
| 6. zip-slip 补绝对路径拒绝 | `restore.rs:203-207` | 0.2 人日 | **高** |
| 7. invoke 名一致性快照测试 | 解析 `client.ts` 70 个 invoke ↔ `main.rs` handler 比对 | 0.5 人日（≈50 行） | **高**：一次性消灭最高频回归（改名漏改） |

### 🥈 第二批：正确性 + 架构纠偏（合计 ≈ 5 人日）
| 动作 | 成本 | 说明 |
|---|---|---|
| 8. 消灭双份默认值真源 | 1 人日 | 修正 `naming.template` / `print.title` 分歧；长期用 `ts-rs` 从 Rust 生成 TS 类型 |
| 9. 加 `CoreError::Cv(#[from] opencv::Error)` | 0.5 人日 | 一次性消掉 ~44 处 `map_err` 样板 |
| 10. 命令层结构化错误 `ApiError{code,message}` | 1.5 人日 | 解锁前端差异化处理（如 Locked → 引导解锁期间） |
| 11. P0-1 根治越层（4 项动作） | 2.5 人日 | ① 补 facade/query service（`services::query::{list, stats, neighbor, search}` 返回稳定 DTO），命令层改调 `services::*`——**主动作**；② CI 固化禁止 `use voucher_core::db::repos`；③ `with_conn`/`with_tx` 加 `# Panics` 文档注释；④ RAII 重入护栏 + 回归测试（**终版实施规格见附录 A**）。前两条治越层，后两条防它演变成死锁 |

### 🥉 第三批：消重复 + 补测试网（合计 ≈ 8.5 人日）
| 动作 | 成本 |
|---|---|
| 12. 引入 ESLint + Prettier + eslint-plugin-vue（纳入 CI） | 1 人日 |
| 13. clippy 覆盖 `src-tauri`（CI 加 `cargo clippy --workspace`） | 0.1 人日 |
| 14. WebDAV mock server 测试（httptest：401/5xx 重试/补传） | 2 人日 |
| 15. DB 损坏 / 异常输入容错测试 + 并发与锁回归测试 | 2 人日 |
| 16. 前端组件测试基建（`environment:"jsdom"`）+ 3 场景 | 1.5 人日 |
| 17. 抽 `useSecretSave()` / `From<&EnhanceConfig>` / `to_gray_owned()` / `query_grouped()` | 1.5 人日 |
| 18. 静默跳过改 `#[ignore]`（5 处）+ `App.vue` listen catch | 0.4 人日 |

### 第四批：扩展性 + 债清理（合计 ≈ 8 人日）
| 动作 | 成本 |
|---|---|
| 19. `trait OcrEngine` + `OcrOutput{lines,capabilities}` | 1.5 人日 |
| 20. pdf.rs 抽表引擎（**下次改 PDF 排版时顺手做，不单独立项**） | 1.5 人日 |
| 21. `ort` 升级正式版（可能 breaking，越早越便宜） | 1–2 人日 |
| 22. `v-model` 单向数据流改造（23 处） | 1.5 人日 |
| 23. 拆分大文件（DashboardPage / pdf.rs / VoucherFormPage） | 1.5 人日 |
| 24. 备份加密改 AEAD + 路径校验补全 + 删 `.archive-*` | 1 人日 |

**总计约 24 人日**（不含第四批则为 16 人日）

---

## 七、建议实施顺序

```
立即（本周）   ├─ 第一批止血 7 项 ≈ 2 人日    → 消 panic + 3 个安全面 + 可用性雷
              └─ 第二批第 8/9 项 ≈ 1.5 人日   → 修实际分歧（2 处配置已不一致）
本迭代        ├─ 第二批 10/11 项 ≈ 3.5 人日   → 契约保护 + 架构纠偏
              └─ 第三批 12–18 项 ≈ 8.5 人日   → 工程化 + 测试网 + 消重复
下迭代        └─ 第四批 19–24 项 ≈ 8 人日     → 扩展性 + 债清理
```

**关键建议**：第一批 7 项合计 **< 2.5 人日**，却消除本项目**唯一已知可 panic 点、3 个外部可利用安全面**和**最严重的可用性故障模式**，建议立即排入下个迭代。
第 20 项（pdf.rs）请按修正后的真实形态重新评估——它不是"拆 700 行函数"，而是"抽一个列表渲染器"，实际收益中等，**不建议单独立项**。

---

## 八、风险登记

| 风险 | 等级 | 触发条件 | 缓解 |
|---|---|---|---|
| `webdav.rs` 字符边界 panic | **高** | 非 ASCII PROPFIND 响应 | 第一批第 1 项 |
| Mutex 中毒致全应用不可用 | **高** | 任一服务 panic | 第一批第 2 项（1 行/处） |
| 凭据明文外传（WebDAV + OCR） | **高** | 用户填 `http://` | 第一批第 3/4 项 |
| 云备份静默失败 | **高** | 网络/鉴权异常，0 自动化监控 | 第三批第 14 项 + 启动补传告警 |
| 前后端契约改名漏改 | **高** | `client.ts`（改 24 次）与命令不同步 | 第一批第 7 项（≈50 行快照测试） |
| 配置双真源分歧 | **中高** | 已实际发生 2 处 | 第二批第 8 项 |
| 备份并发产生损坏 zip | 中高 | 手动/定时/退出前备份并发 | 第三批第 15 项 |
| DB 损坏无降级路径 | 中高 | 介质损坏，`open()` 直接报错 | 第三批第 15 项 |
| `ort` RC 版 breaking | 中 | 上游发正式版 | 第四批第 21 项 |
| 测试"假绿"掩盖回归 | 中 | 精简 CI 环境 | 第三批第 18 项 |

---

## 附：体检方法与局限

- **架构/质量**：全量 Grep 模式统计 + Top 命中文件抽样取证（未获 Bash 权限，未跑 `cargo clippy` / `udeps` / `geiger`）
- **测试**：静态统计 + **前端实测**（`npx vitest run` → 4 文件 25 用例全绿 2.29s）；后端未本地执行（需 OpenCV 4.x + libclang，以 CI 配置为准）
- 代码质量由两位工程师独立评估后合并去重，评分差异（72 vs 83）源于权重侧重，问题清单取并集
- 若需验证完整性，可跑 `cargo clippy -- -W unwrap_used -W expect_used` 核验锁中毒相关条目
