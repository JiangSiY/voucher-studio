// @vitest-environment jsdom
/**
 * config store 的布局应用（`applyLayout` / `saveSidebarWidth`）。
 *
 * 浏览器预览模式下 `App.vue` 会跳过 `configStore.init()`（config 为 null），
 * 此时不应写 CSS 变量——保持 `tokens.css` 的默认 260px。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
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
});
