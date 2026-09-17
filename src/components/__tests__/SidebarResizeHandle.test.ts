// @vitest-environment jsdom
/**
 * 侧边栏拖拽手柄：键盘步进 / 双击复位 / ARIA 属性。
 * 拖拽本身（pointermove 换算）在 `composables/__tests__/sidebarResize.test.ts` 覆盖——
 * jsdom 没有 PointerEvent 与 setPointerCapture，指针路径在这里不可靠。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";

vi.mock("../../api/client", () => ({
  api: { getConfig: vi.fn(), getAppInfo: vi.fn(), saveConfig: vi.fn() },
}));

import SidebarResizeHandle from "../layout/SidebarResizeHandle.vue";
import { SIDEBAR_KEYBOARD_STEP } from "../../composables/useSidebarResize";
import { SIDEBAR_WIDTH_DEFAULT, SIDEBAR_WIDTH_MAX, SIDEBAR_WIDTH_MIN } from "../../config-defaults";

function cssVar(): string {
  return document.documentElement.style.getPropertyValue("--sidebar-w");
}

beforeEach(() => {
  setActivePinia(createPinia());
  document.documentElement.style.removeProperty("--sidebar-w");
});

describe("SidebarResizeHandle", () => {
  it("→ 键按 16px 步进并写入 --sidebar-w（无侧栏元素时以默认值 260 起算）", async () => {
    const w = mount(SidebarResizeHandle, { attachTo: document.body });
    await w.find(".sidebar-resize-handle").trigger("keydown", { key: "ArrowRight" });
    expect(cssVar()).toBe(`${SIDEBAR_WIDTH_DEFAULT + SIDEBAR_KEYBOARD_STEP}px`);
    w.unmount();
  });

  it("双击复位到默认宽度", async () => {
    document.documentElement.style.setProperty("--sidebar-w", "360px");
    const w = mount(SidebarResizeHandle, { attachTo: document.body });
    await w.find(".sidebar-resize-handle").trigger("dblclick");
    expect(cssVar()).toBe(`${SIDEBAR_WIDTH_DEFAULT}px`);
    w.unmount();
  });

  it("带 ARIA window splitter 属性与提示", () => {
    const w = mount(SidebarResizeHandle);
    const el = w.find(".sidebar-resize-handle");
    expect(el.attributes("role")).toBe("separator");
    expect(el.attributes("aria-orientation")).toBe("vertical");
    expect(el.attributes("aria-valuemin")).toBe(String(SIDEBAR_WIDTH_MIN));
    expect(el.attributes("aria-valuemax")).toBe(String(SIDEBAR_WIDTH_MAX));
    expect(el.attributes("tabindex")).toBe("0");
    expect(el.attributes("title")).toContain("拖动");
    w.unmount();
  });
});
