// @vitest-environment node
/**
 * 信息架构守卫（v0.27 工作台重构）。
 *
 * 不变量：
 * - 首页是工作台，四个固定页签（工作台/数据/对账/会计投影）；
 * - 无侧栏、无每页标题栏、无系统窗口拖拽区（网页端为唯一基准）；
 * - 辅助功能走 URL 弹层；重复核查只在工作台；
 * - 数据页是全站唯一的新建/导入/打印导出入口；
 * - 旧路由全部重定向，书签不断。
 */
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { repoFile } from "./helpers/designGuard";

const root = fileURLToPath(new URL("../../", import.meta.url));
const routerSrc = repoFile("src/router/index.ts");
const shellSrc = repoFile("src/layouts/AppShell.vue");
const topBarSrc = repoFile("src/components/shell/TopBar.vue");
const modalHostSrc = repoFile("src/components/shell/ModalHost.vue");
const workbenchSrc = repoFile("src/pages/WorkbenchPage.vue");
const dataSrc = repoFile("src/pages/DataPage.vue");
const paletteSrc = repoFile("src/components/CommandPalette.vue");
const appSrc = repoFile("src/App.vue");
const baseCss = repoFile("src/styles/base.css");
const tokenCss = repoFile("src/styles/tokens.css");

describe("主骨架：工作台 + 四页签", () => {
  it("首页是工作台，旧 /inbox、/dashboard 只作重定向", () => {
    expect(routerSrc).toMatch(/\{\s*path:\s*"",\s*redirect:\s*"\/workbench"\s*\}/);
    expect(routerSrc).toMatch(/path:\s*"workbench",\s*name:\s*"workbench"/);
    expect(routerSrc).toMatch(/\{\s*path:\s*"inbox",\s*redirect:\s*"\/workbench"\s*\}/);
    expect(routerSrc).toMatch(/\{\s*path:\s*"dashboard",\s*redirect:\s*"\/workbench"\s*\}/);
    expect(routerSrc).toMatch(/\{\s*path:\s*"ledger",\s*redirect:\s*"\/reconcile"\s*\}/);
    expect(routerSrc).toMatch(/\{\s*path:\s*"vouchers",\s*redirect:\s*"\/data"\s*\}/);
    expect(routerSrc).toMatch(
      /\{\s*path:\s*"settings",\s*redirect:\s*"\/workbench\?modal=settings"\s*\}/,
    );
    expect(routerSrc).toMatch(
      /\{\s*path:\s*"restore",\s*redirect:\s*"\/workbench\?modal=backup"\s*\}/,
    );
    expect(routerSrc).toMatch(/path:\s*"vouchers\/:id"/);
    expect(routerSrc).not.toMatch(
      /component:\s*\(\)\s*=>\s*import\("\.\.\/pages\/InboxPage\.vue"\)/,
    );
    expect(routerSrc).not.toMatch(/DuplicateReviewPage\.vue/);
  });

  it("旧的侧栏/标题栏/移动 tabbar 已删除（不留可回退的第二套壳）", () => {
    for (const p of [
      "src/layouts/WorkbenchLayout.vue",
      "src/components/layout/SidebarResizeHandle.vue",
      "src/composables/sidebarCollapse.ts",
      "src/composables/useSidebarResize.ts",
    ]) {
      expect(existsSync(`${root}${p}`), `${p} 应已删除`).toBe(false);
    }
    expect(shellSrc).not.toContain("sidebar");
    expect(shellSrc).not.toContain("unified-toolbar");
    expect(shellSrc).not.toContain("mobile-tabbar");
    expect(shellSrc).toContain('id="main-content"');
    expect(shellSrc).toContain("<TopBar");
    expect(shellSrc).toContain("<ModalHost");
  });

  it("页签固定四项、靠左；备份在设置左侧；无窗口拖拽区", () => {
    for (const to of ["/workbench", "/data", "/reconcile", "/accounting"]) {
      expect(topBarSrc).toContain(`to: "${to}"`);
    }
    const order = ["/workbench", "/data", "/reconcile", "/accounting"].map((x) =>
      topBarSrc.indexOf(`to: "${x}"`),
    );
    expect(order).toEqual([...order].sort((a, b) => a - b));
    expect(topBarSrc.indexOf("open('backup')")).toBeGreaterThan(0);
    expect(topBarSrc.indexOf("open('backup')")).toBeLessThan(topBarSrc.indexOf("open('settings')"));
    expect(topBarSrc).not.toContain("data-tauri-drag-region");
    expect(shellSrc).not.toContain("data-tauri-drag-region");
    // 不以“标题栏 + 页面标题”双层骨架出现
    expect(topBarSrc).not.toContain("toolbar-title");
    expect(topBarSrc).not.toContain("toolbar-subtitle");
  });

  it("辅助功能走 URL 弹层，重复核查只在工作台", () => {
    expect(modalHostSrc).toContain('case "duplicates"');
    expect(modalHostSrc).toContain('case "settings"');
    expect(modalHostSrc).toContain('case "backup"');
    expect(modalHostSrc).toContain('case "export"');
    expect(modalHostSrc).toContain('case "help"');
    expect(workbenchSrc).toContain('modal: "duplicates"');
    expect(workbenchSrc).not.toContain('to: "/duplicates"');
    expect(workbenchSrc).not.toContain("/duplicates");
  });

  it("工作台不设新建/导入/台账比对入口；数据页是唯一新建/导入/导出入口", () => {
    expect(workbenchSrc).not.toContain("新建条目");
    expect(workbenchSrc).not.toContain("台账比对");
    expect(workbenchSrc).not.toMatch(/<button[^>]*>[^<]*导入/);
    expect(dataSrc).toContain("新建条目");
    expect(dataSrc).toContain("打印导出");
    expect(dataSrc).toContain("open('export')");
    expect(dataSrc).toContain("later('导入凭证')");
  });

  it("命令面板只登记四页 + 新建 + 记录搜索，无旧词与旧路由", () => {
    expect(paletteSrc).toMatch(/router\.push\("\/workbench"\)/);
    expect(paletteSrc).toMatch(/router\.push\("\/data"\)/);
    expect(paletteSrc).toMatch(/router\.push\("\/reconcile"\)/);
    expect(paletteSrc).toMatch(/router\.push\("\/accounting"\)/);
    expect(paletteSrc).not.toContain("仪表盘");
    expect(paletteSrc).not.toContain("/inbox");
    expect(paletteSrc).not.toContain("/duplicates");
    expect(paletteSrc).not.toContain("/print");
  });

  it("App.vue 保留 ⌘K 与全局任务中心；旧页面组件不再被路由懒加载", () => {
    expect(appSrc).toContain("togglePalette");
    expect(appSrc).toContain("TaskCenter");
    for (const page of [
      "InboxPage",
      "LibraryPage",
      "InsightsPage",
      "VoucherFormPage",
      "PrintCenterPage",
      "ExportHistoryPage",
      "SettingsPage",
      "RestorePage",
      "HelpPage",
      "DuplicateReviewPage",
      "LedgerPage",
    ]) {
      expect(routerSrc).not.toContain(page);
    }
  });

  it("新壳层样式存在，且以网页端为基准（顶栏 52px、页签、内容区）", () => {
    expect(baseCss).toMatch(/^\.topbar\s*\{/m);
    expect(baseCss).toMatch(/^\.tabstrip\s*\{/m);
    expect(baseCss).toMatch(/^\.content\s*\{/m);
    expect(tokenCss).toMatch(/--topbar-h:\s*52px/);
  });
});
