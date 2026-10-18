# G3 视觉证据 · v0.22.3（2026-10-18）

本目录是 **G3 真机/界面走查的视觉证据**：由 Playwright（Chromium headless）对前端预览逐页截图，正常尺寸 PNG，入库保存。

- **截图时间**：2026-10-18 09:50（CST，Asia/Shanghai）
- **版本**：验收目录按计划命名 `v0.22.3`；截图时仓库的 `package.json` / `src-tauri/tauri.conf.json` 仍为 **0.22.2**（本批次只做视觉证据与回归基线，不 bump 版本、不发版）。
- **截图方式**：Vite 开发服务器（`http://localhost:5173`）+ Playwright 注入的 Tauri IPC mock。**不是打包后的 dmg**，也不代表 Rust 侧真实数据；仅用于界面视觉验收与回归。
- **未改动 `src/`**：mock 通过 `page.addInitScript` 在页面加载前注入 `window.__TAURI_INTERNALS__`，前端源码零改动。

## 页面清单

| 文件 | 路由 | 界面状态 |
| --- | --- | --- |
| `vouchers-list.png` | `#/vouchers` | 资料库列表态：12 张凭证取前 10 行（含缩略图/金额/状态），右栏编辑器选中 `2026-10-16-01` |
| `vouchers-editor.png` | `#/vouchers` | 资料库编辑器态：选中第 5 张「设计外包尾款」（草稿），右栏显示凭证图片、字段、状态、收支方向、标签与备注 |
| `insights.png` | `#/insights` | 洞察：近 6 月时间窗（2026-05-01 ~ 2026-10-31）下的导出覆盖度、数据健康、存储占用与月度透视 |
| `print-center.png` | `#/print` | 打印与导出：PDF（打印归档）模式，选中 4 张，右侧 2×2 图片附录预览 |
| `settings.png` | `#/settings` | 设置 · 通用（默认 tab）：界面主题/字号、数据目录、打印默认值 |
| `help.png` | `#/help` | 帮助：三步上手、界面在哪、快捷键、常见问题 |

截图统一参数：viewport **1440×900**、`deviceScaleFactor: 2`（输出 2880×1800）、深色主题（系统深色 + 应用默认 `system`）、默认字号档（`uiScale = 1.15`，与 `config-defaults.ts` 一致）。

## 如何重新生成

依赖已装在 `devDependencies`（`playwright@1.64.1`），Chromium 由 `npx playwright install chromium` 安装。

```bash
cd voucher-studio

# 1) 起前端（另开终端）。脚本也会在 5173 未监听时自动起、并在结束后只杀自己起的进程。
npm run dev

# 2) 重新截图：同时写基线目录和本证据目录
node scripts/visual/capture.mjs

# 3) 与基线逐像素对比（默认阈值 0.5%，超阈值非零退出）
node scripts/visual/compare.mjs
```

- 脚本：`scripts/visual/{fixtures,capture,compare,server}.mjs`
- 基线图（入库，回归真源）：`docs/验收基线/视觉基线/`
- 对比产物（gitignored）：`target/visual-diff/current/`、`target/visual-diff/diff/`、`target/visual-diff/report.json`

## 数据说明

截图里的 12 张凭证、洞察汇总、设置项、打印列表均来自 `scripts/visual/fixtures.mjs` 的**虚构**数据；凭证缩略图是确定性生成的 SVG 收据（data URL），不依赖仓库外的图片文件。未知后端命令统一兜底为 `null`，保证页面不崩。

## 本次复核结果

```text
node scripts/visual/compare.mjs
共 6 张 · 最大差异 0.0000% · 阈值 0.5% · 报告 target/visual-diff/report.json
✅ 视觉回归通过（全部 ≤ 0.5%）
```

## 已知边界

- 基线基于 **Vite dev 预览**；`npm run build` 的产物只做了构建通过验证，未对 dist 单独截图。
- 资料库「展开编辑器」（`⌘⇧E` 宽栏）在 1440 宽下会把左列压到约 440px，行内凭证号/状态会换行；本组证据只用常规双栏态，避免把该挤压布局当作正常界面基线。
- 时间由 Playwright `page.clock` 冻结在 `2026-10-18 10:00 +08:00`，保证「近 6 月」区间等文案在后续复跑时稳定可比。
