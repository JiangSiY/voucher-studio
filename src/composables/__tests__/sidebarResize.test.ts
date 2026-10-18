// @vitest-environment node
/**
 * 侧边栏宽度纯逻辑（钳制 / 拖拽换算 / 键盘步进）。
 *
 * 组件里的指针事件在 jsdom 不可靠（无 PointerEvent / setPointerCapture），
 * 因此拖动换算的正确性在这一层验证；组件层只测键盘与双击（纯 DOM 事件）。
 */
import { describe, expect, it } from "vitest";
import {
  SIDEBAR_KEYBOARD_STEP,
  clampSidebarWidth,
  steppedSidebarWidth,
  widthFromPointer,
} from "../useSidebarResize";
import { SIDEBAR_WIDTH_DEFAULT, SIDEBAR_WIDTH_MAX, SIDEBAR_WIDTH_MIN } from "../../config-defaults";

describe("clampSidebarWidth", () => {
  it("范围内原样返回，小数取整", () => {
    expect(clampSidebarWidth(300)).toBe(300);
    expect(clampSidebarWidth(299.6)).toBe(300);
    expect(clampSidebarWidth(300.4)).toBe(300);
  });

  it("越界钳到上下边界", () => {
    expect(clampSidebarWidth(0)).toBe(SIDEBAR_WIDTH_MIN);
    expect(clampSidebarWidth(-100)).toBe(SIDEBAR_WIDTH_MIN);
    expect(clampSidebarWidth(9999)).toBe(SIDEBAR_WIDTH_MAX);
  });

  it("非有限数回落默认值", () => {
    expect(clampSidebarWidth(Number.NaN)).toBe(SIDEBAR_WIDTH_DEFAULT);
    expect(clampSidebarWidth(Number.POSITIVE_INFINITY)).toBe(SIDEBAR_WIDTH_DEFAULT);
    expect(clampSidebarWidth(Number.NEGATIVE_INFINITY)).toBe(SIDEBAR_WIDTH_DEFAULT);
  });

  it("默认值落在范围内（与 Rust SIDEBAR_WIDTH_* 同值：184 / 220 / 400）", () => {
    // 2026-10-16 收窄：去掉左上角图标 + 徽标改角标后，行内容只需 ~145px
    expect(SIDEBAR_WIDTH_MIN).toBe(184);
    expect(SIDEBAR_WIDTH_DEFAULT).toBe(220);
    expect(SIDEBAR_WIDTH_MAX).toBe(400);
  });
});

describe("widthFromPointer", () => {
  it("按指针位移换算并钳制", () => {
    expect(widthFromPointer(260, 40)).toBe(300);
    expect(widthFromPointer(260, -40)).toBe(220);
    expect(widthFromPointer(260, -500)).toBe(SIDEBAR_WIDTH_MIN);
    expect(widthFromPointer(380, 500)).toBe(SIDEBAR_WIDTH_MAX);
  });
});

describe("steppedSidebarWidth", () => {
  it("方向键 ±16px，边界处停住", () => {
    expect(steppedSidebarWidth(260, "right")).toBe(260 + SIDEBAR_KEYBOARD_STEP);
    expect(steppedSidebarWidth(260, "left")).toBe(260 - SIDEBAR_KEYBOARD_STEP);
    expect(steppedSidebarWidth(SIDEBAR_WIDTH_MAX, "right")).toBe(SIDEBAR_WIDTH_MAX);
    expect(steppedSidebarWidth(SIDEBAR_WIDTH_MIN, "left")).toBe(SIDEBAR_WIDTH_MIN);
  });
});
