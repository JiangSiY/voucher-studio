// @vitest-environment node
/**
 * 重复核查页的**登记一致性**守卫（静态读源码，不挂载、不依赖后端）。
 *
 * 背景：本仓新增页面的惯例是"三处登记"——路由（`router/index.ts`）、侧栏
 * （`WorkbenchLayout.vue` 的 navGroups）、命令面板（`CommandPalette.vue`）。
 * 缺任一处都会让页面"只能靠手敲 URL 打开"，而这类遗漏不会让任何测试变红
 * （2026-09-23 前 `/exports`、`/restore` 就只在侧栏有、命令面板漏登记）。
 *
 * 本测试把三处登记固化成断言，并顺带守住：图标名必须在 `FluentIcon.vue` 里存在
 * （写错名字只会渲染成空白，不报错）。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function repoFile(rel: string): string {
  return readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");
}

const routerSrc = repoFile("src/router/index.ts");
const layoutSrc = repoFile("src/layouts/WorkbenchLayout.vue");
const paletteSrc = repoFile("src/components/CommandPalette.vue");
const iconSrc = repoFile("src/components/FluentIcon.vue");
const pageSrc = repoFile("src/pages/DuplicateReviewPage.vue");
const clientSrc = repoFile("src/api/client.ts");
const dupStoreSrc = repoFile("src/stores/dup.ts");

describe("重复核查页登记", () => {
  it("路由已注册（name/标题/懒加载组件齐备）", () => {
    expect(routerSrc).toMatch(
      /path:\s*"duplicates",\s*name:\s*"duplicates",\s*component:\s*\(\)\s*=>\s*import\("\.\.\/pages\/DuplicateReviewPage\.vue"\)/,
    );
  });

  it("静态段在 vouchers/:id 之前（否则会被动态段吃掉 → 编辑页报「凭证不存在」）", () => {
    const dupIdx = routerSrc.indexOf('path: "duplicates"');
    const dynIdx = routerSrc.indexOf('path: "vouchers/:id"');
    expect(dupIdx).toBeGreaterThan(-1);
    expect(dynIdx).toBeGreaterThan(-1);
    expect(dupIdx).toBeLessThan(dynIdx);
  });

  it("侧栏已登记（含图标与待处理徽标）", () => {
    expect(layoutSrc).toMatch(
      /\{\s*to:\s*"\/duplicates",\s*icon:\s*"duplicate",\s*label:\s*"[^"]+"\s*,\s*badge:[^}]*\}/,
    );
    // 徽标必须接共用 store 的待处理数（各页面各查一次会出现互相矛盾的瞬间）
    expect(layoutSrc).toMatch(/badge:\s*dupStore\.pending\.groups/);
    expect(layoutSrc).toContain("nav-badge");
  });

  it("命令面板已登记（可直接搜「重复」跳转）", () => {
    expect(paletteSrc).toMatch(/router\.push\("\/duplicates"\)/);
    expect(paletteSrc).toMatch(/label:\s*"[^"]*重复/);
  });

  it("图标名在 FluentIcon 里存在（写错只会渲染成空白，不报错）", () => {
    expect(iconSrc).toMatch(/^\s*duplicate:\s*$/m);
  });
});

describe("重复核查页与后端契约", () => {
  it("队列/统计/历史三个只读命令都在用（缺一个都会让页面数据不完整）", () => {
    for (const m of ["scanDuplicateReview", "duplicatePendingSummary", "listDuplicateDecisions"]) {
      expect(pageSrc, `页面应调用 api.${m}`).toContain(`api.${m}`);
      expect(clientSrc, `client.ts 应封装 ${m}`).toContain(`${m}:`);
    }
  });

  it("两个写命令都在用：标记「不是重复」与「保留一张丢一张」", () => {
    expect(pageSrc).toContain("api.resolveDuplicatePair");
    expect(pageSrc).toContain("api.removeDuplicateVoucher");
  });

  it("删除走二次确认（危险操作不得一点就走）", () => {
    expect(pageSrc).toContain("ConfirmDialog");
    expect(pageSrc).toMatch(/confirmRemove/);
    // 确认文案必须说清"不可恢复"与"留痕"，否则用户不知道点了会怎样
    expect(pageSrc).toMatch(/不可恢复/);
    expect(pageSrc).toMatch(/留痕/);
  });

  it("图片路径走 useAttachmentFallback（不得自行拼分隔符——曾导致 macOS 缩略图全空白）", () => {
    expect(pageSrc).toContain("pickCandidate");
    expect(pageSrc).toContain("nextFallbackStep");
    expect(pageSrc).not.toMatch(/replace\([^)]*\\\\/);
  });

  it("已判定的对由后端排除（前端不得自行过滤，否则口径会分叉）", () => {
    // 提示文案要如实说明"已处置的不再列出"
    expect(pageSrc).toMatch(/不再列出/);
  });
});

describe("三处提示都指向处置（不再「只提示不处理」）", () => {
  const dashboardSrc = repoFile("src/pages/DashboardPage.vue");
  const formSrc = repoFile("src/pages/VoucherFormPage.vue");
  const appSrc = repoFile("src/App.vue");

  it("仪表盘面板只显示未处置的对（不得退回全量扫描——那正是「一直挂着提示」的成因）", () => {
    expect(dashboardSrc).toContain("api.scanDuplicateReview");
    expect(dashboardSrc).not.toContain("api.scanDuplicates");
    expect(dashboardSrc).toMatch(/去核查/);
  });

  it("保存提示带「去核查」动作 + 深链到那一对", () => {
    expect(formSrc).toMatch(/label:\s*"去核查"/);
    expect(formSrc).toMatch(/path:\s*"\/duplicates"/);
    expect(formSrc).toContain("pairKey");
  });

  it("本次会话内同一对只提示一次（连续保存同一张不重复弹）", () => {
    expect(formSrc).toContain("wasNotified");
    expect(formSrc).toContain("markNotified");
    expect(dupStoreSrc).toMatch(/const notified = new Set<string>\(\)/);
  });

  it("待处理数是共用 store（侧栏徽标与仪表盘不得各查一次）", () => {
    expect(layoutSrc).toContain("useDupStore");
    expect(dashboardSrc).toContain("useDupStore");
    expect(appSrc).toMatch(/dupStore\.refresh\(\)/);
    expect(appSrc).toMatch(/safeListen\("dup:changed"/);
  });
});
