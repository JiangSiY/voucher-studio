// @vitest-environment jsdom
/**
 * 收件箱的**待办语义**守卫（用户 2026-09-27 要求）：
 * "待处理事项以待办显示，完成之后就隐藏，提高空间利用率"。
 *
 * 锁住的契约：
 * - count > 0 的项渲染（标题 + 数量 + 动作）；
 * - **count == 0 的项完全不渲染**（不是变灰保留——那正是被否掉的做法）；
 * - 全部清零时只剩一条细横幅（"待办都清空了"），不占版面；
 * - 点某一行触发它的动作（这里验证"继续核对"会跳到带筛选的列表页）。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import type { DashboardTodos } from "../../api/client";
import { api } from "../../api/client";
import InboxPage from "../InboxPage.vue";

const push = vi.fn();

vi.mock("../../api/client", () => ({
  api: {
    dashboardTodos: vi.fn(),
    voucherStats: vi.fn(),
    listVouchers: vi.fn(),
    listExportBatches: vi.fn(),
    exportCoverage: vi.fn(),
    runOcrPending: vi.fn(),
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("vue-router", () => ({
  useRoute: () => ({ query: {} }),
  useRouter: () => ({ push }),
}));

const mockApi = vi.mocked(api);

function todos(over: Partial<DashboardTodos> = {}): DashboardTodos {
  return {
    ocrPending: 0,
    amountMissing: 0,
    staleDrafts: 0,
    staleDraftDays: 7,
    unlockedMonths: 0,
    latestBackup: { fileName: "b.zip", modifiedAt: "2026-09-27T08:00:00+08:00" },
    backupStale: false,
    dupPairs: 0,
    dupHighPairs: 0,
    dupGroups: 0,
    ...over,
  };
}

async function mountPage(): Promise<VueWrapper> {
  const w = mount(InboxPage, { attachTo: document.body });
  await flushPromises();
  return w;
}

beforeEach(() => {
  setActivePinia(createPinia());
  mockApi.dashboardTodos.mockResolvedValue(
    todos({ ocrPending: 5, dupPairs: 6, dupHighPairs: 2, amountMissing: 3 }),
  );
  mockApi.voucherStats.mockResolvedValue({
    total: 194,
    draft: 4,
    reviewed: 7,
    exported: 100,
    archived: 83,
    attachments: 194,
    thisMonth: 81,
  });
  mockApi.listVouchers.mockResolvedValue({ items: [], total: 0, thumbs: {} });
  mockApi.listExportBatches.mockResolvedValue([]);
  // 「待打印/导出」的计数来自"从未导出"（审计 P1-3 修的口径），不是 stats.reviewed
  mockApi.exportCoverage.mockResolvedValue({
    total: 10,
    exported: 3,
    neverExported: 7,
    byFormat: [],
  });
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("收件箱待办清单", () => {
  it("有待办时逐条渲染（标题 + 数量 + 动作），数值来自 dashboardTodos/voucher_stats", async () => {
    const w = await mountPage();
    const text = w.text();
    expect(text).toContain("待识别");
    expect(text).toContain("5");
    expect(text).toContain("开始识别");
    expect(text).toContain("疑似重复待处置");
    expect(text).toContain("6");
    expect(text).toContain("去核查");
    expect(text).toContain("待核对");
    expect(text).toContain("4"); // 来自 voucher_stats.draft
    expect(text).toContain("未填金额");
    // 审计 P1-3：待打印/导出用"从未导出"口径（exportCoverage.neverExported），不再用 stats.reviewed
    expect(text).toContain("待打印 / 导出");
    expect(text).toContain("7");
    expect(text).toContain("去看清单");
  });

  it("**数量为 0 的项不渲染**（不是变灰保留）", async () => {
    mockApi.dashboardTodos.mockResolvedValue(todos({ ocrPending: 2 }));
    mockApi.voucherStats.mockResolvedValue({
      total: 1,
      draft: 1,
      reviewed: 0,
      exported: 0,
      archived: 0,
      attachments: 1,
      thisMonth: 1,
    });
    // 「待打印」的计数来自"从未导出"（审计 P1-3 改的口径）
    mockApi.exportCoverage.mockResolvedValue({
      total: 1,
      exported: 1,
      neverExported: 0,
      byFormat: [],
    });
    const w = await mountPage();
    const text = w.text();
    expect(text).toContain("待识别");
    expect(text).not.toContain("疑似重复"); // dupPairs = 0 → 整行消失
    expect(text).not.toContain("待打印"); // neverExported = 0 → 整行消失
    expect(text).not.toContain("未填金额");
    expect(text).not.toContain("未封账");
    expect(text).not.toContain("该备份了"); // backupStale = false
    // 4 行都清零的项不应该在 DOM 里留下任何"0 张"文本
    expect(text).not.toContain("0 张");
  });

  it("全部清零：只剩一条细横幅提示，不出现任何待办行", async () => {
    mockApi.dashboardTodos.mockResolvedValue(todos());
    mockApi.voucherStats.mockResolvedValue({
      total: 5,
      draft: 0,
      reviewed: 0,
      exported: 5,
      archived: 0,
      attachments: 5,
      thisMonth: 0,
    });
    mockApi.exportCoverage.mockResolvedValue({
      total: 5,
      exported: 5,
      neverExported: 0,
      byFormat: [],
    });
    const w = await mountPage();
    expect(w.text()).toContain("待办都清空了");
    expect(w.find(".todo").exists()).toBe(false);
    expect(w.text()).toContain("打开「全部凭证」");
  });

  it("点「待核对」→ 带草稿筛选进资料库", async () => {
    const w = await mountPage();
    const row = w.findAll(".todo").find((b) => b.text().includes("待核对"))!;
    await row.trigger("click");
    // 待核对 = 全部草稿（不带 staleDays；陈旧口径在"陈旧草稿"那一行，见下一个用例）
    expect(push).toHaveBeenCalledWith({ path: "/vouchers", query: { status: "draft" } });
  });

  it("点「陈旧草稿」→ 必须带 staleDays（否则列出全部草稿，审计 P1-2）", async () => {
    // 该行只在 staleDrafts > 0 时渲染（count==0 不渲染是既定行为）
    mockApi.dashboardTodos.mockResolvedValue(todos({ staleDrafts: 3, staleDraftDays: 7 }));
    const w = await mountPage();
    const row = w.findAll(".todo").find((b) => b.text().includes("天没动的草稿"))!;
    await row.trigger("click");
    expect(push).toHaveBeenCalledWith({
      path: "/vouchers",
      query: { status: "draft", staleDays: "7" },
    });
  });

  it("「查看待识别」下钻带多值状态（pending,failed，审计 P1-1）", async () => {
    const w = await mountPage();
    const link = w.findAll(".todo-secondary").find((b) => b.text().includes("查看待识别"))!;
    await link.trigger("click");
    expect(push).toHaveBeenCalledWith({
      path: "/vouchers",
      query: { ocrStatus: "pending,failed" },
    });
  });

  it("待识别行使按钮就绪并可触发批量识别", async () => {
    mockApi.runOcrPending.mockResolvedValue(5);
    const w = await mountPage();
    const btn = w.findAll(".todo").find((b) => b.text().includes("开始识别"))!;
    await btn.trigger("click");
    await flushPromises();
    expect(mockApi.runOcrPending).toHaveBeenCalled();
  });
});
