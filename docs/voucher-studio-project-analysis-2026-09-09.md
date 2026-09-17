# Voucher Studio 项目综合分析报告

> 版本：v0.13.1（HEAD `063f919`）｜ 分析日期：2026-09-09 ｜ 对象：`F:\开发\voucher-studio`
> 方式：软件公司团队 5 角色（架构 / 产品 / 工程 ×2 / QA ×2）并行只读复核，前端测试实测，与当日《健康体检报告》(voucher-studio-health-check-2026-09.md) 交叉印证；未修改任何源码。

> ⚠️ **校正注记（2026-09-17 追加，正文不改）**：本报告是 **v0.13.1 时点快照**。§五 `:104`「Q-1/Q-2/Q-5/Q-6/Q-8/Q-11/Q-12 全部仍在」与 §七 风险登记 Top6 **已全部修复**（v0.13.1→v0.14.0，提交链见 `docs/开发日志.md` §一）；§五 `:124` 的 CI 门禁描述已过期——现为 typecheck→ESLint→vitest→build ／ fmt→clippy(`--workspace --all-targets -D warnings`)→**layering guard**→`cargo test --workspace --no-fail-fast -- --include-ignored`；`:118-123` 的计数（178 / 前端 25）现为 **212 / 前端 11 文件 59 用例**；`:137` 清单1 回写已完成（`2f80819`）。另：**2026-09-17 新增并修复一处安全缺陷**（WebDAV 回环 userinfo 绕过，属 Q-3 同族）。

---

## 〇、结论先行（TL;DR）

**voucher-studio 是一个"本地优先、数据不出机"的财务凭证整理打印桌面工具**（Tauri 2 + Rust + Vue 3 重写，前身为 PySide6 voucher-print）。产品已跑通「拍照导入 → OCR/增强 → 整理 → 打印归档 → 期间封账 → 加密备份」全链路，08-19→09-08 二十天内迭代 13 个小版本，属于**下限高、纪律好、但保障网有空白的 B 级代码库**——与当日体检结论一致，且本次复核后多数口径被进一步精确化而非推翻。

| 维度 | 结论 | 关键依据 |
|---|---|---|
| 产品完整度 | ✅ 高 | 9 路由页 + 命令面板覆盖录入→打印→封账→备份，动线闭环 |
| 架构 | B+ | `FE→SH→CORE` 单向无环；core 对 Tauri 零依赖；代价是命令层越层直连 DB（口径修正 15→**约 20 命中行**） |
| 代码质量 | B | 生产 0 panic、SQL 100% 参数化、凭据走 keyring、前端 0 any；生产 unwrap 实测 **10 处**（修正体检报告"16 处"口径） |
| 测试 | C+ | core 178 用例（质量高）/ 命令桥 0 / WebDAV 联网 0 / 前端 25（实测全绿 2.04s） |
| 文档一致性 | ⚠️ 失真 4 处 | README 微信监听/五步管线/4 卡版式、验收基线"45 用例"等已过时 |
| 健康度 | **B**（建议下一迭代先做"第一批止血"≈2.5 人日） | 消除已知可 panic 点 + 3 个外部安全面 + 1 个可用性雷 |

---

## 一、项目是什么

| 项 | 内容 |
|---|---|
| 定位 | 财务人员整理**原始凭证/记账凭证**的桌面工具：图片导入 → OCR 结构化 → 打印排版 → 期间核对封账 → 备份防丢 |
| 技术栈 | Rust workspace（`crates/voucher-core` 业务核心 + `src-tauri` 壳）+ SQLite(rusqlite/UoW) + ort/ONNX 本地 OCR(PP-OCRv4) + OpenCV 增强 + Vue3/TS/Pinia/Vite + Tauri 2 + Fluent Design |
| 代码规模 | Rust ≈ 14.8k 行（core 13.4k / 60 文件 + 壳 1.45k）；前端 ≈ 10.3k 行（.vue 7.1k + .ts 1.4k，9 页面 + 18 组件） |
| 数据 | 全本地 `%APPDATA%\VoucherStudio`：SQLite + attachments/backups/exports + 凭据入 OS keyring |
| 版本节奏 | 平均 ~1.5 天/版本，每 ~0.1 版一个主题（备份可靠性→前端测试→对账→账务治理→录入质量→加密/命令面板） |

## 二、产品功能地图与用户动线（software-engineer）

**页面清单**（入口统一挂 `WorkbenchLayout.vue` 三组导航，hash 路由；状态仅 config/toast 两个 Pinia store）：

| 页面/路由 | 一句话职责 |
|---|---|
| 仪表盘 `/dashboard` | 统计卡 + 趋势图 + 账户/标签 TOP + 疑似重复 + 账单对账入口，下钻跳列表（1023 行，含手写 SVG 图） |
| 凭证列表 `/vouchers` | 核心列表：筛选/分页/缩略图/批量状态/批量编辑/批量删除/导出，期间锁禁删 |
| 新建/编辑 `/vouchers/new` `/vouchers/:id` | 表单 + 附件：OCR 填单、换图、裁剪旋转(v0.12)、快照历史、摘要联想、疑似重复、Ctrl+S/Enter、Alt+←→（957 行） |
| 全文搜索 `/search` | 编号/摘要/备注/OCR 文本关键词 + v0.11 日期金额组合筛选 |
| 打印中心 `/print` | 选证 + 卡片风格/列数/按天分组 → 预览 → PDF/Excel/CSV 导出 |
| 导出历史 `/exports` | 批次记录、反查批次、打开 PDF |
| 设置 `/settings` | 6 Tab：通用/凭证参数/OCR/扫描矫正/密钥/备份与同步 |
| 备份与恢复 `/restore` | 手动备份、WebDAV 上/下载/测连、恢复演练、恢复=暂存+重启生效 |
| 帮助 `/help` | 三步上手 + 数据/密钥 FAQ |
| 命令面板（全局） | Ctrl+K 页跳转/快速动作/凭证搜索（App.vue 全局注册） |

**5 条主用户动线**：
1. **导入建卡**：拖图/选图 → 后端增强管线自动跑 → MD5 去重 → 一图一凭证 → toast 直达编辑
2. **整理补全**：OCR(本地/云端)结构化填入 → 手动裁剪补救 → Ctrl+S 保存、Ctrl+Enter 保存下一张、Alt+←→ 翻张 → 摘要联想/疑似重复提示
3. **打印归档**：打印中心筛选 → 风格/列数/分组 → PDF 预览 → 系统打印 → 自动置"已导出"+记录批次
4. **封账核对**：账单 CSV 对账(v0.13) → 月/年报 PDF → 期间锁定(v0.11)禁增删改
5. **防丢兜底**：增量备份(快照+附件包) + WebDAV 云端 once_daily + AES-256 加密 → 异常退出恢复引导

**值得肯定**：导入完成直达核对、编辑可同筛选连续翻张，动线设计连续；快捷键覆盖完整。

## 三、架构分析（software-architect）

**分层与模块地图（实测）**：

```
FE (Vue 3, 9 pages/18 comps, api/client.ts 605行集中桥, 70 invoke)
  ↓ 强契约弱（repo 类型泄漏 + 错误抹成 String）
SH (src-tauri, 65 命令 6 组: vouchers·ingest·export·backup·enhance·settings, ≈1.45k 行)
  ↓ 命令层存在越层（见 A-1）
CORE (crates/voucher-core, ≈13.4k 行, 对 Tauri 零依赖)
  ├─ db/     connection·schema·9 repos·UnitOfWork(unit_of_work.rs:12-37, with_tx/with_conn)
  ├─ models/ 6 模型
  ├─ services/ 24 服务: ocr·enhance·reports·restore·recovery…
  ├─ backup/ 6 子模块: local·format·retention·restore·webdav·crypto
  └─ utils/  config·paths·crypto(keyring)·file_lock(唯一 unsafe 生产点 libc::flock)
```

**分层纪律复核：A-1..A-10 全部仍成立（行号零漂移，佐证 09-09 后无修复提交）**，仅 1 处口径需修正：
- core 对 Tauri **零依赖**仍成立（全 core 仅 tests/enhance_e2e.rs:213 一处**注释**提及）
- 越层直连 DB：严格 `db::repos::` 引用 12 处 + 另 8 处 `.with_conn` 裸连接调用 → 建议口径 **"约 20 处命中行"**（原报告"15 处"漏计多行接收方，性质不变）
- 其余：OCR 无 trait（字符串 match，ocr.rs:131）｜ 错误抹成 String 38 处 ｜ 配置裸 String 枚举 ｜ 事件弱契约散落 main.rs+10+.vue ｜ pdf.rs 双表引擎复制粘贴（~150 行、magic number 30/50/70/92/114）——均与体检报告逐行吻合

**设计亮点子系统（确认在位）**：
- **OCR**：`file_hash` 缓存（同图不重复识别，ocr.rs:115/121）+ 低置信度 UI 显式标注
- **增强管线**：`opt_step` 单步失败跳过 + trace 记录（enhance.rs:137），教科书级稳健；v0.6.1 起实际 7 步（README 仍写五步）
- **备份**：`backup_guard` Mutex 串行化（lib.rs:102）+ `drill_backup` 演练（restore.rs:117）+ warnings 显式部分成功——**全仓设计最完整子系统**

**v0.14 扩展性最卡点**：换 OCR 引擎（无 trait + ort 锁 RC）、加第三套打印版式（表引擎第三次复制 150 行）、加新备份目标（无 BackupTarget 抽象 + WebDAV 780 行 0 联网测试）。

## 四、版本演进（software-qa-engineer-2，取自 .git/logs/HEAD）

| 版本带 | 主题 |
|---|---|
| v0.1–0.4 | 骨架 → 3-2-1 备份 → 打印 PoC → 一图一凭证 + 日报 PDF |
| v0.5.x–0.6.6 | 导入链路/OCR/增强管线 + 缺陷修复 |
| v0.7.x | 智能记账/报表 → WebDAV 云备份 → 真服务器加固 → **微信目录监听移除**(50d2adb) → 打印中心/自动备份/日志轮转 |
| v0.8–0.9 | 备份可靠性五项 + 云端/本地增量(快照+内容寻址附件包) → **前端测试落地** |
| v0.10–0.13.1 | 对账出口三件套 → 期间锁/年季报/组合筛选 → 裁剪旋转/OCR 校验 → **备份 AES-256 加密 + 账单对账 + Ctrl+K 命令面板** |

09-09 体检后**零源码提交**（reflog 止于 `063f919`），体检文档本身亦未入库。

## 五、代码质量核验（software-engineer-2）

**✅ 达标项抽验 5/5 一致**：SQL 100% 参数化（`?N`+ESCAPE）｜ 生产 0 panic! ｜ 凭据走 keyring ｜ 前端 0 any/@ts-ignore ｜ 依赖零冗余。

**🔴 Top 问题现状抽验：Q-1/Q-2/Q-5/Q-6/Q-8/Q-11/Q-12 全部仍在**（webdav.rs:480 to_lowercase 越界切片、unit_of_work.rs:25/31 锁中毒、SettingsPage↔config.rs 双默认值真源、zip-slip、assetProtocol 全开、前端无 ESLint、reports.rs 复制+magic index）。其余 Q-# 收录于体检报告，本轮未逐条重验。

**两处口径修正（定案值，供替换体检报告）**：

| 项 | 体检报告原值 | 本次实测 | 建议 |
|---|---|---|---|
| 生产路径 unwrap/expect | "16 处" | **10 处**（9 unwrap：lib.rs:91/97/108/118、unit_of_work.rs:25/31、models/voucher.rs:98、ingestion.rs:193、ocr.rs:153 + main.rs:288 expect） | 按 10 处定案 |
| 越层直连 | "15 处" | **约 20 命中行**（12 `db::repos::` + 8 `.with_conn`） | 按 ~20 定案 |
| 健康报告 Q-30 `.archive-*` | "10 个目录 ≈1MB 在版本库内" | **失真**：0 个在 git index（已被 .gitignore + 清理提交排除）；本地 6 个目录，g3-step 含 ~500 张压测图 ×2（约 7.7MB+） | 按本地 6 个、>1MB 修正 |

## 六、测试覆盖与质量门禁（software-qa-engineer）

| 层 | 用例 | 实测/状态 |
|---|---|---|
| voucher-core 单元 | 95 | 静态计数 |
| voucher-core 集成 | 83 | voucher_flow 39 / enhance_e2e 24 / backup_flow 10 / ocr_flow 5 / spike 5 |
| src-tauri（65 命令） | **0** | 门禁盲区 |
| 前端 | **25**（4 文件） | **本轮实测全绿，2.04s，EXIT=0** |

- 分布与体检报告**完全吻合、零漂移**（178 / 0 / 25）
- **CI 门禁（.github/workflows/ci.yml）**：frontend job = typecheck→vitest→build（**无 lint**）；rust job = fmt check→`clippy -p voucher-core`→`cargo test -p voucher-core`——确认 **src-tauri 无 clippy 无测试门禁、前端无 ESLint**
- **"假绿" 8 处复核在位**（eprintln 静默跳过）：ocr_flow.rs 5 处 + voucher_flow.rs 3 处（缺模型/字体时 `return Ok(())`）；CI 装 fonts-noto-cjk 且模型已入库（assets/ocr-models ≈16MB）故 CI 真执行，风险主要在开发机
- **最值得先补的 3 个空白**：① 命令桥 + client.ts 契约（改动最频却 0 测试，最低成本 = invoke 名快照比对）② WebDAV 联网函数（需 mock HTTP 覆盖失败/重试/补传）③ DB 损坏/并发容错（0 用例，backup_guard/with_tx 零验证）

## 七、文档一致性核对（software-qa-engineer-2 + software-engineer）

**失真清单（4 处主要）**：

| 文档位置 | 声称 | 实际 | 处理 |
|---|---|---|---|
| README:17 | 微信目录监听自动入库 | 功能已于 50d2adb 移除，代码 0 处实现（现存"微信"= 账单 CSV 对账） | 删除该特性行 |
| README:13 | "五步增强管线（可选去噪/锐化）" | v0.6.1 起实为 **7 步**，去噪/锐化已实装非可选 | 改为 7 步 |
| README:15 | "A4 每页 4 卡" | pdf.rs 列数仅 1/2/3（图片附录 2×2/页） | 修正版式描述 |
| docs/验收基线/清单1 | "45 个测试全绿"；H 节 #34–37 `wechat::*` ✅ | 实为 **178 用例**；微信用例随功能消失仍标 ✅；测试名 schema_creates_seven_tables/core_flow 已过时（实为 eight/voucher_flow） | 回写 |
| README:50 | "120+ 用例" | 属实但保守（实际 178） | 顺手更新 |
| docs/开发方案.md | §3/§5 仍把微信监听当落地项 | 已移除未注记 | 补注记 |

**提交卫生建议**：① 体检文档与本次分析报告**纳入版本库**（现 untracked，建议随文档回写一次提交）② README/清单1 失真一次修正 ③ `.archive-*`（本地 6 目录、不在库内）确认无留存价值后删除，g3-step 压测图优先（可省 >10MB）。

## 八、风险登记（Top 6，全部经本轮实证）

| 风险 | 等级 | 位置 |
|---|---|---|
| WebDAV 字符边界 **panic**（to_lowercase 切原串） | 🔥高 | backup/webdav.rs:480 |
| Mutex 中毒 → 全应用 DB 永久不可用、无恢复 | 🔥高 | db/unit_of_work.rs:25/31（7 处同型） |
| 凭据明文外传（WebDAV 9 处 + 云端 OCR 1 处无 https 校验） | 高 | webdav.rs / services/ocr.rs:169 |
| 前后端契约改名漏改（client.ts 改 24 次 0 测试） | 高 | src/api/client.ts ↔ main.rs |
| 配置双真源已实际分歧 2 处（命名模板/打印标题） | 中高 | SettingsPage.vue:37,51 ↔ config.rs:64,184 |
| zip-slip 仅查 `..` 未拒绝对路径 + assetProtocol scope 全开 | 中高 | backup/restore.rs:203 / tauri.conf.json:30 |

## 九、建议路线（对齐体检报告实施顺序 + 版本节奏）

1. **立即（v0.14 首选，≈2.5 人日）**：体检"第一批止血"7 项——修 webdav 字符边界、锁中毒+RAII 护栏、WebDAV/OCR 强制 HTTPS、zip-slip 绝对路径拒绝、收窄 assetProtocol、invoke 一致性快照测试；外加修正已实际分歧的 2 处配置默认值。
2. **本迭代**：clippy 覆盖 src-tauri + 前端 ESLint（0.1+1 人日，成本极低收益高）；文档失真 4 处一次回写 + 体检/分析报告入库。
3. **下迭代**：命令桥/契约测试、WebDAV mock、DB 损坏/并发回归、前端组件测试基建；消重复（CoreError::Cv 变体、query_grouped、useSecretSave）。
4. **按需**：换 OCR 引擎与新增打印版式时，分别落地 `OcrEngine` trait 与 pdf.rs `TableRenderer` 抽表（不建议单独立项）。

---

## 附：方法与局限

- 5 角色**并行独立只读**分析后合并交叉印证；前端 vitest 实测；Rust 侧因 OpenCV/libclang 编译成本**未本地执行**（静态计数 + CI 配置为准，计数与 CI 一致）。
- 口径修正 3 处（生产 unwrap 10、越层 ~20、.archive-* 不在库）建议以本报告为准同步进健康体检文档。
- 未改动任何源码；分析文档未提交（如需入库请指示）。
