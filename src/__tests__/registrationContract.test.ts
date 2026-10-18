// @vitest-environment node
/**
 * 页面登记守卫（审计 A-7，2026-09-27）。
 *
 * 背景：本仓新增页面的惯例是"三处登记"（router / 侧栏 / 命令面板），但此前只有
 * 逐页手写的断言（iaContract 写了 /insights，duplicateReviewContract 写了 /duplicates），
 * `/vouchers`、`/print`、`/exports`、`/restore`、`/help` 全在缺口里——新页面漏登记
 * 不会让任何测试变红（2026-09-23 前 /exports、/restore 就曾在命令面板里缺登记）。
 *
 * 本守卫改判据为通用规则（连接键用 **path**，不是 label：/print 在侧栏叫「打印与导出」、
 * 路由 title 叫「打印中心」）：
 *   1. 每个静态叶子路由都必须登记在**命令面板**（面板 = 全量入口）；
 *   2. 侧栏是精选子集，**未登记侧栏的静态路由必须恰好是下列两条**——新增豁免要改本文件，
 *      因此会在 code review 里被看见；
 *   3. 动态路由已全部收编（编辑页并入右栏后不再有 `:id` 路由）；搜索命中等"去看某一张"
 *      一律走 `/vouchers?focus=<id>` 深链（由下钻契约守卫 `drilldownContract` 保证被读取）；
 *   4. 侧栏/面板用到的图标名必须在 FluentIcon.ICONS 里定义；
 *   5. 动态段必须写在静态兄弟之后（否则 /vouchers/new 会被 /vouchers/:id 吃掉）。
 *
 * 纯文本解析源码（与其它 *Contract.test.ts 一致，不 mount 组件）。
 */
import { describe, expect, it } from "vitest";
import { repoFile } from "./helpers/designGuard";


const routerSrc = repoFile("src/router/index.ts");
const layoutSrc = repoFile("src/layouts/WorkbenchLayout.vue");
const paletteSrc = repoFile("src/components/CommandPalette.vue");
const iconSrc = repoFile("src/components/FluentIcon.vue");

/** 允许"只在面板、不在侧栏"的静态路由（表单页与搜索页由命令面板承载） */
const SIDEBAR_OPTIONAL = ["/search", "/vouchers/new"];

/** router 里的叶子路由（排除父级 "" 与带 redirect 的兼容条目） */
function leafRoutes(src: string): string[] {
  const hits = [...src.matchAll(/path:\s*"([^"]*)"/g)];
  return hits
    .map((m, i) => ({
      path: m[1],
      // 本条正文 = 本条 path 到下一处 path（router 表是扁平数组，比花括号配对更稳）
      body: src.slice(m.index!, i + 1 < hits.length ? hits[i + 1].index! : src.length),
    }))
    // 只取子路由：空 path 是索引重定向，以 "/" 开头的是父级（本仓只有布局那一条）
    .filter((e) => e.path !== "" && !e.path.startsWith("/") && !/redirect:/.test(e.body))
    .map((e) => `/${e.path}`);
}

/** 侧栏登记的 to（模板里的 :to="item.to" 不匹配，只取 navGroups 里的字面量） */
function sidebarPaths(src: string): string[] {
  return [...src.matchAll(/to:\s*"(\/[^"]*)"/g)].map((m) => m[1]);
}

/** 面板登记的字面量 push（动态模板 `/vouchers/${id}` 用反引号，不在此列） */
function palettePaths(src: string): string[] {
  return [...src.matchAll(/router\.push\("(\/[^"]*)"\)/g)].map((m) => m[1]);
}

/** FluentIcon 的 ICONS 字典键 */
function iconNames(src: string): string[] {
  const start = src.indexOf("const ICONS");
  const end = src.indexOf("\n};", start);
  return [...src.slice(start, end).matchAll(/^\s{2}([a-z0-9-]+):/gm)].map((m) => m[1]);
}

const routes = leafRoutes(routerSrc);
const staticRoutes = routes.filter((r) => !r.includes(":"));
const dynamicRoutes = routes.filter((r) => r.includes(":"));
const sidebar = sidebarPaths(layoutSrc);
const palette = palettePaths(paletteSrc);

describe("页面登记：router ↔ 侧栏 ↔ 命令面板", () => {
  it("每个静态叶子路由都在命令面板有入口（面板 = 全量入口）", () => {
    const missing = staticRoutes.filter((r) => !palette.includes(r));
    expect(
      missing,
      `这些页面没有命令面板入口（用户在面板里搜不到，只能手敲 URL）：\n${missing.join("\n")}`,
    ).toEqual([]);
  });

  it("未登记侧栏的静态路由恰好是那两条已声明例外", () => {
    const notInSidebar = staticRoutes.filter((r) => !sidebar.includes(r)).sort();
    expect(
      notInSidebar,
      "侧栏里少/多了页面：新增页面要么登记侧栏，要么把例外加进本守卫并说明理由",
    ).toEqual([...SIDEBAR_OPTIONAL].sort());
  });

  it("动态路由已全部收编：`/vouchers/:id` 编辑页并入右栏后不再有 :id 路由", () => {
    expect(
      dynamicRoutes,
      "新增动态路由须一并设计入口，并在此处登记（v0.19 起编辑页已并入资料库右栏）",
    ).toEqual([]);
  });

  it("「去看某一张」一律走 `/vouchers?focus=` 深链（编辑页入口的替代通道）", () => {
    // 命令面板的搜索结果是最容易出现"跳过去但没落点"的一处，这里点名它
    expect(paletteSrc, "搜索结果应带 focus 深链到资料库").toMatch(
      /router\.push\(\{\s*path:\s*"\/vouchers",\s*query:\s*\{\s*focus:/,
    );
  });

  it("侧栏与面板用到的图标名都在 FluentIcon 里定义", () => {
    const defined = iconNames(iconSrc);
    expect(defined.length, "FluentIcon.ICONS 解析失败或数量异常").toBeGreaterThan(10);
    const used = [
      ...layoutSrc.matchAll(/icon:\s*"([a-z0-9-]+)"/g),
      ...paletteSrc.matchAll(/icon:\s*"([a-z0-9-]+)"/g),
    ].map((m) => m[1]);
    expect(used.length, "侧栏/面板 icon 解析失败").toBeGreaterThan(5);
    const missing = [...new Set(used)].filter((n) => !defined.includes(n));
    expect(missing, `图标名未在 FluentIcon.ICONS 定义（会渲染成空白）：\n${missing.join("\n")}`).toEqual([]);
  });

  it("动态段排在静态兄弟之后（否则 /vouchers/new 会被 /vouchers/:id 吃掉）", () => {
    const problems: string[] = [];
    for (const dyn of dynamicRoutes) {
      const prefix = dyn.slice(0, dyn.indexOf("/:"));
      for (const stat of staticRoutes.filter((r) => r.startsWith(`${prefix}/`))) {
        if (routerSrc.indexOf(`path: "${stat.slice(1)}"`) > routerSrc.indexOf(`path: "${dyn.slice(1)}"`)) {
          problems.push(`${stat} 必须写在 ${dyn} 之前（vue-router 先匹配先胜）`);
        }
      }
    }
    expect(problems, problems.join("\n")).toEqual([]);
  });
});
