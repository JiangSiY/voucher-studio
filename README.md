# Voucher Studio

凭证整理打印工具 —— Rust + Tauri 2 + Vue 3 跨平台重写版（前身为 PySide6 版 voucher-print v4.3，旧数据不保留，全新空库上线）。

## 功能特性

- **凭证管理**：编号自动生成（命名规则可配置）、状态流转（草稿/已复核/已导出/已归档）、快照历史、标签
- **图片导入**：拖放/文件选择导入，MD5 自动去重，支持 BMP/JPEG/PNG/WebP
- **OCR 识别**：本地引擎（PP-OCRv4 det + cls + rec，完全离线，数据不出本机，字符准确率 99.9%）与云端引擎（HTTP API）可切换；识别结果缓存、批量补识别
- **智能记账**：OCR 识别结果结构化抽取（日期/金额/对方账户/商户），凭证表单一键填入
- **批量操作**：批量状态流转、批量编辑（日期/备注/标签）、批量导出 PDF
- **报表看板**：月度金额趋势、标签分类统计、对方账户 TOP 榜；统计卡片联动跳转筛选列表
- **图片增强**：扫描件七步增强管线（倾斜矫正→透视矫正→自动裁边→中值去噪→Unsharp 锐化→灰度→自适应二值化，单步失败自动跳过）；增强版用于 OCR 与 PDF 导出，带命中率统计与智能缓存清理
- **搜索**：按编号/摘要/备注/OCR 全文关键词检索
- **导出与打印**：多列省墨 PDF 排版（A4 每页 1/2/3 列可配）、按天分组「凭证日报」（紧凑省纸 / 档案装订双模板）、图片附录（2×2/页）、导出批次记录与历史
- **对账**：微信/支付宝账单 CSV 与库内凭证只读对账（同日同金额匹配），差异报告 CSV 导出
- **重复凭证核查**：同日期同金额的疑似重复进入「重复核查」队列，可并排比对（图片 + 字段差异高亮 + 置信度证据）后处置——**标记"不是重复"**（不改动凭证）或**确认重复并删除多余的一份**（二次确认 + 留痕，含被删凭证快照）。已处置的对不再重复提示。注：「重复」在本项目分三义——**内容重复**（同一张图，导入期 MD5 阻断）、**疑似重复**（同日期同金额，本入口）、**编号重复**（编号唯一约束）
- **备份与恢复**：本地 zip 备份（带版本清单，快照 + 内容寻址附件包增量）+ WebDAV 云备份（可选：age 加密，ChaCha20-Poly1305 AEAD 整文件密封）+ 定时/退出前自动备份；恢复走「待恢复文件 + 重启生效」流程，避免运行中替换数据库。**增强版图片属"可重建缓存"、不入备份包**——恢复后界面自动回退原图显示（后端不会下发指向不存在文件的增强路径），需要增强版时可在 设置 → 扫描矫正 一键重建
- **安全**：敏感凭据（WebDAV 密码、云端 OCR API Key）存入系统凭据库（macOS 钥匙串 / Keychain），密钥不明文落盘
- **稳定性**：数据目录文件锁 + 单实例保护，同一目录同一时刻只允许一个实例写入；异常退出后启动健康检查并提供恢复选项

## 技术栈

| 层 | 技术 |
|---|---|
| 业务核心 | Rust（`crates/voucher-core`，不依赖 Tauri，可独立测试） |
| 桌面壳 | Tauri 2（单实例、对话框、日志、opener 插件） |
| 本地 OCR | ort 2.0 RC（`=2.0.0-rc.13`；上游暂无 2.x 正式版，故精确 pin，见 `crates/voucher-core/Cargo.toml` 注释）+ RapidOCR PP-OCRv4 模型 |
| 数据库 | SQLite（rusqlite，显式事务 UoW 模式） |
| 前端 | Vue 3 + TypeScript + Vite + Pinia + Vue Router |
| UI | 桌面端 macOS 原生观感（系统字体与语义色、材料分层、深浅双主题）；**自 v0.17.0 起仅支持 macOS**（原为 Fluent Design 双平台，见 `docs/开发日志.md`） |

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
cargo test --workspace     # 后端测试（voucher-core：126 单元 + 107 集成，共 233 用例；加 src-tauri 3 用例共 236；静态计数）
cargo test --workspace --no-fail-fast -- --include-ignored  # 与 CI 同口径（含 #[ignore] 真跑；单目标失败不中止其余）
cargo clippy --workspace --all-targets  # Lint
cargo fmt --all            # 格式化
```

## 构建

```bash
npm run tauri build    # 产出 .app / .dmg（bundle.targets 已钉为 app+dmg）
bash package-macos.sh  # macOS 必需收尾：OpenCV dylib 收进包内 + 重签，产出 dist/macos/*.dmg
```

macOS 必须走 `package-macos.sh`：Rust 侧链接的 OpenCV dylib 是**绝对路径**引用，`tauri build` 直出的 `.app` 只能在装有同一路径 OpenCV 的机器上启动（拷给别人即 dyld `Library not loaded`）。脚本把 6 个 dylib 收进 `Contents/Frameworks`、引用改为 `@rpath`、补 rpath 并 ad-hoc 重签名。无 Developer ID 证书时签名只能是 ad-hoc、**无法公证**，分发对象首次打开需右键→打开（或 `xattr -dr com.apple.quarantine`）。

安装包自动携带本地 OCR 模型（约 16 MB），首次启动复制到用户数据目录。

## 目录结构

```
├── crates/voucher-core/   # 业务核心（models / db / services / utils）
├── src-tauri/             # Tauri 壳（commands / platform）
├── src/                   # Vue 3 前端（pages / components / stores）
├── assets/ocr-models/     # 本地 OCR 模型（PP-OCRv4）
├── spikes/ocr-poc/        # G1 Spike B 验证工程（24 张合成凭证，报告见 report.json）
└── docs/开发方案.md        # 开发方案（阶段计划 / 评审门 / 风险登记）
```

## 数据存储

- macOS：`~/Library/Application Support/VoucherStudio`

包含：`voucher.db`（SQLite）、`attachments/`（附件图片）、`backups/`（备份）、`exports/`（导出 PDF）、`models/`（OCR 模型）、`logs/`。

## 许可证

[MIT](LICENSE)
