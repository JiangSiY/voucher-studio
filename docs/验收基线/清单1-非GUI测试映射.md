# 验收基线 · 清单一：非 GUI 测试映射（cargo test ↔ v4.3 断言语义）

> 固化日期：2026-08-20（G2 评审门前，对应开发方案决策 4）。
> v4.3 源机不在本机，本清单按 v4.3 功能域（开发方案 §3 映射表）与新库测试全集逐条对齐；
> migration 类测试（test_migration / test_migrate_ledger）随旧数据路径退役，不迁移。
> 基线（**G1 固化时点**，2026-08-20）：**45 个测试全绿**（lib 18 + core_flow 23 + spike_a_print 4）。**截至 v0.14.0（2026-09-17）实为 212 个**（121 单元 + 91 集成）——见下方修订注记。

> ⚠️ **修订注记（2026-09-09 初版；2026-10-16 按当前工作区更新）**：本文档固化于 G1 门，测试全集此后持续增长。
> 截至 **2026-10-16（v0.18 洞察批 + 审计剩余项批次 2–7 + OCR 三批（置信度门控 / 评测 harness + 加固 / PP-OCRv5）+ 侧栏收起批）**：`voucher-core` 测试 **282 个**（src 内联 **152** + tests/ 集成 **130**，含
> backup_flow 11 / **dashboard_flow 20** / db_resilience 5 / **dup_review_flow 7** / **dup_decisions_upgrade 1** / enhance_e2e 24 /
> **keyring_native 1** / ocr_flow 6 / **ocr_eval 1（`VOUCHER_OCR_EVAL` 门控，CI 默认不跑）** / spike_a_print 5 / **voucher_flow 43** / webdav_mock 5）；
> 另 `src-tauri` **3** 个（恢复残留清理 + 2026-09-23 新增：半截待恢复库被拒且活库不丢、
> 合法待恢复库正常落位）；前端 vitest **35 文件 217 用例**（含 `DuplicateReviewPage.test.ts` 8 例交互测试、
> `dateRange.test.ts` 13 例时间窗口径用例，以及 2026-09-27 新增的 `registrationContract.test.ts` 5 例通用登记守卫、
> `timerContract.test.ts` 3 例前端资源清理守卫；2026-09-28 右栏编辑器改造后 `LibraryPage.test.ts` 增至 9 例（7 字段保存 / 双击展宽 / `focus` 深链），另补 `voucherEditorLayout.test.ts` 3 例布局契约——锁住「表单一条标签列」，防「每行各算宽度」的错位复发）；
> Rust 侧另有 `tests/perf_bench.rs`（1 例，`VOUCHER_BENCH=1` 门控，CI 默认不跑性能断言）。
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
> 前端合计数 **35 文件 217 用例**（原 33 文件 208 用例，更早 32 文件 200 用例）。
> 计数口径：`cargo test --workspace --no-fail-fast -- --include-ignored` 实际产出（**280 通过 + 5 `#[ignore]`**（含上述新 2 例）= 285）。
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
