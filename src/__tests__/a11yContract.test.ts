// @vitest-environment node
/**
 * 无障碍回退与提示通道守卫（v0.27，网页端为基准）。
 *
 * 保留 2026-10-16 的三条实测教训：
 * ① reduced-motion 除令牌归零外必须有通配兜底；
 * ② prefers-contrast: more 必须存在；
 * ③ 危险 toast 用 assertive，普通提示用 polite。
 * 侧栏材质与 56px 图标栏随工作台重构退役，不再断言。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function src(rel: string): string {
  return readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");
}

const tokens = src("src/styles/tokens.css");
const base = src("src/styles/base.css");
const toastHost = src("src/components/ToastHost.vue");
const dialog = src("src/components/ui/Dialog.vue");
const topBar = src("src/components/shell/TopBar.vue");

describe("无障碍回退", () => {
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
});

describe("弹层与页签的可及性", () => {
  it("弹层有 dialog 语义、Esc 关闭与 Tab 焦点循环", () => {
    expect(dialog).toContain('role="dialog"');
    expect(dialog).toContain('aria-modal="true"');
    expect(dialog).toMatch(/e\.key === "Escape"/);
    expect(dialog).toMatch(/e\.key !== "Tab"/);
    expect(dialog).toContain("lastFocused");
  });

  it("页签有导航语义；窄屏命中区不小于 44px", () => {
    expect(topBar).toContain('aria-label="主导航"');
    expect(base).toMatch(/\.tab \{[\s\S]{0,240}(min-)?height: var\(--tap-min\)/);
  });

  it("输入字号走 16px 令牌（iOS 聚焦不自动放大）", () => {
    expect(tokens).toMatch(/--fs-input:\s*calc\(16px \* var\(--font-scale\)\)/);
    expect(base).toMatch(/\.input[\s\S]{0,400}font-size: var\(--fs-input\)/);
  });
});
