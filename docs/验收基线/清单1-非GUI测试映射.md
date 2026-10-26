# 验收基线 · 清单一：非 GUI 测试映射（cargo test ↔ v4.3 断言语义）

> 固化日期：2026-08-20（G2 评审门前，对应开发方案决策 4）。
> v4.3 源机不在本机，本清单按 v4.3 功能域（开发方案 §3 映射表）与新库测试全集逐条对齐；
> migration 类测试（test_migration / test_migrate_ledger）随旧数据路径退役，不迁移。
> 基线（**G1 固化时点**，2026-08-20）：**45 个测试全绿**（lib 18 + core_flow 23 + spike_a_print 4）。**截至 v0.14.0（2026-09-17）实为 212 个**（121 单元 + 91 集成）——见下方修订注记。

> ⚠️ **修订注记（2026-09-09 初版；2026-10-16 按当前工作区更新）**：本文档固化于 G1 门，测试全集此后持续增长。
> 截至 **2026-10-26（v0.23.1 OCR 三处优化：导入并发 / 列表页不误填 / 结果卡显示引擎）**：`voucher-core` 测试 **315 个**（src 内联 **172** + tests/ 集成 **143**，含
> backup_flow 11 / **dashboard_flow 20** / db_resilience 5 / **dup_review_flow 7** / **dup_decisions_upgrade 1** / enhance_e2e 24 /
> **keyring_native 1** / ocr_flow 8 / **ocr_eval 1（`VOUCHER_OCR_EVAL` 门控，CI 默认不跑）** / spike_a_print 5 / **voucher_flow 43** / **voucher_link_flow 11 + print_appendix 1（v0.22 关联批 / 批次③ / 收⑦ 收支方向 / 状态多选）** / **spike_a_print 1（v0.22 批次①b：原 5 例中 4 例测卡片版式，随「单份 PDF」下线删除，保留日报样张探针）** / webdav_mock 5）；
> 另 `src-tauri` **3** 个（恢复残留清理 + 2026-09-23 新增：半截待恢复库被拒且活库不丢、
> 合法待恢复库正常落位）；前端 vitest **56 文件 351 用例**（2026-10-26 `ocrEngineContract.test.ts` 6 例加入后**实测**值；文档一律写实测、不再并列 `it(` 静态计数——有动态生成用例，静态数会长期差 1；含 `DuplicateReviewPage.test.ts` 8 例交互测试、
> `dateRange.test.ts` 13 例时间窗口径用例，以及 2026-09-27 新增的 `registrationContract.test.ts` 5 例通用登记守卫、
> `timerContract.test.ts` 3 例前端资源清理守卫；2026-09-28 右栏编辑器改造后 `LibraryPage.test.ts` 增至 9 例（7 字段保存 / 双击展宽 / `focus` 深链），另补 `voucherEditorLayout.test.ts` 3 例布局契约——锁住「表单一条标签列」，防「每行各算宽度」的错位复发）；
> Rust 侧另有 `tests/perf_bench.rs`（1 例，`VOUCHER_BENCH=1` 门控，CI 默认不跑性能断言）。
> **2026-10-24 v0.22 关联批收口（关联逻辑三修 + 徽标 + 备份断言）**：`voucher_link_flow.rs` 新增 3 例——
> ① `appendix_rows_lists_each_link_once_when_both_ends_share_the_day`（同一天两张都传进附页时，
> 同一对只列一次、小计不重复计；**只传主凭证侧**同样只列一次）；
> ② `group_writes_are_audited`（建组/加成员/移出成员/删组各写 append-only 组审计，
> 幂等重复加入不重复留痕）；③ `link_counts_for_list_badges`（列表徽标的 explains/detail/组数计数，
> 一次查一页、不做 N+1）→ 集成 135 → **138**、`voucher-core` 292 → **295**、总数 **298**。
> `backup_flow.rs` 的 `backup_and_restore_roundtrip` 补 P1 断言：备份是 `VACUUM INTO` 整库快照，
> 恢复后 `voucher_links` / `voucher_group_items` / `voucher_link_events` / `voucher_group_events`
> 四表内容必须随库恢复（用例数不变，仍 11）。
> 前端 `voucherRelationContract.test.ts` 9 → **10 例**（列表行关系徽标 + 「+N」展开、命令表 10 → 12 个）
> → 前端 **55 文件 345 用例**。
> **2026-10-26 v0.23 OCR 换代**：删除 PP-OCRv5/ONNX 体系（`ocr_local.rs` + 21MB 模型资产 + `ort` 依赖），
> 本地引擎改为 Apple Vision（`crates/voucher-core/src/services/ocr_vision.rs`，手写 objc FFI、零新增依赖），
> 云端新增 DeepSeek 视觉协议（`ocr.rs` 的 `DeepseekEngine`）与三档策略 `auto/cloud/local`。
> 测试侧：`ocr_flow.rs` 的 `local_ocr_end_to_end` 改为 `vision_ocr_end_to_end`（`#[ignore]` + macOS，
> CI macos job 覆盖），`g2_closed_loop_import_ocr_pdf` 改为显式本地引擎，新增 3 例——
> ① `cloud_engine_is_preferred_and_cached`（mock 云端：文本来自云端、缓存 `engine=cloud`、健康状态记录）；
> ② `cloud_fields_take_priority_over_local_layout`（同附件同时有本地版面与云端要素时，取云端要素）；
> ③ `auto_mode_falls_back_to_local_when_cloud_fails`（云端连不上 → 回退本地并记 `last_cloud_error`，`#[ignore]`）；
> 内联新增 `ocr_vision` 6 例（同行合并 / 标签值拼接 / 置信度 0.3 过滤 / 阅读顺序 / 坐标翻转）+ `ocr_fields::from_structured` 4 例
> → 单元 157 → **169**、集成 138 → **141**、`voucher-core` 295 → **310**、总数 298 → **313**。
> 前端新增 `ocrEngineContract.test.ts` 5 例（三档引擎 / 两种云端协议 / 旧 PP-OCRv5 文案清零 / `ocr_health` 命令对应 / 默认 auto+deepseek）
> → **56 文件 350 用例**。
> **2026-10-26 v0.23.1（三处优化，第六十八笔）**：
> 内联新增 `ocr_fields` 3 例（日期 token 计数 / 列表页行数证据判据 / 列表页抑制金额与对方）；
> 集成新增 `ocr_flow` 2 例（`statement_list_page_does_not_autofill_amount_or_counterparty`、
> `import_runs_cloud_ocr_in_parallel_chunks`——后者带一个多连接迷你 HTTP mock，验 4 张图分块并发全部拿到云端文本）
> → 单元 169 → **172**、集成 141 → **143**、`voucher-core` 310 → **315**、总数 313 → **318**。
> 前端 `ocrEngineContract.test.ts` +1 例（结果卡显示引擎标签与列表页提示）→ **56 文件 351 用例**。
>
> 口径说明：本机 `keyring_native`（1 例）因沙箱内 Keychain 拒绝返回 `UNIX[Operation not permitted]`
> 而失败，属环境限制，不随本批改动变化（CI 上正常）。
> 2026-10-09 OCR 置信度门控批新增 8 例（纯文本路径置信度标档 / 填单门控矩阵 / 大写金额解析 / 大写互验扩展至纯文本路径），内联 134 → **142**；
> 同日新增评测 harness `tests/ocr_eval.rs`（1 例，`VOUCHER_OCR_EVAL=<基准集目录>` 门控，跑生产同款链路：增强→本地 OCR→抽取→门控填单，
> 与 2026-09 基准 102 张逐字段比对），集成 128 → **129**；
> 同日评测驱动加固新增 5 例（复合标签不截胡 / 名称类子标签剥离 / 界面噪音词拒收 / 微信「转给」模式 / 版面右邻噪音拒收——首轮评测 50 例对方实填错误全为「户名/账号/备注」类界面词），内联 142 → **147**；
> 2026-10-09 换 PP-OCRv5 模型批新增 1 例（`pinned_model_files_intact`：随包三模型逐字节 SHA256 锁定，防静默换模/损坏），内联 147 → **148**；
> 2026-10-09 用户实报修复（两栏支付页金额漏抽）新增 3 例（唯一兜底同值重复 / 两栏支付页金额几何配对 / 两栏日期右邻），内联 148 → **151**；
> **2026-10-16 侧栏收起 + 去掉分组**（用户要求）新增 3 例：内联 1 例（`sidebar_collapsed_roundtrips_as_camel_case`：默认展开 + `sidebarCollapsed` camelCase 落盘读回，
> 内联 151 → **152**）、集成 1 例（`sidebar_collapsed_persists_across_contexts`：写盘后新上下文读回，voucher_flow 42 → 43，集成 129 → **130**）、
> 前端新增 1 个文件 2 例（`composables/__tests__/sidebarCollapse.test.ts`：根属性 `data-sidebar-collapsed` 通道）；
> 同批改写 `layouts/__tests__/WorkbenchLayout.test.ts`（5 例，原分组折叠 4 例）、`__tests__/iaContract.test.ts`（9 例，新增「扁平一列」「整栏收起」「顶部让开交通灯带」三条契约）、
> `stores/__tests__/configLayout.test.ts`（7 例，原 4 例；新增收起状态落盘与预览模式只改界面）；
同日用户真机走查两次改位置并复跑：收起开关自工具栏最左移到**侧栏底部版本行上方那一行**（整行按钮，`iaContract` 增第 9 例锁位置）、
> `.sidebar-header` 上留白改为 `calc(var(--toolbar-h) - var(--s-2))` 让开交通灯带（例数不变，仅断言）。
> **2026-10-16 界面细节打磨批**（渲染层审查后的第一批修复）新增 2 个前端文件 8 例、改写 1 例：
> `__tests__/a11yContract.test.ts`（4 例：减动效通配兜底 / 提高对比度令牌映射 / 危险 toast 断言式通道 / 窄窗收起开关文案隐藏）、
> `pages/__tests__/exportHistoryContract.test.ts`（4 例：格式列与三格式标签 / 打开按钮 title 随格式 / 批次状态「已完成」/ 过时「点击进入编辑页」已清）、
> `pages/__tests__/printCenterLayout.test.ts`（4 → 5 例：「全选筛选结果」未勾选时也不禁用）。Rust 侧本批未动（仍 **282 = 152 + 130**）。
> **2026-10-16 界面细节打磨第二批**（浮层焦点契约 + 命令面板接线 + 金额口径）新增 3 个前端文件 11 例：
> `composables/__tests__/useModal.test.ts`（4 例：首焦点 / Tab 双向回绕 / Esc 与 escape:false / 关闭归还焦点）、
> `__tests__/dialogContract.test.ts`（3 例：三个浮层都接 useModal 且声明 aria-modal / 查看器 escape:false / 命令面板按键只注册一次）、
> `__tests__/moneyDisplayContract.test.ts`（4 例：打印中心、OCR 卡片、凭证卡片、编辑器重复提示的金额都走 `fmtAmount`）。Rust 侧仍未动。
> **2026-10-16 界面细节打磨第三批**（未保存修改的离开保护，P1 中唯一会丢数据的一条）改写 1 个文件 +4 例：
> `pages/__tests__/LibraryPage.test.ts` 9 → **13 例**（改字段后点另一行先确认且「留在此页」保住草稿 / 干净时切行不打扰 /
> 路由守卫脏数据挂起导航、拒绝与放行两条路径 / `beforeunload` 脏数据取消关闭）；同批文件头补记「已实装离开保护」。
> Rust 侧未动。
> **2026-10-16 界面细节打磨第四批**（P1 剩余五条：收件箱待办行次要动作 / 搜索截断明示 / 设置第二条保存路径 / 恢复确认文案 / 帮助页）
> 新增 3 个前端文件 11 例、改写 1 个文件 +2 例：
> `pages/__tests__/inboxSearchPolishContract.test.ts`（5 例：次要动作是独立按钮且不在主按钮内 / 行容器结构 / 搜索多取 1 条只用于判断截断 /
> 截断时标题带 `+` 与下一步提示 / 恢复确认按钮不再承诺「就绪并重启」）、
> `__tests__/settingsSavePathContract.test.ts`（3 例：`withLiveAppearance` 是唯一覆盖入口 / 设置页不再内联拼 payload / 备份 Tab「测试连接」走同一 helper 并讲清会一并保存）、
> `pages/__tests__/helpPageContract.test.ts`（5 例：快捷键只列已实装 / 不列 ⌘⇧Enter 与 ⌥←→ / 入口名与侧栏一致且旧称已清 / 9 个入口都有说明 / macOS 钥匙串与侧栏收起、搜索上限已写明）、
> `stores/__tests__/configLayout.test.ts` 7 → **9 例**（`withLiveAppearance` 用运行期值覆盖草稿且不改原对象 / 预览模式原样返回）。Rust 侧未动（仍 **282 = 152 + 130**）。
> **2026-10-16 界面细节打磨第五批**（P2 前半：资料库筛选去重 / 批量改状态二次确认 / 选图入口键盘可达 / toast 去重与上限）
> 新增 1 个前端文件 6 例、改写 1 个文件 +3 例：
> `pages/__tests__/libraryPolishContract.test.ts`（6 例：资料库不再有第二份筛选行与死开关 / 空态的可点「清空全部筛选」/ 批量改状态走确认框且文案含跨页量 / 该确认框不是危险样式 / 两处拖放区都是 role=button 且 Enter 与 Space 可触发）、
> `stores/__tests__/toast.test.ts` 3 → **6 例**（连续重复提示合并为一条且文案不同不合并 / 合并时新回调覆盖旧动作 / 同屏最多 4 条超出丢最旧）。
> 同时删除已死的 `.filter-bar.advanced` 与 `.scope` 样式。Rust 侧未动（仍 **282 = 152 + 130**）。
> **2026-10-16 界面细节打磨第五批补丁**（用户实报 `configStore.withLiveAppearance is not a function`）：把侧栏外观覆盖逻辑
> 从 config store 的**方法**搬成纯函数 `composables/liveAppearance.ts`——pinia 的 store 实例按 id 缓存，dev 下改 store 源码只换定义、
> 不换实例，页面会拿到没有新方法的旧实例（生产打包与 vitest 都正常，只有运行中的 dev 客户端会炸）；store 同时补 `acceptHMRUpdate` 双保险。
> 新增 `composables/__tests__/liveAppearance.test.ts`（2 例：用运行期值覆盖侧栏宽度与收起态且不改原对象 / 运行期外观缺失时原样返回）、
> `__tests__/settingsSavePathContract.test.ts` 3 → **4 例**（新增：store 不再挂该方法、且已接 HMR）、
> `stores/__tests__/configLayout.test.ts` 9 → **7 例**（两个 `withLiveAppearance` 用例迁至上述新文件）。Rust 侧未动（仍 **282 = 152 + 130**）。
> **2026-10-16 深色侧栏材质对比度批**（用户真机反馈「深色下侧栏半透明层对比度不太行」）：`src/styles/tokens.css`
> 深色 `--bg-sidebar` 不透明度 0.35 → **0.94**（浅色侧保持 0.28 不动——同一轮用户明确要「看得到玻璃」），
> 深色主题块与 `@media (prefers-color-scheme: dark)` 兜底块两处同步；`__tests__/a11yContract.test.ts` 4 → **5 例**
> （新增：深色侧栏不透明度 ≥0.85、浅色侧 ≤0.35，防两侧被顺手改成一样）。Rust 侧未动（仍 **282 = 152 + 130**）。
> **2026-10-16 任务中心批**（用户「再然后做任务中心」；规范 §1.5 明文要求）新增 3 个前端文件 13 例：
> `stores/__tests__/tasks.test.ts`（6 例：开任务自动展开 / 进度只对运行中生效 / 失败留原文且完成后带动作 / 已完成最多留 3 条且运行中不丢 / 清除与逐条关闭 / 空列表胶囊文案）、
> `components/__tests__/TaskCenter.test.ts`（6 例：无任务不渲染 / 确定态进度与 aria-valuenow / 未知总量走不确定态 / 完成后动作按钮执行并移除 / 最小化只收列表 / 失败显示错误原文）、
> `__tests__/taskCenterContract.test.ts`（7 例：浮层挂 App.vue / 可最小化 / 进度条两态 / 不与 toast 抢角且 z-index 低于 toast / 导入与关闭前备份接线 / 导出·OCR·增强三处接线 / store 的观察面齐全）。
> Rust 侧未动（仍 **282 = 152 + 130**）。
> **2026-10-16 侧栏瘦身 + 排行时间窗 + toast 规范对齐批**（用户点名四项）新增 3 个前端文件 11 例、改写 2 个、Rust +1 例：
> `__tests__/toastSpecContract.test.ts`（4 例：右上角 / 宽 320 与圆角 10 走 `--radius-toast` / ≤760 上限同步 / 规范表那一行同值）、
> `__tests__/insightsWindowContract.test.ts`（5 例：页面传区间 / 面板 props+watch+口径标注 / api 透传 / 命令层 / core 的 `RangeFilter` 区间优先）、
> `__tests__/iaContract.test.ts` 10 → **12 例**（新增：徽标是压在图标上的角标且 99+ 截断、左上角图标已去掉只留可拖留白）、
> `composables/__tests__/sidebarResize.test.ts` 与 `__tests__/config-defaults.test.ts` 改数值（184 / 220 / 400）、
> `__tests__/taskCenterContract.test.ts` 的 toast 角落断言改右上角。
> Rust 侧新增 `voucher_flow.rs::rankings_respect_date_range`（排行与标签统计按日期闭区间过滤、单边区间生效、**区间优先于 months**），
> 集成 130 → **131**、voucher-core 282 → **283**、总数 **286**（281 通过 + 5 ignored）。
> **2026-10-16 上下文翻张批**（用户「上一张、下一张前端实装」）新增 2 个前端文件 14 例、改写 1 个文件 +3 例：
> `composables/__tests__/neighborNav.test.ts`（8 例：位置文案三态 / 用当前 id 与列表同口径 query 取相邻张 / 未选中不发请求 /
> **在途响应不覆盖新响应** / 取数失败静默隐藏 / `clear()` 让在途响应失效）、
> `__tests__/voucherNeighborContract.test.ts`（6 例：右栏可见入口与禁用态 / ⌥←/→ 排在输入框早退之后 / 翻张上下文就是列表 `query()` 且筛选变化重拉 /
> 「保存并下一张」优先跨页 / 走 `showFocused` 安全动线 / 帮助页与规范同步为已实装）、
> `pages/__tests__/LibraryPage.test.ts` 13 → **16 例**（箭头位置与禁用态、点下一张跳到相邻张、⌥→ 同动线、改筛选重拉位置）。
> Rust 侧未动（仍 **283 = 152 + 131**）。
> **2026-10-16 真机走查收口 + ⌘⇧Enter 批**（用户「真机走查通过…去掉挂账，复核通过并下一张实装」）改写 1 个前端文件 +3 例、反转 2 处断言：
> `pages/__tests__/LibraryPage.test.ts` 16 → **19 例**（⌘⇧Enter 置「已复核」+ 保存 + 翻相邻张 / 已是已复核且无改动只翻张不写库 / ⌘Enter 不被抢且状态保持原值）、
> `pages/__tests__/helpPageContract.test.ts` 与 `__tests__/voucherNeighborContract.test.ts` 的 ⌘⇧Enter 断言由「不得出现 / 仍未实装」反转为「必须出现 / 已实装」。
> **2026-10-16 打印日报改版 + 年度报表下线批（方案 v2 定稿批次①上）**：`services/pdf.rs` 日报表格 8.5/9pt → **12pt（小四）**、
> 列宽改 A1（编号 40 / 摘要 68 / 对方 42 / **金额独立预留 40**）、行高 8/12mm、摘要折两行、两套模板共用**贪心分页**，
> 新增 3 例（`daily_layout_invariants` / `daily_pagination_never_overflows` / `summary_wraps_to_two_lines`）、
> 删除 2 例（`yearly_columns_layout_golden` 与 `voucher_flow::export_yearly_report_pdf_and_batch`，**年度汇总报表按用户决定下线**）；
> 内联 152 → **154**、集成 131 → **130**、`voucher-core` 283 → **284**、总数 **287**（282 通过 + 5 ignored）；前端未动。
> **2026-10-16 凭证关联 / 事项组数据层（方案 v2 批次②上）**新增 `tests/voucher_link_flow.rs` **4 例**：
> 关联归一化+幂等+双向可见+审计留痕（建/解都追加、解除幂等）、删凭证级联清关联但**审计留痕独立存活**、
> 事项组建/加成员幂等/顺序/移出/按凭证查组/删组级联、**老库升级安全**（四张表按需补齐且凭证一张不少）；
> 另把此前**漏登记**的 `dup_decisions` / `ocr_layouts` 补进 `voucher_flow.rs` 的表清单断言。
> 集成 130 → **134**、`voucher-core` 284 → **288**、总数 **287 → 291**；前端当时未动（`client.ts` 仅加类型与 10 个 invoke 封装）。
> **2026-10-16 批次②下（右栏 UI + 「未被佐证」筛选）**：新增 `tests/voucher_link_flow.rs::unlinked_filter_matches_only_vouchers_without_links_or_groups`
> （未被佐证＝无任何关联且不在任何组；`unlinked: false` 只筛已有佐证；解除关联后回到未被佐证；`None` 不筛）
> → 集成 134 → **135**、`voucher-core` 288 → **289**、总数 **291 → 292**；
> 前端新增 `__tests__/voucherRelationContract.test.ts` **5 例**（右栏两区块挂 `shown.id` 且走 `focusFromLink` /
> 关联区块模板不得出现「合计」（关联 ≠ 抵消）/ 组内合计必须标注「派生值」/ 10 个命令名一一对应 /
> 「未被佐证」控件+URL 回显+清空三处都在）→ 前端 **50 文件 296 用例**。
> **2026-10-16 批次②收尾（P1′ 无图凭证）**：`voucher_link_flow.rs` 新增
> `memo_voucher_without_image_is_allowed_and_does_not_break_flows`（无图凭证＝**没有附件**的说明条目：
> 编号自动生成、列表可见、缩略图缺席、金额留空不参与合计、日报照样出且批次张数不受影响、
> 能被「未被佐证」筛到）→ 集成 135 → **136**、`voucher-core` 289 → **290**、总数 **292 → 293**；
> 前端 `voucherRelationContract.test.ts` 5 → **7 例**（新增：说明条目入口存在且不走导入路径 / 列表对无图凭证有占位图标）
> → 前端 **50 文件 298 用例**。
> **2026-10-16 批次③（打印默认带佐证）**：新增 `tests/print_appendix.rs` **1 例**
> （日报附带「相关凭证清单」页：关掉＝1 页、默认开＝+1 页、**批次张数只数主凭证且相关凭证不进 `export_items`**、
> 当天无关联则不加页、清单超 24 行继续分页、另存 CSV 且最后一列显式写「否（仅参考）」；
> 无开关时不写 CSV）→ 集成 136 → **137**、`voucher-core` 290 → **291**、总数 **293 → 294**；
> 前端 `voucherRelationContract.test.ts` 7 → **8 例**（打印中心「附带相关凭证清单」开关**默认开**）→ **50 文件 299 用例**。
> **2026-10-16 批次①b（删「单份 PDF」）**：Rust 删卡片版式（`pdf::generate` / `draw_card` / `PdfInput` /
> `PdfOptions.card_style|columns` / `export_vouchers_pdf` / `export_with_defaults` / `images_of` / `print.card_style|columns|group_by_day`）
> 与 `export_pdf` 命令；`spike_a_print` 5 → **1 例**（保留日报样张 + `report.json`，卡片断言删除）、
> 4 处把"单份导出"当载体的 E2E 改走**日报导出**（`enhance_e2e` 3 + `voucher_flow`/`ocr_flow` 各 1）→
> 集成 137 → **133**、`voucher-core` 291 → **287**、总数 **294 → 290**；前端删 `VoucherCard.vue` 及其测试、
> `moneyDisplayContract` 减 1 例 → **49 文件 292 用例**。
> **2026-10-16 右栏编辑器重设计批（B 批，用户点名的四处 + 分区重构）**新增
> `__tests__/editorRedesignContract.test.ts` **6 例**：关系与组插在核心字段之后、识别结果之前（含 `#relations` 插槽）；
> 识别结果/修改历史经 `CollapsiblePanel` 且**默认折叠**、折叠态带徽标；两个区块控件同为一行（flex nowrap + 窄栏媒体查询折行）；
> 附件操作行 `flex-wrap: nowrap` 且未展宽收成纯图标；未保存状态提到右栏头部（`dirty` 事件驱动）；
> 表单高频前置（摘要→日期→金额→状态）且只读编号行移出、备注自适应、操作条吸底 → 前端 **50 文件 298 用例**。
> **同日补记（按钮与输入框同高）**：用户截图反馈「按钮和输入框大小不一致」——根因是行内按钮用了 `.btn-sm`
> （高 `--control-h-sm` = 26px）而 `.input` 是 `--control-h-md`（32px）；两个关联区块的建立行改为显式 grid
> （类型/搜索/搜索按钮同排 + 说明独占下一行铺满，不再截断），行内按钮一律 `.btn`；顺带排查同类问题并修
> 搜索页「清除筛选」（同排整排 `.input`）。契约测试 6 → **7 例**（新增「与输入框同排的按钮一律 md」）→ **50 文件 299 用例**。
> **同日再补（状态分段按钮 + 日期/金额条件并排）**：用户定「状态改为分段按钮，便于操作」；
> 「日期和金额要不要并排，根据页面的控件利用率决定」。实装：状态行由 `<select>` 改为 `role="radiogroup"`
> 的四个按钮（草稿/已复核/已导出/已归档，`aria-checked` + Roving tabindex + ←/→ 切换）；
> 日期+金额合并为一行 `.ed-pair`，**展宽档两列、常规档堆叠**（测算：常规档值格 ≈212px，并排后每格 ≈100px
> 低于原生日期框 ~130px 的可用宽度；展宽档值格 ≈452px，并排每格 ≈222px 富余）。
> 契约测试 7 → **9 例**；`LibraryPage.test.ts` 的状态交互由 `setValue` 改「点分段按钮」→ 前端 **50 文件 301 用例**。
> **2026-10-17 补（对方账户+收支方向同排 / 标签改方向 / 对齐修正）**：用户「日期/金额标签与输入框未对齐；
> 标签和对方账户也做并排；标签改为收支方向（只用标签表示金额正负），可用分段按钮代替输入框」——
> 实装：① 成对行外标签改为与子标签**同一行**（去掉 `margin-top`）；② 对方账户与**收支方向**按同一套
> 成对规则并排；③ 原自由文本「标签」输入框改为 `role="radiogroup"` 的 **未标 / 收入 / 支出** 分段按钮，
> 值写进 `tags`（**非方向旧标签原样保留**，并在下方小字只读展示 `其他标签：…`，不静默丢数据）。
> 契约测试 9 → **11 例**；`LibraryPage.test.ts` 的标签交互由 `setValue` 改「点 支出」→ 前端 **50 文件 303 用例**。
> **2026-10-17 收⑦（「标签」措辞全应用统一为「收支方向」）**：Rust 新增
> `voucher_link_flow.rs::direction_none_filter_matches_vouchers_without_income_or_expense_tag`
> （「未标」＝标签里既无收入也无支出；`tags` 精确匹配收入/支出；**非法 JSON 兜底视为未标且查询不报错**）→
> 集成 133 → **134**、`voucher-core` 287 → **288**、总数 **290 → 291**；前端：列表与打印中心的标签筛选改
> **下拉**（全部/收入/支出/未标）、洞察按钮改「按收支方向」、仪表盘卡片与导出表头改「收支方向」、
> 契约测试 11 → **13 例** → 前端 **50 文件 305 用例**。
> **2026-10-17 方案 A（图片与字段并排 + 标签移到输入框上方）**：用户确认 A 及三项前提后实施——
> `VoucherEditor.vue` 改两列外壳（`.editor-main`，展宽档 图片|字段）+ 字段改**标签在上**（`.ed-field`）、
> **取消两组并排**（`.ed-pair*` 全部退休）、`LibraryPage.vue` 的 `.split--wide` 右栏下限 560 → **720px**；
> **契约改写（不是放宽）**：`components/voucher/__tests__/voucherEditorLayout.test.ts` 由「一条标签列」
> 改为「单列堆叠 / 标签在上 / **不得出现 `max-content` 标签列、`ed-row`、`ed-pair`**」三条，
> `__tests__/editorRedesignContract.test.ts` 的 4 条并排断言改写为方案 A 断言。
> **用例数不变**（前端仍 **50 文件 305 用例**；Rust 未动 **291**）。
> **2026-10-17 竖图预览 + 常规档容器查询（同一批追加）**：`base.css` 凭证预览框改竖向
> `aspect-ratio: 10/22`、`max-height: 420px`，并把 `.att-card img` 的 `object-fit` 由 **cover 改 contain**
> （原来会把长截图上下裁掉）；`.editor` 加 `container-type: inline-size` +
> `@container (min-width: 380px)` → **常规档右栏够宽时也能"图片|字段"并排**（老 WebKit 忽略 → 退回堆叠）。
> 契约测试 13 → **15 例**（竖向预览不裁图 / 容器查询两列）→ 前端 **50 文件 307 用例**；Rust 未动 **291**。
> **2026-10-17 收⑧（筛选多选 + 面板并排 + 字段面板标题）**：四项筛选（状态/OCR/缺项/收支方向）改成
> `MultiSelect` 多选下拉；去掉「导出覆盖」与「对方账户（精确）」控件（URL 参数仍读，下钻不回退）；
> 后端新增 `VoucherQuery.statuses: Vec<String>`（SQL `IN (...)`）+
> `voucher_link_flow.rs::multi_status_filter_matches_any_of_the_statuses`（多选命中 / 单值兼容 / 空不筛）
> → 集成 134 → **135**、`voucher-core` 288 → **289**、总数 **291 → 292**；
> 右栏四个面板（相关凭证/事项组/识别结果/修改历史）2×2 并排（容器 ≥560px）、字段面板加标题「凭证字段」、
> 无翻张信息时不渲染翻张条；契约测试 15 → **16 例** → 前端 **50 文件 309 用例**。
> **2026-10-17 线上缺陷修复（关联凭证无法完成）**：`RelatedVouchers.vue` 的「说明」输入框漏了 `rel-note` 类
> → 它落在网格第 2 行第 1 列，把 `auto` 列撑到约 400px，**中间的关键词输入框被挤成 0 宽**（看不见/点不到），
> 用户反馈"没有位置可以关联其他凭证"。修复：补类 + 搜索空结果给明确提示；
> 新增**组件动线测试** `components/voucher/__tests__/relatedVouchers.test.ts`（3 例：控件齐全且说明独占整行 /
> 输入→搜索→点候选→调 linkVouchers / 搜不到给提示）+ 契约 1 例（说明必须带 rel-note）
> → 前端 **51 文件 313 用例**；Rust 未动 **292**。
> **2026-10-17 第二次塌陷修复 + 合计行重叠修复**：①「建立关联」的说明行**又**挤掉关键词框（这次表现为
> "两个搜索"按钮）→ `.rel-pick` 从 grid 改**两行 flex**（`.rel-pick-row` 放三件、说明在行外）；
> ② 日报合计行「张数」原先右对齐到"右边界 − 2mm"，12pt 下与金额叠在右下角 → 改为**金额左侧 6mm**，
> 并抽出纯函数 `total_line_x()` + 单测 `total_line_labels_do_not_overlap`（三组大额样例断言不重叠、不越界）；
> 另用 PyMuPDF 对真实 PDF 抽样核验：`44 张` 结束于 485.2pt、金额起于 502.1pt（间隙 5.96mm，无重叠）
> → 单测 **+1**、Rust 总数 **292 → 293**（voucher-core 290 = 155 单元 + 135 集成）；前端未变 **51 文件 313 用例**。
> **2026-10-17 列表筛选栏整理**：三个复选框（金额未填 / 对方未填 / 未被佐证）合并为一个「**缺项**」下拉
> （三者互斥；URL 回显仍接受 `amountEmpty` / `counterpartyEmpty` / `unlinked` 三种旧参数）、
> 筛选栏 **吸顶**（滚动列表与编辑器时不动）、去掉右侧「N 张」、列表 **每页 10 条**（`pageSize = 10`，
> 分页控件自动跟随）。契约：`voucherRelationContract` 的「未被佐证」一条改写为下拉口径、
> `libraryPolishContract` **+1 例**（吸顶/缺项下拉/无张数/每页 10 条）→ **50 文件 308 用例**；Rust 未动 **291**。
> Rust 侧本批未动（仍 **290**）。
> Rust 侧未动（仍 **283 = 152 + 131**）。
> 前端合计数 **49 文件 291 用例**（2026-10-16 当日轨迹：43 文件 257 → 清除死代码批净减 17（删 `dirtyGuard` 9 / `neighborNav` 12 / `VoucherNavBar` 3，新增 `terminologyContract` 3 与 `deadCodeContract` 1）→ 42 文件 240 → 任务中心批 +19 → 45 文件 259 → 侧栏瘦身 / 排行时间窗 / toast 规范对齐批 +11 → 47 文件 270 → 上下文翻张批 +18（含 `neighborNav` 回归的 8 例）→ 49 文件 288 → 真机走查收口 + ⌘⇧Enter 批 +3 → **49 文件 291**；更早 41 文件 247 / 38 文件 232 / 35 文件 217）。
> 计数口径：`cargo test --workspace --no-fail-fast -- --include-ignored` 实际产出（**281 通过 + 5 `#[ignore]`**（含上述新 2 例）= 286）。
> 下表逐条映射以 G1 时点为准，未随版本滚动更新；
> **H 节「微信监听」对应功能已于 v0.7.x 移除（提交 50d2adb），该组用例已删除**，不再作为验收项；
> 单测名 `schema_creates_seven_tables` 已演进为 `schema_creates_eight_tables`（voucher_flow.rs），
> `core_flow` 测试文件已改组为 `voucher_flow.rs`；本表 I 节 #38 `keyring_secret_roundtrip` **测试名不变**，
> 但 2026-09-16 起迁至独立文件 `tests/keyring_native.rs`（隔离进程级凭据后端设置，原生用例与内存后端用例不再同进程竞争）。
> 新增域（对账/期间锁/裁剪/WebDAV mock/DB 容错/**重复凭证处置**）见对应验收文档与 README。
> **v0.16 新增的两组用例**（下表未逐条展开）：`dup_review_flow.rs` 7 例覆盖
> 证据分档（同图/同对方/同摘要）、判定幂等（含反向传参归一化）、已判定不再入队与不再提示、
> 留痕删除（被删凭证 JSON + 附件清单 + 物理文件清理）、审计不随凭证删除消失、
> 封账月拒删但允许标注（事务回滚不留假留痕）、越组守卫；`dup_decisions_upgrade.rs` 1 例覆盖
> **老库升级不丢数据**（还原成无处置表的老库后重新初始化 → 凭证齐全 + 表补齐可写）。

## A. 数据库与仓储（v4.3 `db/`）

| # | 测试 | 断言语义（v4.3 对应） | 状态 |
|---|------|----------------------|------|
| 1 | `schema_creates_eight_tables` | 建库产出 8 张表 + user_version（schema.py 建表语义；G1 时 7 表，后演进为 8） | ✅ |
| 2 | `voucher_crud_roundtrip` | 凭证增查改删闭环 + 字段往返无损（voucher CRUD） | ✅ |
| 3 | `duplicate_voucher_no_rejected` | 凭证编号唯一约束：重复编号被拒（UNIQUE 语义） | ✅ |
| 4 | `update_creates_snapshot` | 修改前落快照（快照审计约定：数据是生命线） | ✅ |
| 5 | `set_status_creates_snapshot` | 状态变更前落快照（reason=status） | ✅ |
| 6 | `export_creates_snapshot_and_marks_exported` | 导出改状态前落快照（reason=export）+ 状态流转为已导出 | ✅ |
| 7 | `config_roundtrip_via_context` | 配置读写往返（config.py 语义） | ✅ |

## B. 录入与去重（v4.3 `ingestion` / `dedup.py`）

| # | 测试 | 断言语义 | 状态 |
|---|------|---------|------|
| 8 | `import_images_dedups_and_ocr_text_searchable` | 图片导入建卡 + 附件行落库 + OCR 文本可搜（ingestion 闭环） | ✅ |
| 9 | `import_batch_dedups_same_content` | 同批同内容图片只入库一次（MD5 去重，批内可见性缺陷回归） | ✅ |
| 10 | `delete_vouchers_cleans_attachment_files` | 删除凭证：附件行级联清空 + 物理文件清理 + 幂等 | ✅ |
| 11 | `local_ocr_end_to_end` | 本地 OCR 引擎端到端：加载模型 → 识别图片 → 文本落库 | ✅ |
| 12 | `ocr_cache_hits_without_network` | OCR 缓存按内容哈希命中，不重复识别（ocr_cache 表语义） | ✅ |

## C. 命名规则（v4.3 `naming.py` + naming_rules.json）

| # | 测试 | 断言语义 | 状态 |
|---|------|---------|------|
| 13 | `naming::validate_accepts_known_tokens` | 模板校验：未知令牌报错、无 {seq} 自动补、未闭合 { 报错 | ✅ |
| 14 | `naming::preview_renders` | 预览渲染：{date} 压缩为 YYYYMMDD、{seq} 三位序号 | ✅ |
| 15 | `naming_seq_skips_holes_after_deletion` | 序号跳过删除空洞（UNIQUE 碰撞回归） | ✅ |
| 16 | `update_with_blank_voucher_no_keeps_old` | 更新空白编号视为不修改（NOT NULL 语义防御） | ✅ |

## D. 搜索（v4.3 `services/search.py`）

| # | 测试 | 断言语义 | 状态 |
|---|------|---------|------|
| 17 | `search_escapes_like_wildcards` | LIKE 通配符转义：%/_ 为字面匹配不误命中全部 | ✅ |
| 18 | `ocr::extract_text_supports_common_shapes` | 云端返回多格式解析（text / data.text / result） | ✅ |

## E. 导出与 PDF（v4.3 `services/export.py` + `pdf/`）

| # | 测试 | 断言语义 | 状态 |
|---|------|---------|------|
| 19 | `export_pdf_and_batch_recorded` | 导出 = PDF 落盘 + 批次/明细记录 + 批次号冲突递增 | ✅ |
| 20 | `pdf::truncate_keeps_short_strings` | 长文本截断带省略号、短文本原样（卡片排版语义） | ✅ |
| 21 | `pdf::width_estimation` | CJK/ASCII 宽度估算（省墨排版的字符预算） | ✅ |
| 22 | `imaging::ext_and_mime` | 扩展名 ↔ MIME 映射 + 图片格式识别 | ✅ |

## F. 备份与恢复（v4.3 `webdav_backup.py`，升级为 3-2-1）

| # | 测试 | 断言语义 | 状态 |
|---|------|---------|------|
| 23 | `backup_and_restore_roundtrip` | 备份 zip 可恢复（数据库 + 配置往返无损） | ✅ |
| 24 | `backup_includes_attachments_and_drill_passes` | 附件入包 + sha256 清单 + 恢复演练通过 | ✅ |
| 25 | `tampered_backup_rejected_by_checksum` | 篡改备份被校验和拒绝（完整性防线） | ✅ |
| 26 | `secondary_dir_gets_copy_of_backup` | 异盘双写：备用份目录出现副本（3-2-1 第二介质） | ✅ |
| 27 | `unwritable_secondary_dir_warns_but_backup_succeeds` | 异盘失败不阻断主备份 + 产生告警（非阻断语义） | ✅ |
| 28 | `backup::gfs_keeps_daily_window_weekly_and_monthly` | GFS 分层保留：日/周/月各自留量正确 | ✅ |
| 29 | `backup::gfs_empty` / `gfs_zero_daily_zero_weekly` | GFS 边界：零保留配置不 panic、空列表安全 | ✅ |
| 30 | `backup::date_parsing_from_file_name` | 备份文件名日期解析（保留策略的时间轴依据） | ✅ |

## G. 调度与文件锁（v4.3 `scheduler.py` / `file_lock.py`）

| # | 测试 | 断言语义 | 状态 |
|---|------|---------|------|
| 31 | `scheduler::due_logic` | 到期判定：从未跑/刚跑/超时/坏时间戳四种分支 | ✅ |
| 32 | `file_lock::lock_acquire_and_conflict` | 文件锁获取与冲突检测 | ✅ |
| 33 | `file_lock_prevents_second_instance` | 第二实例启动被拒（并发写保护，数据是生命线） | ✅ |

## H. 微信监听（v4.3 `wechat.py`）——⚠️ 功能已移除（v0.7，提交 50d2adb）

> 本功能（微信收件夹目录监听自动入库）已随 v0.7 移除，对应测试 #34–37 已删除，不再列为验收项。
> 现存「微信」相关能力仅为**账单 CSV 对账**（reconcile，见 `reconcile_matches_and_reports_gaps`）。

| # | 测试 | 断言语义（v4.3 对应） | 状态 |
|---|------|----------------------|------|
| 34 | `wechat::settle_requires_quiet_then_two_equal_samples` | 写入稳定判定：静默期 + 尺寸双采样 | 🗑️ 已移除 |
| 35 | `wechat::new_event_resets_sampling` | 新事件重置计时与采样（持续写入不误判） | 🗑️ 已移除 |
| 36 | `wechat::stuck_or_missing_files_expire` | 消失文件超时放弃（防 pending 泄漏） | 🗑️ 已移除 |
| 37 | `wechat::empty_file_never_settles_but_expires` | 零字节占位不误报稳定 | 🗑️ 已移除 |

## I. 凭据（v4.3 `crypto.py` → keyring）

| # | 测试 | 断言语义 | 状态 |
|---|------|---------|------|
| 38 | `keyring_secret_roundtrip` | 凭据写入系统凭据库 → 新进程可读出（密钥永不落盘明文） | ✅ |

## J. G1 Spike 验证（阶段 1 退出标准）

| # | 测试 | 断言语义 | 状态 |
|---|------|---------|------|
| 39 | `spike_a_print::a4_geometry_and_pagination_exact` | G1-SpikeA 标准②③：A4 几何 + 21张/页 + 附录 4图/页 分页精确 | ✅ |
| 40 | `spike_a_print::cjk_font_embedded_with_cid_encoding` | G1-SpikeA 标准①：CJK 字体嵌入 + Identity-H 编码（无豆腐块前提） | ✅ |
| 41 | `spike_a_print::card_pages_contain_no_raster_content` | G1-SpikeA 标准④：卡片页零位图（控制区不可能混入的架构保证） | ✅ |
| 42 | `spike_a_print::write_poc_sample_and_report` | 样张 + report.json 留档（spikes/print-poc/） | ✅ |

## K. 遗留与说明

| 事项 | 说明 |
|------|------|
| `write_poc_sample_and_report` 样张复制 | PDF 被阅读器锁定时跳过覆盖（判定已由断言完成，样张仅留档） |
| WebDAV 真实服务器验证 | 上传重试/远端保留清理逻辑已实现，待坚果云/Nextcloud 实测（真机清单 §35） |
| macOS 侧 Spike A | 字体候选含 PingFang/STHeiti/Songti；待 Mac 设备按风险表「Spike 结论过期」条目复验 |
| v4.3 原始测试文件 | 源机不在本机；断言语义按开发方案 §3 功能域映射表重建，见上表「断言语义」列 |
