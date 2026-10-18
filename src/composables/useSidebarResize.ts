/**
 * 侧边栏拖拽调宽的纯逻辑与 CSS 变量写入。
 *
 * 数值范围与默认值在 `config-defaults.ts`（与 Rust 同名常量保持一致），
 * 这里只做钳制与换算，便于在 node 环境单测（组件里的指针事件在 jsdom 不可靠）。
 *
 * 宽度经 `document.documentElement.style.setProperty("--sidebar-w", …)` 应用：
 * **不写元素内联 width**——内联样式特异性高于 `@media (max-width: 760px)` 的
 * `.sidebar { width: 56px }`，会破坏极窄窗的图标栏。
 */
import { SIDEBAR_WIDTH_DEFAULT, SIDEBAR_WIDTH_MAX, SIDEBAR_WIDTH_MIN } from "../config-defaults";

/** 键盘步进（px） */
export const SIDEBAR_KEYBOARD_STEP = 16;

/** 把任意输入钳到 [MIN, MAX]；非有限数回落默认值 */
export function clampSidebarWidth(width: number): number {
  if (!Number.isFinite(width)) return SIDEBAR_WIDTH_DEFAULT;
  const rounded = Math.round(width);
  return Math.min(SIDEBAR_WIDTH_MAX, Math.max(SIDEBAR_WIDTH_MIN, rounded));
}

/** 拖拽换算：起始宽度 + 指针位移 → 钳制后的宽度 */
export function widthFromPointer(startWidth: number, deltaX: number): number {
  return clampSidebarWidth(startWidth + deltaX);
}

/** 键盘步进：ArrowRight 变宽、ArrowLeft 变窄 */
export function steppedSidebarWidth(
  width: number,
  direction: "left" | "right",
  step: number = SIDEBAR_KEYBOARD_STEP,
): number {
  return clampSidebarWidth(width + (direction === "right" ? step : -step));
}

/** 写入 CSS 变量（拖拽过程中高频调用，调用方需自行做 rAF 节流） */
export function applySidebarWidth(width: number): void {
  document.documentElement.style.setProperty("--sidebar-w", `${clampSidebarWidth(width)}px`);
}
