# UI 设计规范 · Loon（2026-11-04 起的设计正本）

> **定位**：本文是 voucher-studio 界面与交互的**唯一设计正本**；`src/styles/tokens.css`
> 与 `src/styles/base.css` 是它的代码落地。两者冲突时以本文为准，改完代码必须回写本文。
> **替代关系**：取代 `docs/UI-设计规范-macOS27.md`（v0.18–v0.23 的 macOS 27 平铺风）。
> 旧规范正文保留不删，作为演进记录；现行取值只看本文与令牌文件。
> **触发**：用户 2026-11-04 提供 Loon for Mac「首页」截图，要求「做成 loon mac 版这种风格，
> 统一而美观」。
> **范围**：视觉令牌、材质层级、壳层与页面版式、控件规格、无障碍回退、契约测试映射。
> **不改**：业务功能语义、数据层、PDF 导出链路、键盘与下钻契约。

---

## 0. 设计原则（先定原则，再定数值）

1. **一张柔和的画布**：整个窗口铺鼠尾草绿灰渐变，内容不再是「白纸铺在窗口里」。
   画布用 `--canvas-gradient`（两处径向高光 + 155° 线性渐变），`--canvas-solid` 是其回退色。
2. **内容区可以是玻璃，但只做一层**：主卡用 `--bg-card` + `--blur-card` + `--radius-card`；
   **卡片里不再套玻璃卡**——嵌板降为 `--bg-card-sunken`，且不模糊。层级仍靠间距与分隔线表达。
3. **材质必须有回退**：`prefers-reduced-transparency: reduce` 时卡片/侧栏/浮层全部改实色、
   `--blur-card: none`；不支持 `backdrop-filter` 时卡片改 `--bg-card-strong`。
4. **一份口径只定义一次**：时间窗、范围、计数仍遵守旧规范 §0.3；视觉改造不新增第二套口径。
5. **可点即可达**：统计、图表、指标卡都保留下钻能力；视觉改造不改交互契约。
6. **可读性优先于信息密度（硬约束）**：用户视力不好。正文 15px 起、辅助 13px 起，
   控件高 ≥32px，字号可调（标准 / 大 / 更大，默认「大」）。**禁止为塞信息缩小字号**；
   玻璃卡上的说明文字用 `--label-secondary`，不用 `--label-tertiary`。
7. **系统观感优先**：系统字体、macOS 语义色、交通灯与工具栏排布不变；Loon 是**视觉语言**，
   不是换一套控件体系。

---

## 1. 视觉令牌（`src/styles/tokens.css`）

### 1.1 分层

```
--canvas-*     画布（gradient / solid）
--bg-card*     玻璃卡三层（card / strong / sunken）与实色回退（*-solid）
--card-hairline 卡片 1px 发丝线（浅色为白、深色为白 10%）
--blur-card    卡片模糊（blur(22px) saturate(1.15)；减少透明度时 none）
--shadow-card  卡片软阴影（内容区**只允许**这一档）
--bg-paper*    纸面（打印预览永远白纸黑字，不随主题）
--radius-card  内容卡圆角（20px）
--radius-pill  胶囊（999px）
--toolbar-h    统一工具栏高（56px）
--s-7          28px（设计尺度白名单外的间距值，统一走令牌）
```

其余语义令牌（`--label-*` / `--separator*` / `--accent*` / `--control-h-*` / `--type-*` /
`--motion-*`）沿用旧规范，数值见 §1.2。

### 1.2 数值（浅色 / 深色）

| 令牌 | 浅色 | 深色 | 用途 |
|---|---|---|---|
| `--canvas-solid` | `#d9e4dc` | `#121715` | 画布回退/减少透明度 |
| `--canvas-gradient` | 白 66% 径向高光 + 绿 50% 径向 + `155deg` 绿灰线性 | 绿 28% / 45% 径向 + `155deg` 墨绿线性 | 窗口画布（`.app-shell`） |
| `--bg-window` | `#dfe9e2` | `#121715` | 窗口兜底色（正常不可见） |
| `--bg-sidebar` | `rgba(158,182,167,.30)` | `rgba(42,54,47,.90)` | 侧栏/工具栏着色（叠原生材质）。**深浅不对称**：浅色要「看得到玻璃」，深色要压住壁纸 |
| `--bg-sidebar-solid` | `#dfe8e1` | `#1b211e` | 减少透明度时的侧栏 |
| `--bg-content` | `#ffffff` | `#1c1c1e` | 只给必须不透明的载体：对话框、吸顶表头/操作列、纸面 |
| `--bg-card` | `rgba(255,255,255,.60)` | `rgba(255,255,255,.065)` | 主内容卡 |
| `--bg-card-strong` | `rgba(255,255,255,.74)` | `rgba(255,255,255,.10)` | 吸顶浮条、检视器、选中胶囊 |
| `--bg-card-sunken` | `rgba(31,54,41,.055)` | `rgba(255,255,255,.04)` | 卡内嵌板（不模糊、无阴影；浅色用深绿低 alpha 才是「凹」） |
| `--bg-card-solid` | `#f4f8f5` | `#232925` | 卡片实色回退 |
| `--bg-card-sunken-solid` | `#e9f1eb` | `#1d2320` | 嵌板实色回退 |
| `--bg-field` | `rgba(255,255,255,.55)` | `rgba(255,255,255,.08)` | 输入框/次要按钮底 |
| `--card-hairline` | `rgba(255,255,255,.60)` | `rgba(255,255,255,.10)` | 卡片 1px 边（对比度档改 `--separator-strong`） |
| `--blur-card` | `blur(22px) saturate(1.15)` | 同左 | 仅主卡/浮条/检视器 |
| `--shadow-card` | `0 18px 40px -26px rgba(31,54,41,.55), 0 2px 6px rgba(31,54,41,.06)` | `0 18px 40px -26px rgba(0,0,0,.7), 0 2px 6px rgba(0,0,0,.25)` | 内容卡 |
| `--bg-paper` | `#ffffff` | `#ffffff` | 打印预览纸面（不随主题） |
| `--paper-ink` | `rgba(0,0,0,.86)` | 同左 | 纸面正文 |
| `--paper-ink-2` | `rgba(0,0,0,.60)` | 同左 | 纸面辅助文字 |
| `--paper-line` | `rgba(0,0,0,.12)` | 同左 | 纸面分隔线 |
| `--label-primary` | `rgba(0,0,0,.85)` | `rgba(255,255,255,.92)` | 正文/标题 |
| `--label-secondary` | `rgba(0,0,0,.55)` | `rgba(255,255,255,.62)` | 次要信息；**玻璃卡上的说明最低就用这一档** |
| `--label-tertiary` | `rgba(0,0,0,.35)` | `rgba(255,255,255,.42)` | 仅用于不承载信息的装饰/占位 |
| `--separator` | `rgba(0,0,0,.10)` | `rgba(255,255,255,.12)` | 常规分隔线 |
| `--separator-strong` | `rgba(0,0,0,.18)` | `rgba(255,255,255,.20)` | 表头/强调分隔 |
| `--accent` | `#0a84ff` | `#0a84ff` | 主操作、选中、聚焦 |
| `--radius-card` | `20px` | — | 内容卡（Loon 新增） |
| `--radius-pill` | `999px` | — | 胶囊/分段/角标 |
| `--radius-panel` | `12px` | — | 对话框/浮层（不跟着内容卡放大） |
| `--radius-control` | `10px` | — | 按钮/输入框/行 |
| `--radius-small` | `8px` | — | 缩略图/小标签 |
| `--toolbar-h` | `56px` | — | 统一工具栏 |
| `--s-7` | `28px` | — | 卡内大间距 |

**字号公式不变**（契约测试逐字断言）：

```css
--fs-title: calc(17px * var(--font-scale));
--fs-body: calc(15px * var(--font-scale));
--fs-caption: calc(13px * var(--font-scale));
--fs-display: calc(20px * var(--font-scale));
--type-page-title: 600 calc(22px * var(--font-scale)) / 1.25;
--type-metric: 600 calc(24px * var(--font-scale)) / 1.15;
```

> 兼容别名（`--surface` / `--text` / `--stroke` / `--accent` / `--sidebar-w`）继续保留，
> 但 `--surface` 仍指向**不透明**的 `--bg-content`：对话框、`.dtable` 吸顶列等载体需要它。
> 内容卡一律显式用 `--bg-card*`，不得再把 `--surface` 当卡片底。

---

## 2. 壳层（`src/layouts/WorkbenchLayout.vue` + `src/styles/base.css`）

### 2.1 窗口与侧栏

- 窗口 1280×820、透明 + `titleBarStyle: Overlay`、`trafficLightPosition {x:14,y:18}`、
  `windowEffects: underWindowBackground`；**原生材质不动**。
- 侧栏 220px（可拖 184–400）、一列扁平 9 条入口、可整栏收起为 56px 纯图标栏。
- 侧栏底：`--bg-sidebar`（浅色低 alpha / 深色 0.9）+ 右缘 1px `--card-hairline`。
- 导航行：`min-height: calc(38px * var(--font-scale))`、`padding: 0 var(--s-3)`、
  行距 2px、圆角 `--radius-control`；选中态是**淡色圆角胶囊**（`--bg-card-strong` +
  1px `--card-hairline` + 主文字色），**不再用左侧 3px 竖条**。
- 角标仍压在图标右上角，`--radius-pill`。

### 2.2 统一工具栏

- 高 56px、透明、无下边线、整条 `data-tauri-drag-region="deep"`。
- 左：`.toolbar-heading`（**横向、基线对齐**）内 `.toolbar-title`（22px/600）+
  `.toolbar-subtitle`（13px/`--label-secondary`，单行省略）。副标题由**路由 `meta.subtitle`**
  声明，页面内不再重复大标题。
  - 2026-11-04 真机反馈「标题太靠上」：副标题原为标题下方纵向排列，把标题顶到工具栏上沿；
    改为同行后，标题落在 56px 工具栏的垂直中线上（1.15 倍字号实测 top 12px、中心 28px）。
- 默认 `padding: 0 var(--s-6)`；`data-sidebar-collapsed="true"` 时左内边距回到
  `--toolbar-inset-start`（84px），给交通灯让位。
- 工具栏内不放按钮（2026-10-16 用户要求去掉了命令面板按钮；⌘K 仍在）。

### 2.3 内容区

- `.main-area` 背景 **transparent**，画布从 `.app-shell` 透上来。
- `.main-scroll` 内边距 `var(--s-3) var(--s-6) var(--s-6)`（12 / 24 / 24）。
- 阅读型页面（表单 / 设置 / 帮助）保留 1100px 上限；列表/仪表盘/打印页铺满。

---

## 3. 内容卡与控件

### 3.1 卡片（`.fluent-panel`）

```css
background: var(--bg-card);
border: 1px solid var(--card-hairline);
border-radius: var(--radius-card);
box-shadow: var(--shadow-card);
backdrop-filter: var(--blur-card);
```

- header 内边距 `var(--s-4) var(--s-5)`，body `var(--s-5)`；header 下分隔线用 `--separator`。
- 嵌套卡片：`--bg-card-sunken`、`--radius-panel`、**无阴影、无模糊**。
- `@supports not (backdrop-filter...)` 时卡片改 `--bg-card-strong`。
- **例外（必须保持不透明）**：对话框/命令面板（`--bg-popover`）、`.dtable` 的吸顶复选/操作列
  （`--surface`）、打印纸面（`--bg-paper`）。

### 3.2 控件

- `.btn` / `.input` / `.search` 高度仍走 `--control-h-md`，`.input`/`.search` 用 `--bg-field`。
- `.segmented`：胶囊轨道（`--radius-pill`、`--bg-card-sunken`、1px hairline），
  选中胶囊 `--bg-card-strong` + 1px hairline + 软阴影；不再用实色蓝底。
- `.pager button` 高度仍 `--control-h-sm`，底色 `--bg-field`。
- 缩略图井：`--bg-card-sunken`；打印缩略图/附件卡保持一致。

### 3.3 各处「浮条」

- 资料库筛选栏：`position: sticky; top: var(--s-3)`，玻璃浮条
  （`--bg-card-strong` + `--radius-card` + `--shadow-card` + blur + 1px hairline），
  **不再用 `border-bottom` 硬切**。
- 检视器：玻璃卡（`--bg-card` + `--radius-card` + 软阴影 + blur），仍 sticky 且内部滚动。

---

## 4. 页面版式

### 4.1 待处理（`InboxPage.vue`）

- 页标题只在工具栏；页内第一行只留状态文案（`.scope`，secondary）与右侧动作。
- 待办仍是**一行一件的列表**（v0.18 的刻意取舍：比卡片网格省纵向空间），
  整块列表装进一张玻璃卡；行高 `calc(52px * var(--font-scale))`，悬停用 `--bg-selection`。
- 计数用 `--type-metric`（24px/600），单位用 caption + secondary。
- 「最近凭证 / 最近导出」两块是玻璃卡；空态文案用 secondary。

### 4.2 洞察（`InsightsPage.vue`）

- 页标题只在工具栏；时间窗预设是胶囊分段（`--radius-pill`），当前段为白/深色胶囊。
- 三张台账卡与月度透视都是玻璃卡；说明文字用 secondary。
- `<TopListsPanel>` 的参数行、`>按收支方向</button>` 等逐字契约不变。

### 4.3 全部凭证（`LibraryPage.vue`）

- 顶部筛选玻璃浮条（见 §3.3），列表与右栏各一张玻璃卡；
  列表行圆角仍 `--radius-control`，缩略图井改 `--bg-card-sunken`。
- `.split` 栅格数值不变（含 `minmax(720px, 1.6fr)`，编辑页契约逐字断言）。

### 4.4 设置（`SettingsPage.vue`）

- 分区切换条浮在卡片上方；正文包一张 `.settings-body` 玻璃卡（padding `--s-5`）。
- 卡内嵌板（命中趋势、清理策略、最近增强、快照行）用 `--bg-card-sunken`；
  mini 按钮/数字框用 `--bg-field`。

### 4.5 打印与导出

- 页面卡片走玻璃；预览面板里的 `.print-area` **永远是白纸**：
  `background: var(--bg-paper); color: var(--paper-ink)`，并在局部把
  `--label-* / --text* / --separator* / --surface*` 全部覆盖成 `--paper-*`。
  这是 2026-11-04 审计实测的缺陷修复（深色下曾是白底白字）。
- 最终 PDF 由 Rust 侧生成，纸面观感不依赖主题。

---

## 5. 无障碍与回退（硬约束）

| 触发 | 行为 |
|---|---|
| `prefers-reduced-transparency: reduce` | 侧栏/浮层/卡片/嵌板改实色；`--blur-card: none`；`--canvas-gradient` 退化为 `--canvas-solid` 纯色 |
| 不支持 `backdrop-filter` | 卡片改 `--bg-card-strong` |
| `prefers-reduced-motion: reduce` | 沿用旧规范：动效令牌归零、骨架 `animation: none` |
| `prefers-contrast: more` | `--card-hairline` 改 `--separator-strong`；文字/分隔线提到高对比档 |
| 浅色侧栏 alpha | ≤ 0.35（a11yContract 硬断言） |
| 深色侧栏 | 必须是深色 rgba（r<128）且 alpha ≥ 0.85；不得改成白色低 alpha |

文本对比：玻璃卡上不要用 `--label-tertiary` 承载信息（浅色下约 2.4:1）；
说明/单位/空态一律 `--label-secondary`。

---

## 6. 契约测试映射

| 契约 | 文件 | 守护内容 |
|---|---|---|
| 令牌锚点/别名/禁名 | `src/__tests__/tokenContract.test.ts` | 5 个锚点名、无 fallback 引用必须有定义、禁旧名、禁 px 字号/hex |
| 设计尺度 | `src/__tests__/designScalesContract.test.ts` | 圆角只能 `--radius-*`/100/999/50%；间距白名单（28 见 `--s-7`） |
| 视觉一致性 | `src/__tests__/visualConsistencyContract.test.ts` | 动效走令牌、骨架、行圆角、筛选栏玻璃浮条 |
| 控件尺寸 | `src/__tests__/controlSizeContract.test.ts` | `.btn/.input/.search` 高度与 box-sizing |
| 无障碍 | `src/__tests__/a11yContract.test.ts` | 减少透明度/动效/对比、深色侧栏 alpha |
| Loon 材质 | `src/__tests__/loonVisualContract.test.ts` | 画布/卡片/纸面/工具栏副标题/去页内 h2 |
| 资料库打磨 | `src/pages/__tests__/libraryPolishContract.test.ts` | 筛选栏吸顶（`top: var(--s-3)`）等 |
| 信息架构/编辑/打印 | `iaContract` / `editorRedesignContract` / `insightsWindowContract` | 导航顺序、逐字模板串、栅格数值 |

---

## 7. 演进记录

- v0.18–v0.23：macOS 27 平铺风——内容区不透明、不叠卡片、不用阴影、6px 控件圆角。
  正本 `docs/UI-设计规范-macOS27.md`（正文保留）。
- 2026-11-04（Loon 化批次，v0.23.0 之后）：画布渐变 + 玻璃内容卡 + 20px 卡片圆角 +
  工具栏副标题；页面内重复的 h2 删除；资料库筛选条改吸顶玻璃浮条；打印纸面修复深色可读性。
- 已知限制：原生 vibrancy 与交通灯只能在 `npm run tauri dev` 真机验证；
  纯浏览器预览只能验证 CSS 层（本项目预览仍以 `npm run tauri dev` 为准）。
