// @vitest-environment node
/**
 * 信息架构守卫（v0.18 收件箱驱动重构）。
 *
 * 背景：用户要求的是**结构重做**（"不是在旧布局上增删改"）——主骨架从
 * 「仪表盘 / 凭证管理 / 数据与设置」三组 + 以统计为中心，改为
 * 「收件箱 / 资料库 / 设置」三组 + 以**流程**为中心（收件箱即首页）。
 *
 * 这类改动最容易在后续迭代里被悄悄改回去（比如有人把仪表盘加回导航），
 * 故把不变量固化成断言：首页是收件箱、侧栏是一列**扁平入口**（2026-10-16 用户要求去掉分组标题，
 * 开合语义改由"整栏收起"承担）、命令面板入口与侧栏一致、旧的 /dashboard 只保留重定向。
 */
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

function repoFile(rel: string): string {
  return readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");
}

const routerSrc = repoFile("src/router/index.ts");
const layoutSrc = repoFile("src/layouts/WorkbenchLayout.vue");
const paletteSrc = repoFile("src/components/CommandPalette.vue");
const appSrc = repoFile("src/App.vue");

describe("主骨架：收件箱驱动", () => {
  it("首页是收件箱；/dashboard 只作重定向保留（旧链接不 404，但页面已不存在）", () => {
    expect(routerSrc).toMatch(/\{\s*path:\s*"",\s*redirect:\s*"\/inbox"\s*\}/);
    expect(routerSrc).toMatch(
      /path:\s*"inbox",\s*name:\s*"inbox",\s*component:\s*\(\)\s*=>\s*import\("\.\.\/pages\/InboxPage\.vue"\)/,
    );
    expect(routerSrc).toMatch(/\{\s*path:\s*"dashboard",\s*redirect:\s*"\/inbox"\s*\}/);
    // 旧仪表盘页面文件必须已删除（不留无人引用的死页面）
    const root = fileURLToPath(new URL("../../", import.meta.url));
    expect(existsSync(`${root}src/pages/DashboardPage.vue`)).toBe(false);
  });

  it("侧栏为扁平一列：分组标题已删除，九个入口按流程顺序排列", () => {
    // 曾经的「收件箱 / 资料库 / 设置」三组标题按钮按用户要求删除（2026-10-16），别再加回来
    expect(layoutSrc).not.toMatch(/group:\s*"/);
    const routes = [...layoutSrc.matchAll(/to:\s*"(\/[^"]*)"/g)].map((m) => m[1]);
    expect(routes).toEqual([
      "/inbox",
      "/vouchers",
      "/duplicates",
      "/insights",
      "/print",
      "/exports",
      "/settings",
      "/restore",
      "/help",
    ]);
  });

  it("整栏收起：侧栏底部的整行开关 + 根属性 data-sidebar-collapsed（与主题/宽度同一通道）", () => {
    expect(layoutSrc).toMatch(/class="sidebar-toggle"/);
    expect(layoutSrc).toMatch(/aria-controls="app-sidebar"/);
    expect(layoutSrc).toMatch(/:aria-expanded="!sidebarCollapsed"/);
    // 开关图标必须是面板形（语义化）：真机走查里 chevron 版被读成"返回"（用户 2026-10-16 实报），别改回去
    expect(layoutSrc).toMatch(/<FluentIcon name="sidebar"/);
    expect(layoutSrc).not.toMatch(/chevron(Left|Right)/);
    const icons = repoFile("src/components/FluentIcon.vue");
    expect(icons).toContain("sidebar:");
    // 位置契约（用户 2026-10-16 明确要求）：在导航之后、版本行**上方**那一行；不再放工具栏
    expect(layoutSrc).toMatch(/<\/nav>[\s\S]*class="sidebar-toggle"[\s\S]*class="sidebar-footer"/);
    expect(layoutSrc).not.toMatch(/toolbar-btn sidebar-toggle/);
    // 观感是纯 CSS：收起后隐藏文字，条目标签留在 DOM 里
    const collapse = repoFile("src/composables/sidebarCollapse.ts");
    expect(collapse).toContain("dataset.sidebarCollapsed");
    const css = repoFile("src/styles/base.css");
    expect(css).toContain(':root[data-sidebar-collapsed="true"] .sidebar');
    expect(css).toContain(':root[data-sidebar-collapsed="true"] .sidebar-nav .nav-label');
    expect(css).toContain(".sidebar-toggle {");
    expect(css).toContain(':root[data-sidebar-collapsed="true"] .sidebar-toggle .nav-label');
  });

  it("侧栏入口与「待处理」徽标同源（inbox store），重复核查徽标沿用 dup store", () => {
    expect(layoutSrc).toMatch(/\{\s*to:\s*"\/inbox"[^}]*badge:\s*inboxStore\.pendingCount\s*\}/);
    expect(layoutSrc).toMatch(/\{\s*to:\s*"\/duplicates"[^}]*badge:\s*dupStore\.pending\.groups\s*\}/);
    expect(layoutSrc).toContain("useInboxStore");
  });

  it("窗口拖动区用 deep（bare 只在直点元素本身时生效，点标题会拖不动）", () => {
    // 依据：tauri/src/window/scripts/drag.js —— 无值只认 composedPath[0]，"deep" 才认整棵子树
    const dragRegions = [...layoutSrc.matchAll(/data-tauri-drag-region(="([^"]*)")?/g)];
    expect(dragRegions.length, "至少要有工具栏这一处拖动区").toBeGreaterThan(0);
    for (const m of dragRegions) {
      expect(m[2], "拖动区必须写 deep（写无值/true 会因点在子元素上而失效）").toBe("deep");
    }
    // 工具栏与侧栏顶部都应可拖（macOS 惯例：交通灯那条也能拖）
    expect(layoutSrc).toMatch(/class="unified-toolbar" data-tauri-drag-region="deep"/);
    expect(layoutSrc).toMatch(/class="sidebar-header" data-tauri-drag-region="deep"/);
  });

  it("工具栏不再放命令面板按钮（用户 2026-10-16：「基本没用过，没有价值」）", () => {
    // 工具栏只剩标题；按钮去掉后 ⌘K 快捷键与命令面板本身必须仍在
    expect(layoutSrc).not.toContain("toolbar-actions");
    expect(layoutSrc).not.toContain("toolbar-btn");
    expect(layoutSrc).not.toContain("命令面板（⌘K）");
    expect(appSrc).toContain("togglePalette");
    // 对应样式随之删除（留着就是死代码）
    expect(repoFile("src/styles/base.css")).not.toMatch(/\.unified-toolbar \.toolbar-(btn|actions|hint)\b/);
  });

  it("侧栏顶部让开交通灯带：上留白随 --toolbar-h 走（用户 2026-10-16 反馈「上方不协调」）", () => {
    const css = repoFile("src/styles/base.css");
    // 基础规则：VS 徽标必须落在红绿灯（y≈18–30）下方，不能只给 20px
    expect(css).toMatch(/\.sidebar-header \{[\s\S]{0,400}?padding: calc\(var\(--toolbar-h\) - var\(--s-2\)\) var\(--s-4\) var\(--s-4\)/);
    // 收起态与 ≤760 图标栏同样让开（否则两种收窄形态又会挤回红绿灯那一带）
    expect(css.match(/padding: calc\(var\(--toolbar-h\) - var\(--s-2\)\) 0 var\(--s-4\)/g) ?? []).toHaveLength(2);
  });

  it("命令面板与侧栏一致：收件箱/凭证/打印的入口都在，且没有残留的「仪表盘」入口", () => {
    expect(paletteSrc).toMatch(/router\.push\("\/inbox"\)/);
    expect(paletteSrc).toContain("全部凭证");
    expect(paletteSrc).toContain("打印与导出");
    expect(paletteSrc).not.toContain("仪表盘");
  });

  it("资料库组的「洞察」三处登记一致（趋势/排行/对账有了归属页）", () => {
    expect(routerSrc).toMatch(/path:\s*"insights",\s*name:\s*"insights"/);
    expect(layoutSrc).toMatch(/\{\s*to:\s*"\/insights"[^}]*label:\s*"洞察"/);
    expect(paletteSrc).toMatch(/router\.push\("\/insights"\)/);
  });

  it("收件箱页消费的数据源是待办聚合（不是旧的全量扫描）", () => {
    const inbox = repoFile("src/pages/InboxPage.vue");
    expect(inbox).toContain("useInboxStore");
    expect(inbox).toContain("api.voucherStats");
    expect(inbox).not.toContain("api.scanDuplicates");
    expect(inbox).not.toContain("api.scanDuplicateReview");
  });
});
