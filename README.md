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
- **备份与恢复**：本地 zip 备份（带版本清单，快照 + 内容寻址附件包增量）+ WebDAV 云备份（可选：age 加密，ChaCha20-Poly1305 AEAD 整文件密封）+ 定时/退出前自动备份；恢复走「待恢复文件 + 重启生效」流程，避免运行中替换数据库
- **安全**：敏感凭据（WebDAV 密码、云端 OCR API Key）存入操作系统凭据库（Windows 凭据管理器 / macOS Keychain），密钥不明文落盘
- **稳定性**：数据目录文件锁 + 单实例保护，同一目录同一时刻只允许一个实例写入；异常退出后启动健康检查并提供恢复选项

## 技术栈

| 层 | 技术 |
|---|---|
| 业务核心 | Rust（`crates/voucher-core`，不依赖 Tauri，可独立测试） |
| 桌面壳 | Tauri 2（单实例、对话框、日志、opener 插件） |
| 本地 OCR | ort 2.0（ONNX Runtime）+ RapidOCR PP-OCRv4 模型 |
| 数据库 | SQLite（rusqlite，显式事务 UoW 模式） |
| 前端 | Vue 3 + TypeScript + Vite + Pinia + Vue Router |
| UI | Fluent Design 规范（亚克力、Reveal 光效、#0078D4 主色、深浅双主题） |

## 环境要求

- Rust 1.75+（stable，MSVC 工具链）
- Node.js 20+
- Windows：VS Build Tools（C++ 工作负载）+ WebView2 运行时
- macOS：Xcode Command Line Tools

## 开发

```bash
npm install        # 安装前端依赖
npm run tauri dev  # 启动开发模式（自动编译 Rust + 前端热更新）
```

常用命令：

```bash
npm run typecheck          # 前端类型检查
npm run test               # 前端单元测试（Vitest）
cargo test --workspace     # 后端测试（voucher-core：121 单元 + 91 集成，共 212 用例；静态计数）
cargo clippy --workspace --all-targets  # Lint
cargo fmt --all            # 格式化
```

## 构建

```bash
npm run tauri build   # 产出安装包（Windows: NSIS/MSI；macOS: .app/.dmg）
```

安装包自动携带本地 OCR 模型（约 15 MB），首次启动复制到用户数据目录。

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

- Windows：`%APPDATA%\VoucherStudio`
- macOS：`~/Library/Application Support/VoucherStudio`

包含：`voucher.db`（SQLite）、`attachments/`（附件图片）、`backups/`（备份）、`exports/`（导出 PDF）、`models/`（OCR 模型）、`logs/`。

## 许可证

[MIT](LICENSE)
