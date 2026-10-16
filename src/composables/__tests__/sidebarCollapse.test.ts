// @vitest-environment jsdom
/**
 * 侧栏收起的 CSS 应用：只写根属性 `data-sidebar-collapsed`。
 *
 * 契约来自两处教训：
 *  1. 必须是根属性而不是元素内联样式——内联特异性高于媒体查询，会顶掉窄窗图标栏
 *     （`useSidebarResize.ts` 头注释的同一理由）；
 *  2. 展开也要显式写 `"false"`，否则「收起过 → 再展开」时属性残留会让人以为开关失灵。
 */
import { beforeEach, describe, expect, it } from "vitest";
import { applySidebarCollapsed } from "../sidebarCollapse";

beforeEach(() => {
  delete document.documentElement.dataset.sidebarCollapsed;
});

describe("applySidebarCollapsed", () => {
  it("收起写 true、展开写 false（属性缺失时 CSS 视作展开）", () => {
    applySidebarCollapsed(true);
    expect(document.documentElement.dataset.sidebarCollapsed).toBe("true");

    applySidebarCollapsed(false);
    expect(document.documentElement.dataset.sidebarCollapsed).toBe("false");
  });

  it("不写任何内联样式（宽度/隐藏全部由 CSS 根属性选择器承担）", () => {
    applySidebarCollapsed(true);
    expect(document.documentElement.getAttribute("style") ?? "").not.toContain("sidebar");
    expect(document.documentElement.getAttribute("style") ?? "").not.toContain("width");
  });
});
