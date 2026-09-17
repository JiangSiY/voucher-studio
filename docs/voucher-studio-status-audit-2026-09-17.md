# voucher-studio 现状审计（2026-09-17）

> 审计对象：`F:\开发\voucher-studio`（Tauri 2 + Vue 3 + Rust 凭证管理桌面应用，v0.14.0）
> 审计基线：HEAD `14ea9fe`（2026-09-09 22:26 提交、22:41 推送），工作区自发布后 8 天零改动
> 审计方式：只读取证（代码 / 文档 / 文件系统逐条交叉核对）＋ 本地全量门禁实测复核
> 对照对象：`docs/voucher-studio-health-check-2026-09.md`、`docs/voucher-studio-project-analysis-2026-09-09.md`、`docs/voucher-studio-fix-status-vs-analysis-2026-09-09.md`、`docs/开发方案.md`、`docs/验收基线/清单1~3`
> 取证日志：`F:\开发\.audit-2026-09-17\`（仓库外，未入库）

---

## 〇、结论先行（TL;DR）

**版本冻结健康、发布链路完好、契约与分层无漂移；本次审计发现 1 处真实安全缺陷与 1 处读数失真，均已当场修复并通过全量门禁。**技术债集中在「发布门过期」（G3 走查停在 v0.6.5）与「阶段 6/7 未开始」。

| 维度 | 结论 | 关键证据 |
|---|---|---|
| 版本与同步 | ✅ v0.14.0 三处版本号一致；HEAD 已推送；审计前工作区干净 | `package.json:4` / `Cargo.toml:7` / `tauri.conf.json:4`；`.git/logs/refs/remotes/origin/main` 末行 `update by push` |
| 门禁实测 | ✅ 前端 11 文件 59 用例 + typecheck/lint/build 全绿；Rust **212 用例 0 失败 0 忽略**；fmt/clippy 0 告警 | 本次实测；日志见附录 A |
| 安全 | 🔴→✅ 1 处真缺陷：WebDAV 回环判定 userinfo 绕过（可致凭据明文外发）**本次已修 + 回归测试** | `services/backup/webdav.rs:605`；对照 `services/ocr.rs:261-277` 的正确口径 |
| 契约与分层 | ✅ IPC 65==65 逐名相等；命令层越层 0；配置默认值单一真源成立 | `src/api/client.ts` 65 调用点 ↔ `#[tauri::command]` 65；`ci.yml:87-92` guard |
| 文档一致性 | 🟠 5 处失真（1 处已修，2 处待回写，2 处为历史快照过期） | 见 3.2 节 |
| 发布门 | 🟠 G3 走查停在 **v0.6.5（2026-08-22）**，落后 65 笔提交 / 151 文件；清单 5 项失真；无独立走查记录表，截图链断裂 | `清单2:97,149`；`git rev-list --count v0.6.5..HEAD`=65 |
| 技术债 | 🟡 ort 精确 pin RC、Prettier 未引入、无 v0.14.0 tag、阶段 6 零完成记录、阶段 7 未开始 | `crates/voucher-core/Cargo.toml:41`；`src-tauri/Cargo.toml:20-23` |
| 卫生 | 🔵 `target/` 59G（其中历史安装包 1.4G）、`F:\开发` 根约 1.1G 安装包与日志、G3 走查资产未入库 | 见 D 组清单 |

**三句话**：① 代码质量与工程纪律维持在 9/9 修复批次后的水平，没有回退，实测比文档写得更干净（用例数、告警数）；② 唯一实质风险是本次新发现、且已修掉的 WebDAV 回环判定缺陷——它把「https 强制」这道 9/9 才建立的门给绕过了；③ 真正卡住下一站的是 G3 走查：它 26 天前在 v0.6.5 上通过，而 v0.14.0 引入了 AEAD、ApiError 迁移、assetProtocol 收窄等多项高风险变更，从未在真机上复验。

### 本次已修（2 项）

| # | 项 | 提交前状态 | 修复 |
|---|---|---|---|
| F1 | WebDAV 回环判定 userinfo 绕过（P0） | `http://127.0.0.1:9999@evil.com/dav` 被判为回环、放行 http，随后 Basic 凭据明文发往 evil.com | 判定改走 `reqwest::Url::parse` + `host_str()`（与 `ocr.rs` 统一），解析失败从严；新增 5 个恶意 URL 反例 + 2 个合法回环用例 |
| F2 | README 用例数失真（P2） | 写 118 单元 + 91 集成 = 209 | 改为实测静态计数值（含 F1 新增用例后为 121 + 91 = 212，并标注口径） |

---

## 一、取证口径

1. **静态计数**：`#[test]` / `it(` 属性行计数（非运行时计数）。运行时数字另以 `cargo test` / `vitest` 实测输出为准，两者本次完全吻合（见附录 A）。
2. **越层与危险调用**：以 `grep -rnE` 精确匹配，并做事后 sanity 校验（同一命令族在同文件上的命中数非零，证明 grep 生效）。
3. **环境**：MSVC 14.44.35207（Hostx64\x64）+ Windows SDK 10.0.26100.0 + LLVM + OpenCV 4.10，配方源出仓库既有 `activate-dev.ps1` 与 `F:\开发\dev-env.cmd`。
4. **一处历史结论翻案**：9/9 记录「本地 cargo test 受沙箱 `link.exe` 被 Git Bash coreutils 假身抢占而阻塞，需 CI 复核」。本次以 `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER` **显式指向真实 MSVC 链接器**（而非依赖 PATH 优先级——PATH 前置后 `which link.exe` 仍返回 `/usr/bin/link.exe`），本地全量测试一次通过。该阻塞结论不再成立。

---

## 二、实测快照（2026-09-17）

| 项 | 实测值 |
|---|---|
| 版本 | 0.14.0（`package.json:4`、`Cargo.toml:7`、`tauri.conf.json:4` 三处一致；运行时 `env!("CARGO_PKG_VERSION")` 渲染同值） |
| HEAD | `14ea9fe`（`docs: 修复状态与基线对照评估报告入库`，2026-09-09 22:26:27 +0800） |
| 远端同步 | 已推送（`origin/main` 跟踪引用更新至 `14ea9fe`，2026-09-09 22:41 +0800） |
| 本地 tag | 止于 `v0.13.1`——**无 `v0.14.0` tag** |
| 工作区 | 审计前干净（`git status --porcelain` 空，`ls-files` 267 个文件） |
| Rust 用例 | **212**（单元 121 + 集成 91）；`--include-ignored` 下 5 个原 `#[ignore]` 用例全部真跑并通过 |
| 前端用例 | 11 文件 **59** 用例（vitest 2.36s） |
| 门禁 | `fmt --all --check` ✅／`clippy --workspace --all-targets -D warnings` ✅（31.6s）／`cargo test` ✅／`vue-tsc` ✅／ESLint ✅／`vite build` ✅（1.09s） |
| Rust 规模 | 21,298 行（voucher-core/src 14,595 + tests 4,264 + examples 764 + src-tauri 1,413 + spike 259） |
| 前端规模 | Vue 7,304 行 + TS 1,242 行（另有测试 844 行、CSS 1,757 行） |
| 最大源文件 | `tests/voucher_flow.rs` 1,867；`src/services/pdf.rs` 1,492；`tests/enhance_e2e.rs` 1,022；`src/services/backup/webdav.rs` 944 |
| 关键依赖 | tauri 2.11.5／printpdf 0.7.0／opencv 0.100.1／ort **2.0.0-rc.13**（`=` 精确 pin）／age 0.11.5／rusqlite 0.32.1／zip 2.4.2；vue 3.5.41／vite 6.4.3／vitest 3.2.7／typescript 5.7.3 |

---

## 三、三线取证结论

### 3.1 代码与测试实况

**测试分布（Top）**：`tests/voucher_flow.rs` 39、`tests/enhance_e2e.rs` 24、`services/ocr_fields.rs` 21、`services/backup/webdav.rs` 12（含本次新增 1）、`tests/backup_flow.rs` 10、`services/ocr_layout.rs` 9、`services/ocr_validate.rs` 8、`services/backup/crypto.rs` 7。

**生产路径 `unwrap`/`expect`：7 处（逐条可判安全）**——全仓原始 395 次中，369 次落在 `tests/`、`examples/`、`spikes/` 与注释里。

| 位置 | 判定 |
|---|---|
| `models/voucher.rs:98` | 千分位格式化，仅在 `n > 0` 分支取首段，安全 |
| `backup/webdav.rs:295,372,459,651` | `Method::from_bytes(b"MKCOL"/b"PROPFIND")` 常量字节，不可失败 |
| `services/ingestion.rs:185` | 外层 `if enhanced_rel.is_some()` 守护，安全但脆（注释已标） |
| `src-tauri/src/main.rs:288` | 启动期 `.expect("Voucher Studio 启动失败")`，显式失败可接受 |

`panic!`/`todo!`/`unimplemented!` 在生产代码 **0 处**（4 处命中全在 examples 与 `#[cfg(test)]` 内）。

**分层越层：0 命中**。`src-tauri/src` 下 `db::repos`、`.with_conn(`、`.with_tx(`、乃至 `conn`/`repos` 字面词均零命中；命令层统一经 `services::query` 等门面。

CI 的 guard（`ci.yml:87-92`）为内联 grep，有 4 个已知盲区：① 不覆盖 `voucher_core::db::connection::open(...)`；② 不覆盖 `Database::new` 与直接 `use rusqlite`；③ 不覆盖 `db::{repos::x}` 大括号写法；④ 只扫 `src-tauri/src`（`scripts/` 为空目录，无独立脚本）。

**安全实现现状（四项均落地）**

| 项 | 位置 | 备注 |
|---|---|---|
| WebDAV 强制 https（回环例外） | `backup/webdav.rs:586-602` | 🔴 本次修复前回环判定可被 userinfo 绕过（见 P0-1） |
| 云端 OCR `api_url` 强制 https | `services/ocr.rs:261-290` | 使用 `Url::parse().host_str()`，**口径正确**，测试覆盖 6 类输入；携带 Bearer Key 前校验 |
| zip 解包拒绝绝对路径/`..` | `backup/restore.rs:256-270` | 拒绝整包而非静默跳过；覆盖 `\` 分隔、UNC/盘符、混合 `..`；4 个反例用例 |
| 备份加密 age 信封 | `backup/crypto.rs:31-139`、`BACKUP_VERSION = 4`（`backup/mod.rs:44`） | 写侧 age passphrase 信封 + 同目录临时文件 rename；读侧三态兼容；取不到密码即失败，不静默降级明文 |
| assetProtocol scope | `src-tauri/tauri.conf.json` | `["$DATA/VoucherStudio/attachments/**/*"]`——已是收窄后的值 |

**CI 门禁（2 job / ubuntu-latest）**：fmt ✅、clippy(-D warnings, --workspace --all-targets) ✅、test(--include-ignored) ✅、layering guard ✅、typecheck ✅、ESLint ✅、vitest ✅、vite build ✅。
**缺失**：依赖漏洞扫描（cargo-audit / npm audit）、覆盖率门禁、双平台矩阵（无 Windows/macOS runner，而产品是桌面应用）、`--locked` 可复现构建、action 未按 SHA 固定、无 `tauri build` 打包验证。

### 3.2 文档与代码一致性

**失真 5 处**

| # | 位置 | 文档原值 | 实测 | 处置 |
|---|---|---|---|---|
| 1 | `README.md:51` | 118 单元 + 91 集成 = 209 | 121 + 91 = **212**（含 F1 新增） | ✅ 本次已修 |
| 2 | `README.md:27` | 本地 OCR「ort 2.0」 | `ort = "=2.0.0-rc.13"`（上游无 2.x stable） | 建议改为「ort 2.0 RC」 |
| 3 | `docs/开发方案.md:181,267` | 「macOS 需 brew install opencv（已记入 README）」 | README 无任何 OpenCV 字样，而 `tauri.conf.json` 打包强依赖 `vendor/opencv_world4100.dll` | 建议补 README 环境要求 |
| 4 | `docs/验收基线/清单1:6,9-11` | 「45 个测试全绿（lib 18 + core_flow 23 + spike 4）」「约 204 个；前端 7 文件 37 用例」 | 42 行映射（38 ✅ + 4 已移除）；当前 212 / 前端 11 文件 59 用例 | 清单1 自带「未随版本滚动」注记，可接受；但「当前基线」措辞应加日期 |
| 5 | `fix-status 报告`5 处结论 | v-model 23 处未做 / AEAD 未做 / pdf 表引擎未做 / 组件测试仅 1 样例 / 版本仍 0.13.1 | 均已在 v0.14.0 完成 | 属 9/9 20:05 快照，建议在报告头部标注「后续已收口」 |

**一致项（本次复核全部成立）**
- IPC 契约：`client.ts` 65 个 `invoke` 调用点 ↔ 65 个 `#[tauri::command]` ↔ `generate_handler!` 65 项，逐名相等；守卫测试 `invoke-contract.test.ts` 只断言集合相等（不写死数字，避免脆断言）。
- 配置默认值单一真源：`src/config-defaults.ts:17,20` ↔ `crates/voucher-core/src/utils/config.rs:65,185`，逐字段全等；3 个守卫测试在位。
- README 功能描述：微信监听（已随 v0.7 移除，README 亦已删尽）、增强管线七步、A4 每页 1/2/3 列 + 附录 2×2 图/页、本地 OCR 默认引擎（`ocr.rs:189` `"local" => LocalEngine`）、age/AEAD 备份描述——均与实现相符。
- 三处版本号一致；`README.md` 无版本号字样；无 CHANGELOG（不构成矛盾）。

### 3.3 遗留项盘点

**A. 发布门（G3 → G4）**
- G3 最后走查为 **v0.6.5（2026-08-22）**，结论「通过（可用范围）40 ✅ / 1 ⚠️ / 4 ⏳」，挂账 #13/#15（云端账号）、#35（WebDAV）、#45（macOS）。⚠️ 为 #6 拖放 OLE 手势（超出自动化能力，待人工一次确认）。
- 该判定与当前 HEAD 的差距：**65 笔提交 / 151 文件 / +23,672 −5,438 行**。v0.14.0 引入的 AEAD 信封、ApiError 全量迁移、assetProtocol 收窄、路径穿越防御、v-model 单向流**均未经真机复验**。
- `清单2` 5 项失真：#8 微信监听（功能 v0.7 已移除仍列 ✅）、#1 体积「≈23MB」（实为 NSIS 38MB）、#2 控制面板版本「0.2.0」（实为 0.14.0）、#3 路径 `%APPDATA%\com.voucherstudio.desktop`（实为 `%APPDATA%\VoucherStudio\data\`，明细段已自行更正而表格未回写）、`清单3` #32 微信监听开关（同源失效）。
- `开发方案.md:407` 称「G3 门走查记录表随附」——`docs/验收基线/` 下**无此文件**，记录实际内嵌于清单2/清单3 表格；且两份清单中 **9 处 `file:///.../.archive-g3-step/...` 截图链接指向的目录已不存在**（全盘无该目录），证据链断裂。存活的相关资产在仓库外：`F:\开发\.archive-g3-debug`（20M）、`F:\开发\G3测试`（11M，含 UIA 脚本与 24 张**合成**凭证图），均未入版本控制。

**B. 挂账（需设备或外部条件）**
| 项 | 来源 | 阻塞原因 |
|---|---|---|
| WebDAV 真实服务器验证（含 v0.14 新增的 age 远端 manifest 往返） | `开发方案.md:438,452`、`清单2` §35 | 无真实 WebDAV 服务器；现有仅 mock（`tests/webdav_mock.rs` 4 用例） |
| macOS 侧 Spike A 复验 + 五项基础链路冒烟 | `开发方案.md:453`、`清单2` #45 | 无 Mac 设备（行动清单第 1 项仍「待办」） |
| 5 张真实凭证图（手机拍 3 + 截图 2） | `开发方案.md:257,362` | 现有全部为合成图（`spikes/ocr-poc/testdata` 24 张），量化验收「识别字段数提升 ≥ 10%」无法执行 |
| 云端 OCR 真机项 #13/#15 | `清单2:105` | 无云端账号 |

**C. 技术债**
- `ort = "=2.0.0-rc.13"` 精确 pin（`crates/voucher-core/Cargo.toml:41`），上游发 stable 后需改 `ocr_local.rs` 约 6 处 API。
- **阶段 6（平台集成）零完成记录**：托盘、开机自启、自动更新未做（`src-tauri/Cargo.toml:20-23` 仅 dialog/opener/single-instance/log 四个插件；单实例✅已做）。
- **阶段 7（加固·试运行·发布 → G4）未开始**：试运行（新版空库直上）、1.0 发布均未发生；`开发方案.md:370` 期望「Vue Test Utils 承接 32 个 GUI 场景语义」——现状 11 个组件/composable 测试套件并非该语义的承接。
- Prettier 未引入（体检 #12 的后半，仍成立）：`package.json` 无 prettier，CI 无 format 步骤（Rust 侧有 fmt）。
- Windows 代码签名未做（`开发方案.md:392` 自评高风险）：`tauri.conf.json` bundle 无签名配置，已发布未签名安装包。
- 前端 vitest 无全局配置：`vite.config.ts` 无 `test` 块、无 `vitest.config.*`，jsdom 靠 6 个文件首行 pragma；`invoke-contract.test.ts` 仅 1 个用例承载 65 命令的契约守门。

**D. 卫生与打扫**
- `target/` **59G**；`target/release/bundle` **1.5G**（msi 878M/15 个文件、nsis 562M/15 个文件，含 v0.6.5–v0.13.1 共 14 组历史安装包约 1.4G，可清）。
- `F:\开发` 根（仓库外）约 1.1G：`LLVM-22.1.8-win64.exe` 434M、`opencv-4.10.0-windows.exe` 174M、`opencv-4.8.0-windows.exe` 168M、`typora-setup-x64.exe` 108M、25 个构建日志约 466K（含 201K 的 `.opencv-build16.log`）。
- 仓库根杂物（均已 ignored）：`app-icon.png`、`sha256sums.txt`、4 个 `.log`、`spikes/print-poc/sample-*.pdf`；`scripts/` 为空目录。
- 已入库大件属有意为之：`src-tauri/vendor/opencv_world4100.dll` 62M（打包资源）、`assets/ocr-models` 16M。

---

## 四、分级清单

### P0 — 安全（1 项，已修）

| # | 项 | 来源 | 现状 | 验收标准 |
|---|---|---|---|---|
| P0-1 | WebDAV 回环判定 userinfo 绕过 → Basic 凭据明文外发 | `services/backup/webdav.rs:605`（修前） | ✅ 已修：改走 `Url::parse().host_str()`；`http://127.0.0.1:9999@evil.com/dav` 等 5 类反例已拒绝，合法回环（含 userinfo）仍放行 | 回归测试 5 反例 + 2 正例通过；真实服务器验证时（挂账 B-1）复测一次 |

> 放大链已核实：`backup/restore.rs:192` 会从备份包恢复 `config` 段，即 `backup.webdav.url` 可由备份包注入——攻击者若能提供备份包并诱导恢复后上传，即可端到端利用。修复后该链闭合。

### P1 — 发布门（4 项）

| # | 项 | 来源 | 现状 | 验收标准 |
|---|---|---|---|---|
| P1-1 | G3 走查对 v0.14.0 重跑 | `开发方案.md:147`、`清单2:97` | 停在 v0.6.5，落后 65 提交 | 按清单2（45 项）+ 清单3（32 场景）在 `target/release/bundle` 的 v0.14.0 安装包上重跑，逐项记录 ✅/⚠️/⏳ |
| P1-2 | 清单2/清单3 失真项回写 | `清单2:11,12,13,23`、`清单3:82` | 5 项失效（微信监听 ×2、体积、版本号、数据目录路径） | 删除或改写为 v0.14.0 实况 |
| P1-3 | 独立《G3 走查记录表》+ 证据链归档 | `开发方案.md:407` | 无独立文件；9 处截图链接指向已删除的 `.archive-g3-step/` | 新建记录表；把 `F:\开发\.archive-g3-debug`、`F:\开发\G3测试` 归档为带日期压缩包并纳入版本控制（`.gitignore` 现排除 `/.archive-*/`，需同步调整） |
| P1-4 | 补 `v0.14.0` tag | 本地 `git tag` | 止于 v0.13.1；`开发方案.md:378`「每阶段结束打 tag」未落实 | `git tag v0.14.0 14ea9fe` 并推送（或先 `git fetch --tags` 确认远端是否已存在） |

### P2 — 技术债（6 项）

| # | 项 | 来源 | 验收标准 |
|---|---|---|---|
| P2-1 | README ort 表述（「ort 2.0」→「2.0 RC」）与 OpenCV 环境要求缺失 | `README.md:27,32-37`；`开发方案.md:181,267` | README 与实现一致，且开发方案对 README 的转述成立 |
| P2-2 | CI 加固：`--locked`、action SHA 固定、依赖漏洞扫描、覆盖率阈值 | `ci.yml:83,96` 等 | 逐项或明确记录「不做」 |
| P2-3 | CI 双平台矩阵（Windows 至少） | `ci.yml:15,42`（仅 ubuntu-latest） | Windows runner 跑通 test 与 `tauri build` 冒烟 |
| P2-4 | Prettier 引入或明确关闭该债 | 体检 #12 后半 | 二选一并落文档 |
| P2-5 | 前端 vitest 全局配置 + 契约守门加厚 | `vite.config.ts`（无 test 块） | 集中 environment/setup；契约测试覆盖参数形状而非仅命令名集合 |
| P2-6 | 越层 guard 盲区收口 | `ci.yml:87-92` | guard 覆盖 `db::connection::open`、`Database::new`、大括号导入三种写法 |

### P3 — 卫生（4 项，建议，不擅自执行）

| # | 项 | 现状 | 建议命令/动作 |
|---|---|---|---|
| P3-1 | 历史安装包 | `target/release/bundle` 1.4G（v0.6.5–v0.13.1） | 保留最新两版，其余删除 |
| P3-2 | `F:\开发` 根安装包与日志 | 约 1.1G（LLVM/OpenCV×2/Typora + 25 个日志） | 移入归档目录或删除（安装包可重新下载） |
| P3-3 | 仓库根杂物 | `app-icon.png`、`sha256sums.txt`、4 个 `.log` | 确认无引用后删除（已 ignored，不影响构建） |
| P3-4 | 空 `scripts/` 目录 | 0 文件 | 删除，或放入 P2-6 的独立 guard 脚本 |

---

## 五、与 9/9 两份报告的差异追溯

| 9/9 报告结论 | 现状 | 判定 |
|---|---|---|
| 用例 178（单元 95 + 集成 83）、前端 4 文件 25 用例 | 212（121+91）、前端 11 文件 59 用例 | 已过期（v0.14 扩展所致，非失真） |
| 越层 ≈20 命中行 → 目标 0 | 0 命中 + CI guard | ✅ 仍成立 |
| 生产 `unwrap/expect` 10 处 | 7 处（逐条可判安全） | ✅ 改善 |
| 「假绿」8 处（ocr_flow 5 + voucher_flow 3） | 5 处 `#[ignore]` + CI `--include-ignored` 真跑，本次本地也全绿 | ✅ 已治理 |
| 配置双真源分歧（Q-5） | 单一真源 + 3 守卫测试 | ✅ 已修 |
| ESLint 已落地、Prettier 未引入（#12） | 同前 | 🟡 仍成立 |
| #16 组件测试「仅 1 样例」 | 3 个组件套件（ConfirmDialog/VoucherCard/VoucherNavBar） | ✅ 已补 |
| #20 pdf 表引擎「按建议搁置」、#24 AEAD 未做、#22 v-model 23 处未做、版本号仍 0.13.1 | 全部在 v0.14.0 完成（`da57272`／`0304b8b`／`7234e6e`／`20dc7f9`） | 报告内该 5 处结论已过期 |
| 「本地 cargo test 受沙箱 link.exe 阻塞，需 CI 复核」 | 本次以显式指定 MSVC 链接器破局，本地全量通过 | ❌ 结论翻案 |
| 文档失真回写 4 处已完成 | README 用例数再次漂移（v0.14 新增 2 个 golden 测试未回写） | 需建立「加测试即更数」的收尾动作 |

**口径修正（相对 9/9「10 处」）**：本次把 `examples/`（49 处）与 `spikes/`（6 处）单列，不计入生产路径；生产路径由 10 处降为 7 处（其中 `main.rs:288` 启动期 expect 属设计选择）。

---

## 附录 A：门禁实测输出（2026-09-17）

```
### 前端（F:\开发\voucher-studio）
npm run typecheck                      exit=0   → .audit-2026-09-17/frontend-typecheck.log
npm run lint                           exit=0   → frontend-lint.log
npm run test                           exit=0   → Test Files 11 passed (11) / Tests 59 passed (59) / 2.36s
npm run build                          exit=0   → built in 1.09s
### Rust（MSVC 14.44.35207 + SDK 10.0.26100.0 + OpenCV 4.10）
cargo fmt --all --check                exit=0   → rust-fmt-after.log
cargo clippy --workspace --all-targets -- -D warnings   exit=0（31.59s）→ rust-clippy-after.log
cargo test --workspace -- --include-ignored             exit=0   → rust-test-after.log
  test result: ok. 121 passed; 0 failed; 0 ignored   (lib)
  test result: ok. 10 passed; 0 failed; 0 ignored    (backup_flow)
  test result: ok. 4 passed; 0 failed; 0 ignored     (db_resilience)
  test result: ok. 24 passed; 0 failed; 0 ignored    (enhance_e2e)
  test result: ok. 5 passed; 0 failed; 0 ignored     (ocr_flow)
  test result: ok. 5 passed; 0 failed; 0 ignored     (spike_a_print)
  test result: ok. 39 passed; 0 failed; 0 ignored    (voucher_flow)
  test result: ok. 4 passed; 0 failed; 0 ignored     (webdav_mock)
  合计 212 passed / 0 failed / 0 ignored
```

对照：审计基线（HEAD `14ea9fe`，未含 F1）实测 211 passed / 0 failed / 0 ignored，与静态计数 120 + 91 = 211 一致。

## 附录 B：本次改动（2 文件）

```
README.md                                            |  2 +-
crates/voucher-core/src/services/backup/webdav.rs    | 48 +++++++++++++++++------
```

1. `webdav.rs` `is_loopback_host`：手写字符串切分 → `reqwest::Url::parse` + `host_str()`（与 `services/ocr.rs::validate_api_url` 同口径），解析失败从严判非回环。
2. `webdav.rs` 新增 `webdav_url_rejects_userinfo_loopback_bypass`：5 个反例（`127.0.0.1:9999@evil.com`、`localhost:8080@evil.com`、`127.0.0.1@evil.com`、`[::1]@evil.com`、`127.0.0.1.evil.com` 子域后缀伪装）+ 2 个正例（回环带 userinfo、主机名大小写不敏感）。
3. `README.md:51`：用例数 209 → 212（含本次新增 1 个），并标注「静态计数」口径。

## 附录 C：取证日志目录

`F:\开发\.audit-2026-09-17\`（仓库外，未入库）：`env.sh`（构建环境）、`frontend-{typecheck,lint,test,build}.log`、`rust-{fmt,clippy,test}.log`（审计基线）、`rust-{fmt,clippy,test}-after.log`（修复后）。
