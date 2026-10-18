// @vitest-environment node
/**
 * 无障碍回退与提示通道守卫（`docs/UI-设计规范-macOS27.md` §4 基线）。
 *
 * 背景（2026-10-16 渲染层实测）：规范 §4 写了四条无障碍回退，实现只落了两条——
 * ① `prefers-reduced-motion: reduce` 只把 `--motion-fast/--motion-normal` 置 0，
 *    而样式表里还有硬编码时长（`transition: all .15s`、`clip-path .06s`、`animation: toast-in .2s`），
 *    令牌管不到它们 → 系统开了"减少动效"仍照播；
 * ② `prefers-contrast: more` 全仓零命中；
 * ③ 危险 toast 与普通提示同用 `aria-live="polite"`：读屏要念完当前句子才播报，
 *    "删除失败 / 备份失败"这类信息晚几秒就已误事；
 * ④ 深色侧栏材质只有 0.35 不透明度：浅色底上合成出中灰，白字对比度 1.5:1，
 *    侧栏观感随桌面壁纸忽明忽暗（2026-10-16 用户真机反馈）。
 * 这些都是"靠约定不如靠门禁"的典型，故固化为断言。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function src(rel: string): string {
  return readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");
}

const tokens = src("src/styles/tokens.css");
const base = src("src/styles/base.css");
const toastHost = src("src/components/ToastHost.vue");

describe("无障碍回退（规范 §4）", () => {
  it("减少动效：令牌归零之外还要有通配兜底（硬编码时长不受令牌控制）", () => {
    const idx = tokens.indexOf("@media (prefers-reduced-motion: reduce)");
    expect(idx).toBeGreaterThan(-1);
    const block = tokens.slice(idx);
    expect(block).toContain("--motion-fast: 0ms");
    expect(block).toContain("--motion-normal: 0ms");
    expect(block).toMatch(/transition-duration:\s*0\.01ms\s*!important/);
    expect(block).toMatch(/animation-duration:\s*0\.01ms\s*!important/);
  });

  it("提高对比度：描边升强、次要文字升到主级", () => {
    const idx = tokens.indexOf("@media (prefers-contrast: more)");
    expect(idx).toBeGreaterThan(-1);
    const block = tokens.slice(idx);
    expect(block).toContain("--separator: var(--separator-strong)");
    expect(block).toContain("--text-2: var(--label-primary)");
    expect(block).toContain("--text-3: var(--label-primary)");
    expect(block).toContain("--text-4: var(--label-secondary)");
  });

  it("危险 toast 走断言式通道，普通提示仍为礼貌式", () => {
    expect(toastHost).toMatch(/:role="t\.kind === 'danger' \? 'alert' : 'status'"/);
    expect(toastHost).toMatch(/:aria-live="t\.kind === 'danger' \? 'assertive' : 'polite'"/);
  });

  it("深色侧栏材质不透明度足够（浅色底上不被冲成中灰）", () => {
    const decls = [
      ...tokens.matchAll(/--bg-sidebar:\s*rgba\((\d+),\s*(\d+),\s*(\d+),\s*([\d.]+)\)/g),
    ].map((m) => ({ r: Number(m[1]), alpha: Number(m[4]) }));
    // 深色主题块 + `@media (prefers-color-scheme: dark)` 兜底块，两处数值必须都够
    const dark = decls.filter((d) => d.r < 128);
    const light = decls.filter((d) => d.r >= 128);
    expect(dark.length).toBeGreaterThanOrEqual(2);
    for (const d of dark) expect(d.alpha).toBeGreaterThanOrEqual(0.85);
    // 浅色侧**故意**保持低不透明度（用户要"看得到玻璃"，见 tokens.css 注释）——
    // 若有人顺手把它一起抬高，这条会红，提醒先跟用户确认，别悄悄改掉玻璃观感。
    expect(light.length).toBeGreaterThanOrEqual(1);
    for (const d of light) expect(d.alpha).toBeLessThanOrEqual(0.35);
  });

  it("窄窗图标栏里收起开关的文案必须一起隐藏（56px 里会竖排成 5 行撑高整行）", () => {
    const start = base.indexOf("@media (max-width: 760px)");
    const end = base.indexOf("/* ---- 侧栏收起");
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    expect(base.slice(start, end)).toMatch(/\.sidebar-toggle \.nav-label\s*\{\s*display:\s*none/);
  });
});
