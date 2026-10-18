# Voucher Studio

凭证整理打印工具 —— Rust + Tauri 2 + Vue 3 跨平台重写版（前身为 PySide6 版 voucher-print v4.3，旧数据不保留，全新空库上线）。

## 功能特性

- **凭证管理**：编号自动生成（命名规则可配置）、状态流转（草稿/已复核/已导出/已归档）、快照历史、**收支方向**（原「标签」，只用来表达金额正负）
- **图片导入**：拖放/文件选择导入，MD5 自动去重，支持 BMP/JPEG/PNG/WebP
- **OCR 识别**：本地引擎（PP-OCRv5 det + cls + rec，完全离线，数据不出本机；准确度基线见 `crates/voucher-core/tests/ocr_eval.rs` 评测）与云端引擎（HTTP API）可切换；识别结果缓存、批量补识别
- **智能记账**：OCR 识别结果结构化抽取（日期/金额/对方账户/商户），凭证表单一键填入
- **批量操作**：批量状态流转、批量编辑（日期/备注/收支方向）、批量导出 PDF
- **待处理驱动**（v0.18 起的主骨架；原称「收件箱驱动」）：首页是**待办流水线**——待识别 → 待核对 → 待打印 → 疑似重复，每项可就地推进；完成的项自动隐藏（不给已完成的事留版面），全部清零时给明确状态
- **全部凭证**（原称「资料库」）：**双行列表**（缩略图 + 摘要 + 金额 / 编号 · 日期 · 状态）+ **右侧编辑器**（选中即看大图与字段，**可就地改全部字段**：日期/摘要/对方/金额/状态/收支方向/备注，另含换图、重跑识别、裁剪、修改历史；双击/回车展开编辑区，⌘⇧E 切换展宽——原独立编辑页已并入此处）；精确筛选（状态 / **收支方向**（下拉：收入·支出·未标）/ 对方 / 金额区间 / 金额未填 / 对方未填 / OCR 状态 / 导出覆盖 / **未被佐证**）与五种排序；跨页选择集与"按筛选全选"；右栏在编辑器下方另有**「相关凭证」**与**「所属事项组」**两块（v0.22 起）
- **凭证关联 / 佐证**（v0.22 起）：有些凭证要另一张凭证来解释——右栏可建立**附件说明**（一张说明凭证解释另一张）与**明细**（一次付款/一笔事项下的多张凭证，用**事项组**承载）两种关系，双向可见、可写说明文本、可解除；列表能筛「**未被佐证**」（既无关联也不在任何组）；新建页可建**无图凭证**（说明 / 备忘 / 原始凭证分割单）。纪律：**关联不带金额、不改任何合计口径**（关联 ≠ 抵消），组的合计只是派生展示值；每个关系建立/解除都写 append-only 审计，删凭证时关联级联清理但**留痕独立存活**
- **任务中心**（v0.21.0 起）：导入 / 关闭前备份 / 生成导出 / 批量识别 / 重建增强这些长任务统一进**内容区左下的全局浮层**，换页不丢、可最小化；有总量画进度条、后端不报进度走不确定态，完成后可一键「打开文件 / 开始核对」（右下角仍留给 toast 结果提示）
- **洞察**：统一时间窗控制器（本月 / 近 3 月 / 近 6 月 / 今年 / 自定义）驱动**月度趋势 + 状态分布 + 月度透视 + 导出覆盖度**；月度透视可按对方/收支方向切换，**每行精确下钻**到「全部凭证」；另有存储占用台账（原图体积走数据库，增强版体积按需计算）
- **图片增强**：扫描件七步增强管线（倾斜矫正→透视矫正→自动裁边→中值去噪→Unsharp 锐化→灰度→自适应二值化，单步失败自动跳过）；增强版用于 OCR 与 PDF 导出，带命中率统计与智能缓存清理
- **搜索**：按编号/摘要/备注/OCR 全文关键词检索
- **导出与打印**：**日报是主线**——表格正文/表头/金额/合计一律 **12pt（小四）**、列宽按 A1（金额**独立预留 40mm**，不再被「对方账户」压到）、摘要超宽折两行、按行高贪心分页（省纸紧凑 / 账本式双模板保留）、图片附录（竖票 3×3 / 横票 2×2）；日报**默认带「相关凭证清单」页**（按关系类型分类小计，页首印"金额不并入合计"）并另存**关联清单 CSV**，单次可关；表格式导出 Excel / CSV 与导出批次历史保留。**年度汇总报表与「单份 PDF」（含卡片风格 / 每行列数 / 预览）均已下线**——打印中心只剩日报一条主线
- **对账**：微信/支付宝账单 CSV 与库内凭证只读对账（同日同金额匹配），差异报告 CSV 导出
- **重复凭证核查**：同日期同金额的疑似重复进入「重复核查」队列，可并排比对（图片 + 字段差异高亮 + 置信度证据）后处置——**标记"不是重复"**（不改动凭证）或**确认重复并删除多余的一份**（二次确认 + 留痕，含被删凭证快照）。已处置的对不再重复提示。注：「重复」在本项目分三义——**内容重复**（同一张图，导入期 MD5 阻断）、**疑似重复**（同日期同金额，本入口）、**编号重复**（编号唯一约束）
- **备份与恢复**：本地 zip 备份（带版本清单，快照 + 内容寻址附件包增量）+ WebDAV 云备份（可选：age 加密，ChaCha20-Poly1305 AEAD 整文件密封）+ 定时/退出前自动备份；恢复走「待恢复文件 + 重启生效」流程，避免运行中替换数据库。**增强版图片属"可重建缓存"、不入备份包**——恢复后界面自动回退原图显示（后端不会下发指向不存在文件的增强路径），需要增强版时可在 设置 → 扫描矫正 一键重建
- **安全**：敏感凭据（WebDAV 密码、云端 OCR API Key）存入系统凭据库（macOS 钥匙串 / Keychain），密钥不明文落盘
- **稳定性**：数据目录文件锁 + 单实例保护，同一目录同一时刻只允许一个实例写入；异常退出后启动健康检查并提供恢复选项

## 代码审计

- **审计指南**：[`docs/审计指南.md`](docs/审计指南.md)（15 个维度 / 四级严重度 / 闭环三档 L1–L3 / 报告模板）
- **一键自动检查**：`bash scripts/audit.sh`（门禁复用 + 用例数三处口径实测 + 危险模式计数 + 契约守卫存在性 + 文档纪律抽查；`--fast` 跳过门禁，`--strict` 可接 CI）
- **最近一次审计**：[`docs/voucher-studio-audit-2026-09-27.md`](docs/voucher-studio-audit-2026-09-27.md)（P0 0 / P1 6 全部修完 / P2 3 已挂账）

## 文档

- **开发方案**：[`docs/开发方案.md`](docs/开发方案.md)（阶段计划 / 评审门 / 风险登记）
- **UI 设计正本**：[`docs/UI-设计规范-macOS27.md`](docs/UI-设计规范-macOS27.md)（令牌 / 导航 / 键盘总表 / 无障碍回退 / 任务中心）
- **开发日志（唯一过程证据源）**：[`docs/开发日志.md`](docs/开发日志.md)（批次七字段记录 + 文末**挂账表**）
- **术语表（非程序员版）**：[`docs/术语表.md`](docs/术语表.md)（当字典查，每次问到新词就补一条）· 配套 [`docs/学习路线-非程序员版.md`](docs/学习路线-非程序员版.md)
- **方案（v0.22 打印日报与凭证关联）**：[`docs/方案-v2-打印日报与凭证关联（定稿）.md`](docs/方案-v2-打印日报与凭证关联（定稿）.md)（执行契约）· 分析底稿 [`docs/方案-打印日报改进-分析底稿.md`](docs/方案-打印日报改进-分析底稿.md)· 关联详版 [`docs/方案-凭证关联与佐证.md`](docs/方案-凭证关联与佐证.md)· 外部实践 [`docs/调研-凭证关系-外部实践.md`](docs/调研-凭证关系-外部实践.md)
- **真机走查**：[`docs/走查清单.md`](docs/走查清单.md) + [`docs/验收基线/`](docs/验收基线/)（清单1 非 GUI 测试映射 / 清单2 真机 50 项 / 清单3 GUI 场景）

## 技术栈

| 层 | 技术 |
|---|---|
| 业务核心 | Rust（`crates/voucher-core`，不依赖 Tauri，可独立测试） |
| 桌面壳 | Tauri 2（单实例、对话框、日志、opener 插件） |
| 本地 OCR | ort 2.0 RC（`=2.0.0-rc.13`；上游暂无 2.x 正式版，故精确 pin，见 `crates/voucher-core/Cargo.toml` 注释）+ RapidOCR PP-OCRv5 模型 |
| 数据库 | SQLite（rusqlite，显式事务 UoW 模式） |
| 前端 | Vue 3 + TypeScript + Vite + Pinia + Vue Router |
| UI | macOS 原生观感：**Liquid Glass 窗口材质**（macOS 26+ 的 `NSGlassEffectView`，失败回退 VisualEffect）、系统字体与语义色、统一工具栏 + 自绘窗口拖拽、**侧栏一列扁平入口且可整栏收起**（收起后 56px 纯图标栏，用**侧栏底部版本行上方那一行**的开关切换、状态跨启动保持；原「收件箱/资料库/设置」分组标题自 2026-10-16 起取消）、**侧栏无左上角图标（顶部只留可拖留白）、待办/疑似重复数以图标右上角角标显示（>99 显示 99+）**、侧栏宽度 184–400 可拖（默认 220）、**界面字号三档可调（默认偏大）**；**自 v0.17.0 起仅支持 macOS**。设计正本见 [`docs/UI-设计规范-macOS27.md`](docs/UI-设计规范-macOS27.md) |

## 环境要求

仅支持 **macOS**（Apple Silicon / arm64）。开发环境：

- Rust 1.75+（stable）+ Xcode Command Line Tools
- Node.js 20+
- **OpenCV 4.10（必须，编译前置；原文为 `brew install opencv`，自 v0.15.1 起改为源码编译固定 4.10）**：`opencv` crate 按**本机已装头文件**生成绑定，代码依赖 4.10 的 API 面（`imgproc::bounding_rect`/`contour_area`/`approx_poly_dp`、`ImreadModes::IMREAD_COLOR`、`cvt_color`/`HoughLines` 参数个数），而 brew 现供 5.x（`opencv@4` 亦已 4.14）上述位置全部变更，直接编译报错。需源码编译 4.10 并让 `PKG_CONFIG_PATH` 指向其 `lib/pkgconfig`；**机器上不要保留 brew 的 opencv**（crate 的 pkg-config 探测优先 `opencv5.pc`，会静默改用 5.x 头文件）
- `libclang`：`opencv` crate 的 bindgen 需要；若 `DYLD_*` 被系统剥离导致找不到，可在 `$HOME/lib` 放符号链接指向 Command Line Tools 的 `libclang.dylib`
- Linux 侧仅用于 CI 的快速门禁（`libopencv-dev` + `clang`/`libclang-dev`），不产出桌面产物

## 开发

```bash
npm install        # 安装前端依赖
npm run tauri dev  # 启动开发模式（自动编译 Rust + 前端热更新）
```

常用命令：

```bash
npm run typecheck          # 前端类型检查
npm run test               # 前端单元测试（Vitest）
cargo test --workspace     # 后端测试（voucher-core：155 单元 + 135 集成，共 290 用例；加 src-tauri 3 用例共 293；静态计数）
cargo test --workspace --no-fail-fast -- --include-ignored  # 与 CI 同口径（含 #[ignore] 真跑；单目标失败不中止其余）
cargo clippy --workspace --all-targets  # Lint
cargo fmt --all            # 格式化
```

构建缓存长期会长得很大（682 个依赖 × debug/release/test 三套 profile × 15 个测试目标，`target/` 曾涨到 39 GB）。
定期清理用仓库脚本（默认只预览，`--apply` 才删；只动 `target/`，不碰源码与 `.git`）：

```bash
bash scripts/clean-build.sh                  # 预览体积与将清理的内容
bash scripts/clean-build.sh --apply          # 清 dev 档 + 旧版本发布件（保留 release 依赖缓存）
bash scripts/clean-build.sh --apply --all    # 全量 cargo clean（下次构建全量重编）
bash scripts/clean-build.sh --apply --sweep  # 叠跑 cargo sweep（需 cargo install cargo-sweep）
```

`Cargo.toml` 已对 dev 档减重（`debug = "line-tables-only"` + 第三方依赖 `debug = false`），
`target/debug` 通常只有原来的一半以下。

## 构建

```bash
npm run tauri build    # 产出 .app / .dmg（bundle.targets 已钉为 app+dmg）
bash package-macos.sh  # macOS 必需收尾：OpenCV dylib 收进包内 + 重签，产出 target/release-artifacts/*.dmg
```

macOS 必须走 `package-macos.sh`：Rust 侧链接的 OpenCV dylib 是**绝对路径**引用，`tauri build` 直出的 `.app` 只能在装有同一路径 OpenCV 的机器上启动（拷给别人即 dyld `Library not loaded`）。脚本把 6 个 dylib 收进 `Contents/Frameworks`、引用改为 `@rpath`、补 rpath 并 ad-hoc 重签名。无 Developer ID 证书时签名只能是 ad-hoc、**无法公证**，分发对象首次打开需右键→打开（或 `xattr -dr com.apple.quarantine`）。

安装包自动携带本地 OCR 模型（约 16 MB），首次启动复制到用户数据目录。

### 安装（分发给别人时）

没有 Apple Developer ID，安装包只做了 **ad-hoc 签名、无法公证**，所以第一次打开会被 Gatekeeper 拦下。
给同事或另一台机器装，按下面任一种方式打开：

1. **右键 → 打开**：双击 dmg → 把 `Voucher Studio.app` 拖进「应用程序」→ 在「应用程序」里**右键点图标 → 打开** → 弹窗再点「打开」（只需一次）。
2. **命令行去隔离标记**（适合脚本/远程分发）：

   ```bash
   xattr -dr com.apple.quarantine "/Applications/Voucher Studio.app"
   open "/Applications/Voucher Studio.app"
   ```

打不开时的自检：`codesign --verify --strict "/Applications/Voucher Studio.app"`（ad-hoc 也应通过）、
`otool -L "/Applications/Voucher Studio.app/Contents/MacOS/voucher-studio" | grep -v @rpath`（**不应**出现
`/Users/…` 这类本机绝对路径——出现说明拿的是 v0.22.2 及更早的坏包，请换 v0.22.3+）。

### 发版（一条命令）

```bash
# 前置：先在本版开发日志写条目 + 写 docs/release-notes/<版本>.md（脚本会强制检查）
bash scripts/release.sh 0.22.3            # 预览：只打印步骤，不改任何文件
bash scripts/release.sh 0.22.3 --apply    # 真正发版
```

`scripts/release.sh` 把发版全流程固化成 7 步：预检（干净/在 main/已推送/日志与说明就绪）→ 门禁 →
**过期产物清理**（`clean-build.sh --apply`：清 dev 档 + 旧版本 dmg / release-artifacts）→ 版本号 bump 三处 →
`tauri build` + **`package-macos.sh`**（OpenCV dylib 收进包内、改 @rpath、重签、出 dmg）→
**产物自包含硬校验**（主二进制不得引用本机绝对路径 / `Contents/Frameworks` 必须含 6 个 dylib / 引用必须 @rpath /
包内版本号一致 / sha256 校验）→ 提交 + tag + `gh release create` + 下载回来复核（`shasum -c` + `cmp`）+
收尾清理（`--keep-build` 可跳过）。

> 2026-10-18 教训：手工发版连续三次漏了 `package-macos.sh`，发布件在别的机器上会 dyld `Library not loaded`。
> 现在第 6 步会**直接拒绝**这种产物，从流程上堵死。

## 目录结构

```
├── crates/voucher-core/   # 业务核心（models / db / services / utils）
├── src-tauri/             # Tauri 壳（commands / platform）
├── src/                   # Vue 3 前端（pages / components / stores）
├── assets/ocr-models/     # 本地 OCR 模型（PP-OCRv5）
├── spikes/ocr-poc/        # G1 Spike B 验证工程（24 张合成凭证，报告见 report.json）
└── docs/开发方案.md        # 开发方案（阶段计划 / 评审门 / 风险登记）
```

## 数据存储

- macOS：`~/Library/Application Support/VoucherStudio`

包含：`voucher.db`（SQLite）、`attachments/`（附件图片）、`backups/`（备份）、`exports/`（导出 PDF）、`models/`（OCR 模型）、`logs/`。

## 许可证

[MIT](LICENSE)
