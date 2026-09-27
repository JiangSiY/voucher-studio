# voucher-studio 代码审计报告（2026-09-27）

> **性质**：**时点快照**（正文永不修改；需更新只在头部加校正注记）。
> **依据**：`docs/审计指南.md`（维度 / 严重度 / 字段 / **闭环三档 L1/L2/L3**）。
> **基线**：`v0.17.0` tag 之后的 v0.18 全部改动（UI 与操作逻辑重构 + 后端只读聚合 + 精确筛选），`HEAD = 80c2b91` 起。
> **方式**：`bash scripts/audit.sh`（自动部分）+ 人工逐面取证（含三个**首次纳入**的维度：无障碍 / 供应链 / 可观测性）。
> **范围声明**：全量（代码 + 文档 + CI/门禁 + 打包发布）；本轮**未**做真机走查（见 §5）。
>
> **校正注记（2026-09-27 同日追加，正文不改）**：§6 记的是**最终跑**（脚本自检缺陷修毕后）；其中「快照头部注记」一项在 `scripts/audit.sh` 第 7.4 节改为 glob 后覆盖 **5 份**（含本报告），原写 4 份。审计结论与 §1–§3 的发现状态不受影响。

---

## 0. TL;DR

| 项 | 结果 |
|---|---|
| 结论 | **可继续推进，但有两类必须收口**：① 用户可见的**口径分叉**（已修，L2）；② **流程缺口**（bump 未记日志、用例数口径漂移，已补） |
| 发现总数 | **11 条**：P0 **0** ｜ P1 **6**（全部修完）｜ P2 **3**（进挂账）｜ P3 **2**（已修 + 部分登记） |
| 自动化 | `scripts/audit.sh` 全跑：FAIL 0 ｜ WARN 1（`unwrap_or_default` 24 处，设计内提示项）；门禁全绿、10 项守卫/守卫存在性齐全、无越层与平台残留（详见 §6） |
| 首次审计的三个维度 | 无障碍：**有规范无审计项**（本次给出实测清单，未做，进挂账）；供应链：**缺自动化**（无 cargo-audit / npm audit / 许可审查）；可观测性：**基本可用**，缺口见 A-10 |
| 未达 L3 的项 | 全部真机目视类（材质/拖动/检视器手感/a11y 三档）与压测类 → 挂账**第 27 项** |

---

## 1. P1（发布门）——**全部已修，闭环到 L2**

### A-1 待识别计数 ≠ 列表（口径分叉）｜维度 3｜**P1**
- **现象**：收件箱"待识别 N 张"的计数是 `ocr_status IN ('pending','failed')`，而下钻只带 `ocrStatus=pending` → 有 `failed` 附件时数字与列表对不上。
- **证据**：`crates/voucher-core/src/services/dashboard.rs:250`（计数）vs `src/pages/InboxPage.vue:61` 原实现（只推 pending）。
- **影响**：用户按提示去处理，看到的清单少了失败件 → 漏办。
- **建议/修法**：`VoucherQuery.ocr_status` 支持**多值**（逗号分隔，任一命中），下钻改带 `pending,failed`。
- **验收标准**：`todos.ocr_pending == list(ocr_status="pending,failed").total`。
- **状态**：✅ **L2** —— 修于本报告同批提交；用例 `dashboard_flow.rs::ocr_pending_count_matches_list_with_multi_status`（同时固定住"只带 pending 会少算"这一差异）。

### A-2 陈旧草稿计数 ≠ 列表｜维度 3｜**P1**
- **现象**：计数是 `draft 且 updated_at < 现在−7天`，下钻只带 `status=draft` → 文案写"超过 7 天没动的草稿 1 张"，点进去是全部草稿。
- **证据**：`dashboard.rs:266-270` vs `InboxPage.vue` 原下钻（`status=draft&sort=updated_desc`）。
- **影响**：同上（提示与清单不一致）；且 `sort=updated_desc` 当时**无任何用例**。
- **修法**：`VoucherQuery` 新增 `stale_days`（`updated_at < 现在−N 天`），下钻带 `staleDays=7`。
- **状态**：✅ **L2** —— 用例 `stale_draft_count_matches_list_with_stale_days`。

### A-3 「待打印/导出」用错了口径｜维度 3｜**P1**
- **现象**：该阶段计数取 `voucher_stats.reviewed`（全库"已复核"张数），与"待导出"不是一回事；下钻到 `/print` 还不带任何筛选。
- **证据**：`InboxPage.vue:78`（原 `count: stats.value?.reviewed`）。
- **影响**：用户被引导去做一件与数字无关的事；"从没导出过的凭证"这一真实缺口被掩盖。
- **修法**：改用 `export_coverage().neverExported`（与洞察"导出覆盖度"同口径），下钻带 `exported=never`。
- **状态**：✅ **L2** —— 前端用例 `InboxPage.test.ts`（待打印计数源自 neverExported）；后端交叉用例见 A-4。

### A-4 `never_exported` 聚合与筛选两条 SQL 无交叉用例｜维度 3/10｜**P1**
- **现象**：聚合（`dashboard.rs:496`）与列表筛选（`voucher_repo.rs` 的 `exported=never` 反连接）是两条独立 SQL，**"已导出必须被 never 排除"这一分支没有任何用例**——历史用例只覆盖"谁都没导出"。
- **影响**：任一侧改动都会静默产生新的口径分叉（正是 A-1/A-2 的同类）。
- **状态**：✅ **L2** —— 用例 `never_exported_matches_list_after_an_export`（造批次后断言 `never_exported == list(never).total` 且 `exported == list(any).total`）。

### A-5 `bump 0.18.0` 未按纪律追加开发日志条目｜维度 12｜**P1**
- **现象**：`fe95f52` 是版本 bump 提交，但当时没有对应开发日志条目——违反 `AGENTS.md`「开发日志纪律（强制）①」。
- **影响**：过程档案断链（本仓以开发日志为唯一过程证据源）。
- **状态**：✅ **L1+L2** —— 已补条目（并注明"本条为审计补记"）；`scripts/audit.sh` 新增"最新 bump 提交必须有日志条目"的自动检查（防止再犯）。

### A-6 前端用例数未纳入口径且已漂移｜维度 10/12｜**P1**
- **现象**：`AGENTS.md` 纪律 3 只管 Rust `#[test]`；前端计数写在 `清单1` 且已与实际差 1（181 vs 实测 **182**）。
- **状态**：✅ **L1+L2** —— 计数同步；纪律 3 扩为"前端同样三处一致"；`scripts/audit.sh` 实测双侧计数并与文档三处比对。

---

## 2. P2（技术债）——进挂账，未修

| 编号 | 维度 | 现象与证据 | 影响 | 建议 |
|---|---|---|---|---|
| A-7 | 9 契约 | "三处登记"守卫**逐页手写**（`iaContract` 写了 /inbox、/insights；`duplicateReviewContract` 写了 /duplicates），而 `/vouchers`(LibraryPage)、`/print`、`/exports`、`/restore`、`/help` **没有登记断言**——新页面恰落在缺口里 | 新页漏登记不会让任何测试变红（该类缺陷本月已犯过一次：`/exports`、`/restore` 曾在命令面板缺登记） | 改成"扫描 router 全部叶子路由 → 必须出现在 sidebar 或 palette（带白名单）"的通用守卫 |
| A-8 | 3 数据一致性 | `services/dup_review.rs:306` 的 `high_pairs.min(pairs)` 是**静默 clamp**（注释自称一定不超）；且用例 `pending_count_matches_queue` 只断言初始态的 `high_pairs`，**判定一对后未断言其同步下降** | 若两处口径漂移，静默给出偏小的"高置信度对数"，无人发现 | 去掉 clamp 改为断言或计数修正；把用例扩成矩阵（部分判定/全部判定/同图与同字段叠加） |
| A-9 | 4 安全/一致性 | `db/repos/voucher_repo.rs:111` 的 `tags` 解析用 `unwrap_or_default()`——JSON 损坏时**静默变成空标签**，进而影响标签精确筛选与重复判定证据（`same_description`/`same_counterparty` 之外的标签维度） | 数据损坏无声无息 | 确认是"有意容错"还是"掩盖损坏"；建议改为告警日志 + 保留原值兜底 |

**另记（不单独立条）**：生产代码 `unwrap/expect` 共 **7** 处（`scripts/audit.sh` 实测），其中 4 处是 `webdav.rs` 里 `Method::from_bytes(b"PROPFIND")` 这类字面量常量（可用 `OnceLock` 消除）；`models/voucher.rs:112` 的 `parts.first().unwrap()` 建议目视确认空串边界（本次未判定为缺陷，故不入 P2）。

---

## 3. P3（卫生）——已修或已登记

| 编号 | 现象 | 处置 |
|---|---|---|
| A-10 | `DescriptionInput.vue:37` 防抖定时器未在组件卸载时清理（回调可能触发一次）；`stores/config.ts:15` 的 `matchMedia` 监听未配对（store 单例全程存活，低危） | 已登记（挂账第 27 项⑤侧） |
| A-11 | 注释时效：`LibraryPage.vue:12` 自称"待补：跨页选择集/就地编辑"（**已实装**）；`InsightsPage.vue:11` 措辞易读成"统一时间窗未上线"（**已实装**） | ✅ 已改写为真实状态 + 待补项 |

---

## 4. 三个首次纳入维度的判定

### 4.1 无障碍（a11y）｜**有规范、无审计项**
- **现状**：设计正本 §4 已写要求（焦点环、焦点陷阱与归还、`reduce-transparency/motion/contrast` 三档、大字号承受力），但**从未实测**；代码侧可见：全局 `:focus-visible` 已按规范加粗（`base.css`）、`prefers-reduced-transparency/motion` 已接入令牌层（Spike M 已验证媒体查询可读）。
- **判定**：**未取证**。按审计指南 §4.3 的 5 条清单执行，属**真机人工**项 → 挂账第 27 项①。
- **风险**：本产品用户视力不佳，a11y 不是加分项而是主需求；建议下一轮优先走查。

### 4.2 供应链｜**缺自动化（首次判定）**
- **现状**：`ort` 精确 pin 在 `=2.0.0-rc.13`（有注释说明上游无 stable，属**有意**，挂账 #5）；`Cargo.lock` 与三处版本号同步（CI 与本地均验证）；**无 `cargo audit` / `npm audit` / 依赖许可审查**。
- **判定**：缺自动化，非缺陷。**建议**（低成本）：把 `cargo audit` 与 `npm audit --omit=dev` 加进 `scripts/audit.sh` 的"可选节"（默认 WARN 不 FAIL），观察一段时间再决定是否进 CI（避免一上来就因上游告警卡门禁）。

### 4.3 可观测性｜**基本可用**
- **现状**：`tauri-plugin-log` 落盘到数据目录 `logs/`（分级 Info）；错误文案多数带路径/标识（如"附件 {id} 不存在"）；崩溃恢复报告在启动时生成缓存（`get_recovery_report`）；长任务有 `import:*` / `ocr:progress` / `enhance:progress` 事件。
- **缺口**：`unwrap_or_default` 类**静默失败无日志**（24 处，见 A-9）；`pending_summary` 这类"重算"路径无耗时观测（性能问题不易定位）。
- **判定**：可用；建议随 A-9 一并加告警日志。

---

## 5. 本轮**未**验证（如实登记 → 挂账第 27 项）

1. **真机目视类**：Liquid Glass 实际观感、统一工具栏拖动、检视器与透视的操作手感（清单2 #48–50）——本机无法截屏（屏幕录制权限被拒），只能人工。
2. **a11y 实测**（§4.1 五条）——同上。
3. **性能压测**：全量扫描（数千张）下的列表/聚合耗时；`storage_usage(withEnhancedBytes=true)` 的逐文件 stat 成本。
4. **供应链扫描**：无工具，未跑。
5. **Windows 行为**：自 v0.17.0 起不支持，不再验证（既定口径）。
6. **真实 WebDAV / 掉电语义**：与前次审计同（挂账 #2、#24）。

---

## 6. 自动检查结果（`bash scripts/audit.sh`，2026-09-27 最终跑）

```
门禁          前端四门 + Rust 三门（含 #[ignore]）全绿
grep 守卫      越层 OK ｜ 平台范围 OK ｜ 无 Windows 脚本 OK
用例数口径     实测 Rust 258（单元 132 + 集成 123 + 壳 3）、前端 29 文件 184 用例
               README（voucher-core 255 / 总数 258）、清单1（255 / 前端 184）、开发日志（255）四处一致
危险模式       生产 unwrap/expect 7 处 ｜ panic! 1（测试）｜ unsafe 5 ｜ unwrap_or_default 24（见 A-9）
               SQL format! 拼接 9 处（逐处确认为结构拼接，无用户输入）
前端健壮性     完全空 catch 0 处；静默（无日志/重抛）14 处——均已带说明注释
               无绕过 api 封装直 invoke ｜ 无自行拼附件路径 ｜ v-html 1（图标）｜ as any 0
契约守卫      8 份齐全（invoke / 配置 / 令牌 / 附件路径 / 下钻 / 控件尺寸 / IA / 重复核查）
文档纪律      最新 bump（0.18.0）有日志条目 ✅（本次补）｜ 无过期措辞 ✅
               挂账表 27 条 ｜ 4 份快照均有头部注记 ✅
平台打包      package-macos.sh 三项自检在 ｜ OpenCV 4.10.0 断言在
汇总          FAIL 0 ｜ WARN 1（unwrap_or_default，设计内提示项，见 A-9）
```

---

## 7. 结论与建议顺序

1. **P1 全部收口**（A-1…A-6，L2 已完成）——本轮不因审计而阻塞发布。
2. **下一步优先**：把 A-7（通用登记守卫）与 A-8（`high_pairs` 断言矩阵）做掉——它们与本月已发生的三类缺陷（漏登记、口径分叉）同源，属"会再犯"的模式。
3. **再下一步**：a11y 真机走查（§4.1）与清单2 #48–50 一并做，一次走查覆盖两处。
4. A-9（tags 静默容错）在下次碰标签相关代码时顺手确认。
