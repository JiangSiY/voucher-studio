/**
 * `withLiveAppearance`：设置的两条「整包提交」路径共用的覆盖入口（纯函数）。
 *
 * 它从 config store 的方法搬出来，是因为 dev 下改 store 源码时 pinia 只换定义、不换实例，
 * 页面会拿到没有新方法的旧实例（用户实报 `configStore.withLiveAppearance is not a function`）。
 * 纯函数 import 不受实例缓存影响，这里顺带守住"只覆盖这两项、其余照旧、不改原对象"。
 */
import { describe, expect, it } from "vitest";
import { withLiveAppearance } from "../liveAppearance";
import { SIDEBAR_WIDTH_DEFAULT, defaultConfig } from "../../config-defaults";
import type { AppConfig } from "../../api/client";

describe("withLiveAppearance", () => {
  it("用运行期值覆盖草稿里的侧栏宽度与收起状态", () => {
    const live: AppConfig["appearance"] = {
      theme: "system",
      sidebarWidth: 300,
      uiScale: 1.15,
      sidebarCollapsed: true,
    };
    // 进设置页时的旧快照（此后用户拖过侧栏、收起过侧栏）
    const stale = defaultConfig();
    expect(stale.appearance.sidebarWidth).toBe(SIDEBAR_WIDTH_DEFAULT);

    const merged = withLiveAppearance(stale, live);

    expect(merged.appearance.sidebarWidth).toBe(300);
    expect(merged.appearance.sidebarCollapsed).toBe(true);
    // 其余字段照旧来自草稿（helper 只覆盖这两项）
    expect(merged.appearance.theme).toBe(stale.appearance.theme);
    expect(merged.appearance.uiScale).toBe(stale.appearance.uiScale);
    // 不改原对象（草稿仍可继续编辑）
    expect(stale.appearance.sidebarWidth).toBe(SIDEBAR_WIDTH_DEFAULT);
    expect(stale.appearance.sidebarCollapsed).toBe(false);
  });

  it("运行期外观缺失时原样返回草稿（预览模式不凭空造值）", () => {
    const draft = defaultConfig();
    draft.appearance.sidebarWidth = 240;

    expect(withLiveAppearance(draft, null).appearance.sidebarWidth).toBe(240);
    expect(withLiveAppearance(draft, undefined).appearance.sidebarWidth).toBe(240);
  });
});
