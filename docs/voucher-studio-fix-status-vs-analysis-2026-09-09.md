# Voucher Studio 修复状态评估 — 与分析报告对照

> 评估时间：2026-09-09 20:05 ｜ 基线 HEAD：`063f919`（v0.13.1，分析报告落笔点）｜ 当前 HEAD：`cabcbee`（+17 笔修复提交，版本号仍 0.13.1）
> 对照对象：《voucher-studio-project-analysis-2026-09-09.md》《voucher-studio-health-check-2026-09.md》

---

## 〇、结论先行（TL;DR）

**体检报告四批改进清单共 24 项：已完成 14 项、部分完成 4 项、按建议搁置 1 项、未做 3 项；第一批（止血）与第二批（正确性纠偏）全部落地。** 分析报告指出的 3 处口径修正（越层 ~20、生产 unwrap 10、`.archive-*` 不在库）与 Top6 风险项已全部按计划处置。

**本评估额外发现并修复 1 处真实缺陷**：2f80819 引入的组件测试样例 `VoucherNavBar.test.ts` 导入路径错误（`../voucher/` → `../`），导致该套件加载失败——修复后前端 **8 文件 43 用例全绿**（此前 37 过 + 1 套件挂）。

| 批次 | 状态 | 说明 |
|---|---|---|
| 第一批 止血速赢（7 项） | ✅ 全部完成 | 7 笔提交 + 门禁修复，QA 门禁复核通过 |
| 第二批 正确性纠偏（#8–#11） | ✅ 全部完成 | 配置真源、CoreError::Cv、ApiError、越层根治 |
| 第三批 消重复+补测试网（#12–#18） | 🟡 4 完成 / 2 部分 / 1 缺 Prettier | ESLint、clippy 全 workspace、WebDAV mock、DB 容错、消重复、假绿治理 |
| 第四批 扩展性+债清理（#19–#24） | 🟡 1 完成 / 2 部分 / 3 未做 | OcrEngine ✅；大文件拆分、路径穿越、删 archive 部分 ✅ |
| 文档失真回写 | ✅ 全部落地 | README 4 处、验收基线清单1/H节、开发方案注记、两份报告入库 |

---

## 一、批次清单逐项对照（体检报告"六、统一改进清单"）

### 第一批（7/7 ✅）
| # | 项 | 提交 | 验证 |
|---|---|---|---|
| 1 | webdav.rs 字符边界 panic | `b71ba2d` 字节级 ASCII 不敏感搜索 | ✅ 回归单测（İ/中文 PROPFIND） |
| 2 | 锁中毒恢复+RAII 重入护栏（勿拆） | `294c58a` 7 处 into_inner+log、ReentryGuard(Drop)、#注意 文档 | ✅ 嵌套重入/poison 恢复 2 测试 |
| 3 | WebDAV 强制 HTTPS | `b71ba2d` 回环放行、两入口收口 | ✅ http 非回环拒绝用例 |
| 4 | 云端 OCR 强制 https | `294c58a` validate_api_url | ✅ 请求前校验+用例 |
| 5 | 收窄 assetProtocol | `e9a9d1c` `["**"]`→`["$DATA/VoucherStudio/attachments/**/*"]` | ✅ convertFileSrc 实证 7 处全覆盖 |
| 6 | zip-slip 绝对路径拒绝 | `b81daf3` ensure_safe_rel_entry | ✅ 9 类恶意条目用例 |
| 7 | invoke 一致性快照测试 | `e56400f` client 65==Rust 65 | ✅ 契约测试绿 |

### 第二批（4/4 ✅）
| # | 项 | 提交 | 说明 |
|---|---|---|---|
| 8 | 双默认值真源 | `b500e42` | FE 对齐 Rust：`{date-}-{n}`/「原始凭证」；`src/config-defaults.ts` 单一真源+3 守卫测试 |
| 9 | CoreError::Cv | `644f3e3` | enhance_geom/pixel ~42 处 map_err 样板消除 |
| 10 | ApiError{code,message} | `8fd7f26` | 命令层全量迁移 + `src/api/errors.ts`（asApiError/errMsg）+ 3 页面适配 + errors.test.ts |
| 11 | 根治越层 | `b39463c` | `services::query` 门面（稳定 DTO、serde 形状不变）；**越层 21→0**；CI Layering guard |

### 第三批（#12–#18）
| # | 项 | 状态 | 证据 |
|---|---|---|---|
| 12 | ESLint + Prettier | 🟡 **ESLint 已落地，Prettier 未引入** | `eslint.config.js`(flat/essential) + CI Lint 步骤 + 修 3 真问题；本地 `npm run lint` 0 错；package.json 无 prettier |
| 13 | clippy 覆盖 src-tauri | ✅ | ci.yml `cargo clippy --workspace`（注释引 Q-28） |
| 14 | WebDAV mock 测试 | ✅ | `tests/webdav_mock.rs`（240 行） |
| 15 | DB 损坏/并发容错测试 | ✅ | `tests/db_resilience.rs`（142 行） |
| 16 | 前端组件测试基建+3 场景 | 🟡 **基建落地，样例 1 个**（VoucherNavBar 6 用例） | 目标 3 场景，待补 |
| 17 | 消重复四件套 | ✅ | `useSecretSave`（BackupTab/SecretsTab 复用）、reports `query_grouped`、`From`/灰度收口（`1435928`） |
| 18 | 假绿改 #[ignore]+App.vue listen | ✅ | 5 测试 `#[ignore]`+CI `--include-ignored` 真跑；App.vue `safeListen` 单点隔离 |

### 第四批（#19–#24）
| # | 项 | 状态 | 证据 |
|---|---|---|---|
| 19 | trait OcrEngine | ✅ | `ocr.rs:142 trait OcrEngine`（`1eb63ef`） |
| 20 | pdf.rs 抽表引擎 | ⛔ **未做（按体检建议搁置）** | pdf.rs 自基线 0 提交；原文"下次有 PDF 排版需求顺手做，不单独立项" |
| 21 | ort 升级正式版 | ⛔ 未做 | `Cargo.toml:34` 仍 `=2.0.0-rc.13` |
| 22 | v-model 单向流改造（23 处） | ⛔ 未做 | 当前 `v-model="cfg."` 仍 23 处 |
| 23 | 拆分大文件 | 🟡 **两页已拆**：DashboardPage 826 行瘦身（TrendPanel 549/TopListsPanel 234/ReconcilePanel 118，`1eb63ef`）；VoucherFormPage 拆 4 子组件（`525e828`） | pdf.rs 未动（归 #20） |
| 24 | AEAD + 路径校验 + 删 archive | 🟡 路径穿越防御 ✅（`paths.rs` +69 收口）；`.archive-*` 本地已清 ✅ | **备份加密仍 WinZip AES-256 非 AEAD**（`services/backup/crypto.rs` 未改） |

**汇总：14 完成 / 4 部分（#12#16#23#24）/ 1 按建议搁置（#20）/ 3 未做（#21#22 + #24-AEAD）**

---

## 二、Top6 风险项对照（分析报告"八、风险登记"）

| 风险 | 分析报告评级 | 现状 | 处置提交 |
|---|---|---|---|
| WebDAV 字符边界 panic | 🔥高 | ✅ 已修 | b71ba2d |
| Mutex 中毒全库不可用 | 🔥高 | ✅ 已修（+RAII 护栏） | 294c58a |
| 凭据明文外传（WebDAV/OCR） | 高 | ✅ 已修（回环例外） | b71ba2d/294c58a |
| 契约改名漏改 | 高 | ✅ 快照测试锁死（65=65 仍绿，ApiError 迁移未破契约） | e56400f/8fd7f26 |
| 配置双真源分歧 | 中高 | ✅ 已修+守卫测试 | b500e42 |
| zip-slip + assetProtocol 全开 | 中高 | ✅ 已修 | b81daf3/e9a9d1c |

## 三、口径修正核对（分析报告定案值）

| 修正项 | 报告定案 | 现状 |
|---|---|---|
| 越层直连 DB | ≈20 命中行 | ✅ **0**（b39463c + CI guard 固化） |
| 生产路径 unwrap/expect | 10 处 | ✅ 锁相关 7 处已随 #2 消除，写法对齐 into_inner |
| `.archive-*` 不在版本库 | 本地 6 目录 >1MB | ✅ 本地已删除（gitignored，无需提交） |

## 四、文档失真回写核对

| 文档 | 状态 | 证据 |
|---|---|---|
| README 微信监听行 | ✅ 已删 | b9136ce |
| README 五步管线/每页4卡 | ✅ 回写为七步/1-2-3列 | b9136ce |
| README 用例数 120+ | ✅ 更新（178→202，实际静态计数 204） | b9136ce |
| 验收基线清单1（45 用例/H节微信✅） | ✅ 回写 | 2f80819 |
| 开发方案微信移除注记 | ✅ | 2f80819 |
| 体检/分析报告入库 | ✅ 已纳入版本库（d63d3e7） | d63d3e7 |

## 五、测试现状（实测）

| 层 | 用例 | 实测 |
|---|---|---|
| 前端 | **43 用例 / 8 文件** | ✅ 本轮实测全绿 2.25s（含修复后的 VoucherNavBar 6 用例）；typecheck、ESLint 通过 |
| Rust（静态计数） | 单元+集成 **≈204**（#[test]/#[tokio::test]） | ⚠️ 本地 cargo test 仍受沙箱 MSVC link.exe BOM 缺陷阻塞；CI 已配 `--include-ignored` 全量执行，**须 CI 绿后闭环** |
| 测试网新增 | webdav_mock.rs、db_resilience.rs、errors.test.ts、invoke-contract、config-defaults、VoucherNavBar | — |

## 六、本评估中修复的缺陷

- **cabcbee** `fix(test): 修正VoucherNavBar组件测试导入路径`：`src/components/voucher/__tests__/VoucherNavBar.test.ts:10` 原 `../voucher/VoucherNavBar.vue`（错误解析到 `voucher/voucher/`）→ `../VoucherNavBar.vue`。修复前该套件文件级加载失败（37 过+1 挂），修复后 43 用例全绿。

## 七、残余清单与下一步建议

**未做（建议排 v0.14）**：
1. #22 v-model 单向数据流改造（23 处，Q-18 配套深拷贝 hack 一并处理）
2. #21 ort RC→正式版（可能 breaking，越早越便宜）
3. #24-AEAD 备份加密改造（WinZip AES-256 无 MAC，密文可篡改——**安全相关，建议优先**）
4. #16 组件测试补至 3 场景；#12 如需可补 Prettier

**收口动作**：
- 17 笔修复已实质构成 v0.14 内容（含第四批部分），但版本号仍 0.13.1 → 建议 bump 至 0.14.0 后提交
- Rust 190+ 用例本地未能执行（环境），**请在 CI 或健康 Windows 机跑一次全量 `cargo test --workspace` 复核**后认为闭环
- 分析报告与本文档均可同步更新"批次状态"

---

## 附：验证方法与局限

- 状态经 git log/show 逐提交核对 + 本地实测（vitest / typecheck / eslint）；Rust 侧因沙箱 MSVC 链接器缺陷未本地执行，以静态计数+提交证据为准。
- 提交未推送远端；本地 main 分支 17 笔修复均在 `063f919` 之上、工作区干净。
