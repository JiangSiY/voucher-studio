// @vitest-environment node
/**
 * 页面登记守卫（v0.27 工作台重构）。
 *
 * 规则：
 *   1. 非登录静态页必须出现在命令面板（面板 = 全量页面入口）；
 *   2. 没有侧栏，「精选子集」概念随之取消；
 *   3. 动态路由已全部收编：条目详情改为 `/data?focus=<id>` 深链，不允许 `:id` 页面；
 *   4. 面板/数据页用到的图标名必须在 FluentIcon.ICONS 定义；
 *   5. 登录页独立于主壳，不登记面板。
 */
import { describe, expect, it } from "vitest";
import { repoFile } from "./helpers/designGuard";

const routerSrc = repoFile("src/router/index.ts");
const paletteSrc = repoFile("src/components/CommandPalette.vue");
const topBarSrc = repoFile("src/components/shell/TopBar.vue");
const iconSrc = repoFile("src/components/FluentIcon.vue");

/** 只在主壳外、不需要命令面板入口的页面 */
const STANDALONE = ["/login"];

/** router 叶子路由（排除父级 "" 与 redirect 兼容条目） */
function leafRoutes(src: string): string[] {
  const hits = [...src.matchAll(/path:\s*"([^"]*)"/g)];
  return hits
    .map((m, i) => ({
      path: m[1],
      body: src.slice(m.index!, i + 1 < hits.length ? hits[i + 1].index! : src.length),
    }))
    .filter((e) => e.path !== "" && !e.path.startsWith("/") && !/redirect:/.test(e.body))
    .map((e) => `/${e.path}`);
}

function palettePaths(src: string): string[] {
  return [...src.matchAll(/router\.push\("(\/[^"]*)"\)/g)].map((m) => m[1]);
}

function iconNames(src: string): string[] {
  const start = src.indexOf("const ICONS");
  const end = src.indexOf("\n};", start);
  return [...src.slice(start, end).matchAll(/^\s{2}([a-z0-9-]+):/gm)].map((m) => m[1]);
}

const routes = leafRoutes(routerSrc);
const staticRoutes = routes.filter((r) => !r.includes(":"));
const dynamicRoutes = routes.filter((r) => r.includes(":"));
const palette = palettePaths(paletteSrc);

describe("页面登记：router ↔ 命令面板", () => {
  it("除登录外每个静态页都在命令面板有入口", () => {
    const missing = staticRoutes
      .filter((r) => !STANDALONE.includes(r))
      .filter((r) => !palette.includes(r));
    expect(missing, `这些页面没有命令面板入口：\n${missing.join("\n")}`).toEqual([]);
  });

  it("四个固定页签与命令面板同源", () => {
    const tabs = [...topBarSrc.matchAll(/to:\s*"(\/[^"]*)"/g)].map((m) => m[1]);
    expect(tabs).toEqual(["/workbench", "/data", "/reconcile", "/accounting"]);
    for (const t of tabs) expect(palette).toContain(t);
  });

  it("动态路由已收编：不存在 `:id` 页面，详情走 `/data?focus=`", () => {
    expect(dynamicRoutes).toEqual([]);
    expect(paletteSrc).toMatch(/router\.push\(\{\s*path:\s*"\/data",\s*query:\s*\{\s*focus:/);
  });

  it("面板与壳层图标名都在 FluentIcon 里定义", () => {
    const defined = iconNames(iconSrc);
    expect(defined.length).toBeGreaterThan(10);
    const used = [
      ...paletteSrc.matchAll(/icon:\s*"([a-z0-9-]+)"/g),
      ...topBarSrc.matchAll(/name="([a-z0-9-]+)"/g),
    ].map((m) => m[1]);
    const missing = [...new Set(used)].filter((n) => !defined.includes(n));
    expect(missing, `图标名未定义：\n${missing.join("\n")}`).toEqual([]);
  });
});
