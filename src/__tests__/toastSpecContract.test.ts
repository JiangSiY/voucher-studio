// @vitest-environment node
/**
 * 通知条（toast）与规范的一致性守卫（2026-10-16 用户点名「toast 位置与规范不符的问题」）。
 *
 * 规范 §1.5 写的是：**右上角、圆角 10、`--bg-popover`、宽 320、`--motion-normal` 滑入**；
 * 而实现当时是右下角、圆角 12、宽 380、硬编码 0.2s ease。这类"规范写了、代码没跟"的漂移
 * 每次改动都容易被顺手改回去，故直接把两边的数值都锁进断言：**改哪边都得同时改另一边**。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (rel: string) => readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");

const css = read("src/styles/base.css");
const tokens = read("src/styles/tokens.css");
const spec = read("docs/UI-设计规范-macOS27.md");

describe("通知条与规范 §1.5 一致", () => {
  it("位置：右上角（右下角让给任务中心浮层）", () => {
    const host = css.slice(css.indexOf(".toast-host {"), css.indexOf(".toast-item {"));
    expect(host).toMatch(/position: fixed/);
    expect(host).toMatch(/top: var\(--s-5\)/);
    expect(host).toMatch(/right: var\(--s-5\)/);
    expect(host).not.toMatch(/bottom:/);
  });

  it("尺寸与圆角：宽 320、圆角 10（走 --radius-toast 令牌）", () => {
    const host = css.slice(css.indexOf(".toast-host {"), css.indexOf(".toast-item {"));
    expect(host).toMatch(/max-width: 320px/);
    expect(tokens).toMatch(/--radius-toast: 10px/);
    const item = css.slice(css.indexOf(".toast-item {"), css.indexOf("@keyframes toast-in"));
    expect(item).toMatch(/border-radius: var\(--radius-toast\)/);
    expect(item).toMatch(/animation: toast-in var\(--motion-normal\) var\(--motion-curve\)/);
  });

  it("窄窗不再溢出：≤760 档的宽度上限同步成 320", () => {
    expect(css).toMatch(/\.toast-host \{\s*max-width: min\(320px, calc\(100vw - 32px\)\);\s*\}/);
  });

  it("规范表里的那一行也保持 右上角 / 圆角 10 / 宽 320（改实现就得改规范）", () => {
    expect(spec).toMatch(/\| 通知条（toast） \|[^|\n]*右上角[^|\n]*圆角 10[^|\n]*宽 320/);
  });
});
