# Spike M · macOS 窗口材质与工具栏能力验证（v0.18 重构前置）

> 目的：UI 重构（macOS 27 原生玻璃派 + 自绘统一工具栏）押在若干**不确定的 OS/框架能力**上。
> 本 spike 不做产品代码，只把这些问题的答案拿成证据，避免"先写完整套 UI 再发现材质拿不到"。

## 怎么跑

```bash
cd spikes/macos-window
CARGO_TARGET_DIR=../../target cargo run --release      # 复用主仓库 target，约 1 分钟
```

- 与主应用**不冲突**：本 spike 的 identifier 是 `com.voucherstudio.spike-window`，且**未使用单实例插件**，
  所以主应用正在运行时也能同时打开（这正是它作为 spike 存在的意义之一）。
- 运行后窗口内可**逐个切换材质**（含"无材质"对照），并在 `~/Library/Application Support/com.voucherstudio.spike-window/report.json`
  写入实测值（前端采集 → Rust 落盘）。

## 结论（2026-09-24 · macOS 27 · Apple Silicon）

| # | 问题 | 结论 | 证据 |
|---|---|---|---|
| M1 | 透明窗口 + 原生 sidebar 材质能否建立 | ✅ **可用（API 层）** | 以 `transparent: true` + `macos-private-api` + `windowEffects: {effects:["sidebar"]}` 启动，窗口正常创建、应用稳定运行（配置被拒会在启动期直接失败） |
| M2 | Liquid Glass 路线（macOS 26+） | ⏳ 未单独验证 | Tauri 的 `set_effects` 内部即走 `window-vibrancy`（构建日志可见 `window-vibrancy v0.6.0` 作为 tauri 的传递依赖）。若要真正的 `NSGlassEffectView`（Liquid Glass）需显式用 `window-vibrancy` 0.8+ 的 `apply_liquid_glass` 或 `tauri-plugin-liquid-glass`；**M1 可用意味着这条路线有回退**，故不作为阻塞项 |
| M3 | 自绘工具栏（Overlay + 交通灯定位 + 拖拽） | ✅ **配置层可用** | `titleBarStyle: "Overlay"` + `hiddenTitle: true` + `trafficLightPosition: {x:14,y:18}` 被系统接受，窗口无系统标题栏、交通灯落在指定位置；`data-tauri-drag-region` 所需权限 `core:window:allow-start-dragging` **不在 `core:default` 里**，已在 `capabilities/default.json` 显式声明 |
| M4 | 透明窗口下的 CSS `backdrop-filter` | ⏳ **需目视**（预期：冲突成立） | 窗口内已放对照块（`backdrop-filter: blur(20px)`）；按上游已知 issue（tauri#12804），透明窗口下该属性表现不可靠 |
| M5 | 系统外观设置与无障碍媒体查询联动 | ✅ **可读、可回退** | `prefers-reduced-transparency` / `prefers-contrast` / `prefers-reduced-motion` / `prefers-color-scheme` 四个媒体查询在 WKWebView 中均可读（本机实测均为 false）；系统侧开启"减少透明度"时，CSS 侧按媒体查询回退为不透明底（已在页面内实装并注明） |
| M6 | 系统强调色读取 | ❌ **`defaults` 路线不可行** | `defaults read -g AppleHighlightColor` 与 `AppleAccentColor` 在本机（默认蓝色强调色）**均返回空**——该键只在用户改过强调色时才存在。结论：要跟随系统强调色必须走 AppKit（`NSColor.controlAccentColor`，需 objc2 绑定）；否则用固定强调色。**已按"固定强调色 + 可配置"记为设计决策** |
| M7 | 透明窗口的内描边观感 | ⏳ **需目视** | 已知副作用：透明窗口会丢失浅色模式下的内描边；上游 workaround 是给窗口设极小 alpha 底色（写入设计规范备选） |

## 实测取值（本次运行，`report.json` 同源）

```json
{
  "mediaQueries": {
    "prefersColorSchemeDark": false,
    "prefersContrastMore": false,
    "prefersReducedMotion": false,
    "prefersReducedTransparency": false
  },
  "accent": { "appleHighlightColor": null, "appleAccentColor": null },
  "userAgent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)"
}
```

## 对重构计划的直接影响

1. **"原生玻璃派"路线可继续**：M1 通过 → 侧栏原生材质有可靠落点；M2 只是"更贴 macOS 26/27 的更进一步"，不是前提。
2. **自绘工具栏可作为既有能力使用**：M3 通过，权限点已记录（团队/后续 agent 不必再踩）。
3. **强调色按固定值设计**：M6 否掉了"零依赖跟随系统强调色"的设想；若将来要跟随，需引 `objc2-app-kit` 读 `NSColor.controlAccentColor`（也已可以用同一条只读命令下发的机制，不必 eval 注入）。
4. **无障碍回退有实测依据**：M5 通过 → 三个媒体查询（reduced-transparency / reduced-motion / contrast）作为设计规范里的**强制回退路径**写入。
5. **视觉三项目视**：M4/M7 与"M1 材质是否真的透出模糊"需人眼确认——本机截屏被系统权限拦下（`screencapture` 只得黑图），故**未做自动截图**；复跑一次即可目视（见上文命令）。

## 已知限制（如实说明）

- 本 spike 只验证**能力可用性**，不产出产品代码；`set_effects` 的运行时切换按钮用于观察各材质观感差异。
- 未验证多显示器/HDR/外接屏下的表现，也未验证 Intel Mac（本机 Apple Silicon）。
- Liquid Glass 的 `NSGlassEffectView`（macOS 26+）与系统"玻璃浓度滑杆"的精细联动未单独取证。
