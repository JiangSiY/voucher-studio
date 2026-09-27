// @vitest-environment jsdom
/**
 * 侧栏分组折叠（用户 2026-09-27 要求："收件箱、资料库、设置做成可折叠，默认折叠"）。
 *
 * 锁住的契约：
 * - **默认全部收起**（初次进入时不展开任何组，除了……见下条）；
 * - **当前页面所在组自动展开**（否则收起后看不到自己在哪一页、高亮也失去意义）；
 * - 手动点击可展开/收起，且手动状态优先于自动规则；
 * - 收起时**组标题上显示组内徽标合计**（不展开也知道有事要办）。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { api } from "../../api/client";
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

function groupButton(w: VueWrapper, label: string) {
  return w.findAll(".nav-group").find((b) => b.text().includes(label))!;
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
  vi.clearAllMocks();
});

describe("侧栏分组折叠", () => {
  it("默认收起：只有当前页所在组展开，其余组的条目不可见", async () => {
    const w = await mountLayout();
    // 收件箱组（当前页 /inbox）自动展开 → 其条目可见
    expect(w.text()).toContain("待处理");
    // 资料库 / 设置两组默认收起 → 条目虽然渲染但带折叠类（CSS 隐藏）
    const collapsedItems = w.findAll("a.nav-item-collapsed");
    const texts = collapsedItems.map((a) => a.text());
    expect(texts).toContain("全部凭证");
    expect(texts).toContain("设置");
    // 组按钮的 aria-expanded 如实反映状态
    expect(groupButton(w, "收件箱").attributes("aria-expanded")).toBe("true");
    expect(groupButton(w, "资料库").attributes("aria-expanded")).toBe("false");
    expect(groupButton(w, "设置").attributes("aria-expanded")).toBe("false");
  });

  it("点击组标题可展开/收起，手动状态优先于自动规则", async () => {
    const w = await mountLayout();
    const lib = groupButton(w, "资料库");
    await lib.trigger("click");
    expect(groupButton(w, "资料库").attributes("aria-expanded")).toBe("true");

    // 再点一次 → 收起，即使当前页就在该组（手动优先）
    currentPath = "/insights";
    await groupButton(w, "资料库").trigger("click");
    expect(groupButton(w, "资料库").attributes("aria-expanded")).toBe("false");
  });

  it("切到别的页面时：那一组自动展开，当前页条目不再带折叠类", async () => {
    currentPath = "/settings";
    const w = await mountLayout();
    expect(groupButton(w, "设置").attributes("aria-expanded")).toBe("true");
    const settingsItem = w.findAll("a").find((a) => a.text() === "设置")!;
    expect(settingsItem.classes()).not.toContain("nav-item-collapsed");
    // 而上一组（收件箱）此时不再自动展开
    expect(groupButton(w, "收件箱").attributes("aria-expanded")).toBe("false");
  });

  it("收起时组标题上显示组内徽标合计（疑似重复 6 → 资料库组显示 6）", async () => {
    // 侧栏徽标读 dupStore；应用启动时会 refresh 一次，这里照同样路径准备状态
    useDupStore().pending = { groups: 6, pairs: 6 };
    const w = await mountLayout();
    const lib = groupButton(w, "资料库");
    expect(lib.text()).toContain("6");
    // 展开后不再重复显示（条目自身的徽标会显示）
    await lib.trigger("click");
    expect(groupButton(w, "资料库").text()).not.toContain("6");
  });
});
