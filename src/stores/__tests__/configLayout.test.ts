// @vitest-environment jsdom
/**
 * config store 的布局应用（`applyLayout` / `saveSidebarWidth` / 侧栏收起）。
 *
 * 浏览器预览模式下 `App.vue` 会跳过 `configStore.init()`（config 为 null），
 * 此时不应写 CSS 变量——保持 `tokens.css` 的默认 260px；
 * 侧栏收起的开关同样要在这种模式下"只改界面不落盘"（用户 2026-10-16 要求）。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { createPinia, setActivePinia } from "pinia";

vi.mock("../../api/client", () => ({
  api: { getConfig: vi.fn(), getAppInfo: vi.fn(), saveConfig: vi.fn() },
}));

import { api } from "../../api/client";
import { useConfigStore } from "../config";
import { SIDEBAR_WIDTH_DEFAULT, SIDEBAR_WIDTH_MAX, defaultConfig } from "../../config-defaults";

function cssVar(): string {
  return document.documentElement.style.getPropertyValue("--sidebar-w");
}

beforeEach(() => {
  setActivePinia(createPinia());
  document.documentElement.style.removeProperty("--sidebar-w");
  delete document.documentElement.dataset.sidebarCollapsed;
  vi.mocked(api.getConfig).mockReset();
  vi.mocked(api.saveConfig).mockReset();
});

describe("config store 布局应用", () => {
  it("init 后把已保存的侧边栏宽度写进 --sidebar-w", async () => {
    const cfg = defaultConfig();
    cfg.appearance.sidebarWidth = 320;
    vi.mocked(api.getConfig).mockResolvedValue(cfg);

    const store = useConfigStore();
    await store.init();

    expect(cssVar()).toBe("320px");
  });

  it("配置未加载时不写变量（浏览器预览模式的现状）", () => {
    const store = useConfigStore();
    expect(store.config).toBeNull();
    expect(cssVar()).toBe("");
  });

  it("saveSidebarWidth 钳制越界输入并落盘", async () => {
    vi.mocked(api.getConfig).mockResolvedValue(defaultConfig());
    vi.mocked(api.saveConfig).mockImplementation(async (c) => c);

    const store = useConfigStore();
    await store.init();
    await store.saveSidebarWidth(9999);

    expect(cssVar()).toBe(`${SIDEBAR_WIDTH_MAX}px`);
    const saved = vi.mocked(api.saveConfig).mock.calls.at(-1)?.[0];
    expect(saved?.appearance.sidebarWidth).toBe(SIDEBAR_WIDTH_MAX);
  });

  it("saveSidebarWidth 在值未变时不重复落盘", async () => {
    vi.mocked(api.getConfig).mockResolvedValue(defaultConfig());
    vi.mocked(api.saveConfig).mockImplementation(async (c) => c);

    const store = useConfigStore();
    await store.init();
    await store.saveSidebarWidth(SIDEBAR_WIDTH_DEFAULT);

    expect(api.saveConfig).not.toHaveBeenCalled();
    expect(cssVar()).toBe(`${SIDEBAR_WIDTH_DEFAULT}px`);
  });

  it("init 后把已保存的收起状态写到根属性", async () => {
    const cfg = defaultConfig();
    cfg.appearance.sidebarCollapsed = true;
    vi.mocked(api.getConfig).mockResolvedValue(cfg);

    const store = useConfigStore();
    await store.init();
    await nextTick();

    expect(store.sidebarCollapsed).toBe(true);
    expect(document.documentElement.dataset.sidebarCollapsed).toBe("true");
  });

  it("toggleSidebarCollapsed 翻转界面并落盘（收起与展开两态都要存）", async () => {
    vi.mocked(api.getConfig).mockResolvedValue(defaultConfig());
    vi.mocked(api.saveConfig).mockImplementation(async (c) => c);

    const store = useConfigStore();
    await store.init();

    await store.toggleSidebarCollapsed();
    await nextTick();
    expect(document.documentElement.dataset.sidebarCollapsed).toBe("true");
    expect(vi.mocked(api.saveConfig).mock.calls.at(-1)?.[0].appearance.sidebarCollapsed).toBe(true);

    await store.toggleSidebarCollapsed();
    await nextTick();
    expect(document.documentElement.dataset.sidebarCollapsed).toBe("false");
    expect(vi.mocked(api.saveConfig).mock.calls.at(-1)?.[0].appearance.sidebarCollapsed).toBe(false);
  });

  it("配置未加载时切换只改界面不落盘（浏览器预览模式）", async () => {
    const store = useConfigStore();
    expect(store.config).toBeNull();

    await store.toggleSidebarCollapsed();
    await nextTick();

    expect(store.sidebarCollapsed).toBe(true);
    expect(document.documentElement.dataset.sidebarCollapsed).toBe("true");
    expect(api.saveConfig).not.toHaveBeenCalled();
  });

  it("withLiveAppearance 用运行期值覆盖草稿里的侧栏宽度与收起状态", async () => {
    const cfg = defaultConfig();
    cfg.appearance.sidebarWidth = 300;
    cfg.appearance.sidebarCollapsed = true;
    vi.mocked(api.getConfig).mockResolvedValue(cfg);

    const store = useConfigStore();
    await store.init();
    // 进设置页时的旧快照（此后用户拖过侧栏、收起过侧栏）
    const stale = defaultConfig();
    expect(stale.appearance.sidebarWidth).toBe(SIDEBAR_WIDTH_DEFAULT);

    const merged = store.withLiveAppearance(stale);

    expect(merged.appearance.sidebarWidth).toBe(300);
    expect(merged.appearance.sidebarCollapsed).toBe(true);
    // 其余字段照旧来自草稿（helper 只覆盖这两项）
    expect(merged.appearance.theme).toBe(stale.appearance.theme);
    expect(merged.appearance.uiScale).toBe(stale.appearance.uiScale);
    // 不改原对象（草稿仍可继续编辑）
    expect(stale.appearance.sidebarWidth).toBe(SIDEBAR_WIDTH_DEFAULT);
  });

  it("配置未加载时 withLiveAppearance 原样返回草稿（预览模式不凭空造值）", () => {
    const store = useConfigStore();
    const draft = defaultConfig();
    draft.appearance.sidebarWidth = 240;

    expect(store.withLiveAppearance(draft).appearance.sidebarWidth).toBe(240);
  });
});
