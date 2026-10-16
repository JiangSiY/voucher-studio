// @vitest-environment jsdom
/**
 * 侧栏开合（用户 2026-10-16 要求："去掉侧边栏分组功能，增加侧边栏收起功能，收起后只显示图标"）。
 *
 * 锁住的契约：
 * - 导航是**扁平一列**：分组标题按钮（`.nav-group`）不复存在，九个条目全部渲染；
 * - 工具栏有侧栏开关，点一下收起（写根属性 `data-sidebar-collapsed="true"`，CSS 依据它只留图标）；
 * - 再点一下恢复展开（属性回 `"false"`）；
 * - 启动时尊重已保存的收起状态（`appearance.sidebarCollapsed`，跨启动保持）；
 * - 收起是**纯 CSS 行为**：条目标签与 `title` 必须留在 DOM 里（v-if 掉就没有图标栏的可发现性兜底了）。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { api } from "../../api/client";
import { defaultConfig } from "../../config-defaults";
import { useConfigStore } from "../../stores/config";
import { useDupStore } from "../../stores/dup";
import WorkbenchLayout from "../WorkbenchLayout.vue";

// 当前路由由用例改写
let currentPath = "/inbox";

vi.mock("../../api/client", () => ({
  api: { dashboardTodos: vi.fn(), duplicatePendingSummary: vi.fn(), getAppInfo: vi.fn(), getConfig: vi.fn() },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("vue-router", () => ({
  useRoute: () => ({ path: currentPath, meta: { title: "x" } }),
  useRouter: () => ({ push: vi.fn() }),
  RouterLink: { template: "<a><slot /></a>" },
  RouterView: { template: "<div />" },
}));

const mockApi = vi.mocked(api);

const NAV_LABELS = [
  "待处理",
  "全部凭证",
  "重复核查",
  "洞察",
  "打印与导出",
  "导出历史",
  "设置",
  "备份与恢复",
  "帮助",
];

function navLinks(w: VueWrapper) {
  return w.findAll(".sidebar-nav a");
}

async function mountLayout(): Promise<VueWrapper> {
  const w = mount(WorkbenchLayout, {
    global: {
      stubs: {
        // 组件里用的是 <router-link>；测试未安装 router 插件，需显式桩成真 <a>，否则类名查不到
        RouterLink: { template: '<a :class="$attrs.class"><slot /></a>' },
        "router-link": { template: '<a :class="$attrs.class"><slot /></a>' },
        RouterView: true,
        SidebarResizeHandle: true,
      },
    },
  });
  await flushPromises();
  return w;
}

beforeEach(() => {
  setActivePinia(createPinia());
  currentPath = "/inbox";
  mockApi.dashboardTodos.mockResolvedValue({
    ocrPending: 0,
    amountMissing: 0,
    staleDrafts: 0,
    staleDraftDays: 7,
    unlockedMonths: 0,
    latestBackup: null,
    backupStale: false,
    dupPairs: 6,
    dupHighPairs: 2,
    dupGroups: 3,
  } as never);
});

afterEach(() => {
  document.body.innerHTML = "";
  delete document.documentElement.dataset.sidebarCollapsed;
  vi.clearAllMocks();
});

describe("侧栏：扁平导航 + 整栏收起", () => {
  it("导航扁平一列：没有分组标题，九个条目全部渲染", async () => {
    const w = await mountLayout();
    expect(w.findAll(".nav-group")).toHaveLength(0);
    expect(navLinks(w)).toHaveLength(NAV_LABELS.length);
    for (const label of NAV_LABELS) expect(w.text()).toContain(label);
  });

  it("点工具栏开关即收起：根属性置 true，aria-expanded 与选中态同步翻转", async () => {
    const w = await mountLayout();
    expect(w.get(".sidebar-toggle").attributes("aria-expanded")).toBe("true");
    // 侧栏可见时按钮带选中态（真机走查：箭头版被误读成"返回"，状态改由选中态 + 面板图标表达）
    expect(w.get(".sidebar-toggle").classes()).toContain("is-active");
    expect(document.documentElement.dataset.sidebarCollapsed).toBe("false");

    await w.get(".sidebar-toggle").trigger("click");
    await flushPromises();
    expect(document.documentElement.dataset.sidebarCollapsed).toBe("true");
    expect(w.get(".sidebar-toggle").attributes("aria-expanded")).toBe("false");
    expect(w.get(".sidebar-toggle").classes()).not.toContain("is-active");
  });

  it("再点一下恢复展开", async () => {
    const w = await mountLayout();
    await w.get(".sidebar-toggle").trigger("click");
    await flushPromises();
    await w.get(".sidebar-toggle").trigger("click");
    await flushPromises();
    expect(document.documentElement.dataset.sidebarCollapsed).toBe("false");
    expect(w.get(".sidebar-toggle").attributes("aria-expanded")).toBe("true");
  });

  it("启动时尊重已保存的收起状态", async () => {
    const store = useConfigStore();
    const cfg = defaultConfig();
    cfg.appearance.sidebarCollapsed = true;
    store.config = cfg;
    await flushPromises();

    const w = await mountLayout();
    expect(document.documentElement.dataset.sidebarCollapsed).toBe("true");
    expect(w.get(".sidebar-toggle").attributes("aria-expanded")).toBe("false");
  });

  it("收起后标签与 title 仍留在 DOM（图标栏的可发现性兜底）", async () => {
    useDupStore().pending = { groups: 6, pairs: 6 };
    const w = await mountLayout();
    const dup = navLinks(w).find((a) => a.text().includes("重复核查"))!;
    expect(dup.text()).toContain("6");
    expect(dup.attributes("title")).toContain("6");

    await w.get(".sidebar-toggle").trigger("click");
    await flushPromises();
    expect(dup.text()).toContain("重复核查");
    expect(dup.attributes("title")).toContain("重复核查");
  });
});
