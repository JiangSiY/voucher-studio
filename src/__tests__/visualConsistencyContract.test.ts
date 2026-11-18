// @vitest-environment node
/**
 * 视觉一致性契约（U4+U5）：动效时长只走令牌 / 设置页无静态内联样式 / 加载三态有骨架 /
 * 列表行与缩略图 / 筛选栏分隔线 / A-B 标签材质统一。
 *
 * 依据：`docs/voucher-studio-audit-2026-10-18.md`（2026-10-18 UI/UX 审美审计）
 * 与 `docs/UI-设计规范-macOS27.md` §1.4（动效）/ §1.5（组件）/ §2.2（列表与三态）。
 *
 * 为什么要有这个契约：
 * - 动效时长此前有 0.06/0.12/0.15/0.2/0.25/1.2s 六套字面量，`prefers-reduced-motion`
 *   只能靠通配 `!important` 兜底；收编到令牌后，减少动效是"真的把时长设为 0"。
 * - 设置页六个 Tab 各写一套内联样式（86 处、43 种），同一语义在不同 Tab 数值不一；
 *   收编到 `.settings-*` 后，只允许动态绑定（`:style`）存在。
 */
import { describe, expect, it } from "vitest";
import {
  repoFile,
  srcFilesRelative as srcSourceFiles,
  srcStyleText as styleText,
} from "./helpers/designGuard";

/** 取出 .vue 的 `<template>` 块 */
function templateText(rel: string): string {
  const text = repoFile(`src/${rel}`);
  const m = text.match(/<template>([\s\S]*)<\/template>/);
  return m ? m[1] : "";
}

const SETTINGS_FILES = [
  "pages/SettingsPage.vue",
  ...srcSourceFiles([".vue"]).filter((f) => f.startsWith("components/settings/")),
];

describe("动效时长只走令牌", () => {
  it("transition / animation 的时长只能是 var(--motion-*)", () => {
    const bad: string[] = [];
    for (const rel of srcSourceFiles([".vue", ".css"]).filter((f) => f !== "styles/tokens.css")) {
      const css = styleText(rel);
      for (const m of css.matchAll(/(?:transition|animation)(?:-[\w-]+)?\s*:\s*([^;}\n]+)/g)) {
        const value = m[1];
        if (!/(?:transition|animation)/.test(m[0])) continue;
        if (/\b\d*\.?\d+m?s\b/.test(value)) bad.push(`${rel}: ${m[0].trim()}`);
      }
    }
    expect(bad, `动效时长必须用 var(--motion-fast|normal|slow)：\n${bad.join("\n")}`).toEqual([]);
  });

  it("tokens.css 定义了 --motion-slow，且减少动效时也置 0", () => {
    const tokens = repoFile("src/styles/tokens.css");
    expect(tokens).toMatch(/--motion-slow:\s*\d+m?s;/);
    const reduced = tokens.slice(tokens.indexOf("@media (prefers-reduced-motion: reduce)"));
    expect(reduced).toMatch(/--motion-slow:\s*0ms;/);
  });
});

describe("设置页视觉统一（U4）", () => {
  it("设置页与六个 Tab 不再有静态内联样式（只允许 :style 动态绑定）", () => {
    const bad: string[] = [];
    for (const rel of SETTINGS_FILES) {
      const tpl = templateText(rel);
      for (const m of tpl.matchAll(/(?<![:@\w-])style="([^"]*)"/g))
        bad.push(`${rel}: style="${m[1]}"`);
    }
    expect(bad, `静态内联样式应改成 .settings-* 类：\n${bad.join("\n")}`).toEqual([]);
  });

  it("全仓不再用内联 font-size 字面量（字号只走 --fs-* 令牌）", () => {
    const bad: string[] = [];
    for (const rel of srcSourceFiles([".vue"])) {
      const text = repoFile(`src/${rel}`);
      for (const m of text.matchAll(/style="[^"]*font-size:\s*(\d+(?:\.\d+)?px)/g)) {
        bad.push(`${rel}: font-size: ${m[1]}`);
      }
    }
    expect(bad, `内联 font-size 字面量应改成令牌或类：\n${bad.join("\n")}`).toEqual([]);
  });

  it("设置页共享类定义齐全（分区/行/说明/窄控件）", () => {
    const base = repoFile("src/styles/base.css");
    for (const cls of [
      ".settings-section",
      ".settings-head",
      ".settings-inline",
      ".settings-caption",
      ".settings-narrow",
      ".settings-grow",
      ".settings-actions",
    ]) {
      expect(base.includes(`${cls} {`) || base.includes(`${cls},`), `缺少 ${cls}`).toBe(true);
    }
  });
});

describe("加载三态有骨架（U4）", () => {
  it("骨架类已定义，且减少动效时关闭脉冲", () => {
    const base = repoFile("src/styles/base.css");
    expect(base).toMatch(/\.skeleton\s*\{/);
    expect(base).toMatch(/@keyframes skeleton-pulse/);
    expect(base).toMatch(/\.skeleton\s*\{\s*animation:\s*none/);
  });
});
