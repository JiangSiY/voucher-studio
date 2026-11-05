// @vitest-environment node
/**
 * Loon 视觉契约（2026-11-04）：渐变画布 + 玻璃内容卡 + 纸面打印。
 *
 * 依据：`docs/UI-设计规范-Loon.md`（原 macOS 27 平铺风，自 2026-11-04 Loon 化批次起改为玻璃卡片风）。
 *
 * 为什么要有这个契约：
 * - 画布、卡片、发丝线、纸面是**跨页面**的四个材质层，任何一个页面绕过令牌直接写
 *   `--bg-content`，都会在渐变上戳出一块不透明白斑；这类回归靠肉眼很难在 10 个页面里看全。
 * - 打印预览的纸面永远是白的，深色主题必须局部把文字切回墨色——2026-11-04 审计实测过
 *   「白底白字、预览全白」，这里用契约把它钉住。
 */
import { describe, expect, it } from "vitest";
import { repoFile } from "./helpers/designGuard";

/** 样式块里 `selector { ... }` 的声明体（首个匹配，够用：本仓库未嵌套） */
function cssBlock(css: string, selector: string): string {
  const at = css.indexOf(`${selector} {`);
  if (at < 0) return "";
  const start = css.indexOf("{", at);
  const end = css.indexOf("}", start);
  return css.slice(start + 1, end);
}

function templateText(rel: string): string {
  const text = repoFile(`src/${rel}`);
  const m = text.match(/<template>([\s\S]*)<\/template>/);
  return m ? m[1] : "";
}

describe("Loon 材质令牌", () => {
  const tokens = repoFile("src/styles/tokens.css");

  it("画布/卡片/发丝/模糊/纸面令牌在浅色、深色与首帧兜底三处都有定义", () => {
    // 纸面不随主题变化（打印永远是白纸），只定义一次；其余材质必须三处同步。
    const once = new Set(["--radius-card", "--bg-paper", "--paper-ink"]);
    for (const name of [
      "--canvas-gradient",
      "--bg-card",
      "--card-hairline",
      "--blur-card",
      "--shadow-card",
      "--bg-paper",
      "--paper-ink",
      "--radius-card",
    ]) {
      const hits = tokens.match(new RegExp(`^\\s*${name}\\s*:`, "gm")) ?? [];
      expect(hits.length, `${name} 定义次数`).toBeGreaterThanOrEqual(once.has(name) ? 1 : 2);
    }
  });

  it("减少透明度时卡片回到实色并关闭模糊", () => {
    const at = tokens.indexOf("@media (prefers-reduced-transparency: reduce)");
    expect(at).toBeGreaterThan(-1);
    const block = tokens.slice(at, at + 900);
    expect(block).toContain("--blur-card: none");
    expect(block).toContain("--bg-card: var(--bg-card-solid)");
  });
});

describe("壳层与卡片材质", () => {
  const base = repoFile("src/styles/base.css");

  it("画布渐变挂在 app-shell，main-area 不再铺不透明底色", () => {
    expect(cssBlock(base, ".app-shell")).toContain("background: var(--canvas-gradient)");
    const main = cssBlock(base, ".main-area");
    expect(main).toContain("background: transparent");
    expect(main).not.toContain("var(--bg-content)");
  });

  it("fluent-panel 是玻璃卡：card 底 + 大圆角 + 软阴影 + 模糊", () => {
    const panel = cssBlock(base, ".fluent-panel");
    expect(panel).toContain("var(--bg-card)");
    expect(panel).toContain("var(--radius-card)");
    expect(panel).toContain("var(--shadow-card)");
    expect(panel).toContain("var(--blur-card)");
  });

  it("工具栏有副标题槽，且副标题由路由声明", () => {
    expect(cssBlock(base, ".toolbar-subtitle")).toContain("var(--type-caption)");
    expect(templateText("layouts/WorkbenchLayout.vue")).toContain('class="toolbar-subtitle"');
    expect(repoFile("src/router/index.ts")).toContain("subtitle: ");
  });

  it("Inbox / Insights 不再在页内重复大标题（标题只由工具栏承担）", () => {
    expect(templateText("pages/InboxPage.vue")).not.toContain("<h2");
    expect(templateText("pages/InsightsPage.vue")).not.toContain("<h2");
  });

  it("设置页正文是一张玻璃卡", () => {
    expect(templateText("pages/SettingsPage.vue")).toContain('class="settings-body"');
    expect(cssBlock(base, ".settings-body")).toContain("var(--bg-card)");
  });
});

describe("纸面与可读性", () => {
  const base = repoFile("src/styles/base.css");

  it("打印预览是白纸黑字：深色主题下也局部覆盖文字与分隔线", () => {
    const paper = cssBlock(base, ".print-area");
    expect(paper).toContain("background: var(--bg-paper)");
    expect(paper).toContain("color: var(--paper-ink)");
    expect(paper).toContain("--label-primary: var(--paper-ink)");
    expect(paper).toContain("--separator: var(--paper-line)");
    expect(paper).toContain("--surface: var(--bg-paper)");
  });
});
