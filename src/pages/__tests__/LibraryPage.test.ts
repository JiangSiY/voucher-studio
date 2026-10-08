// @vitest-environment jsdom
/**
 * 资料库（双行列表 + 右栏编辑器）的交互契约：
 * - 选中行 → 右栏显示该张并预填字段（v0.19：右栏即编辑器，承载原编辑页全部字段）；
 * - 保存走统一校验（非法金额直接拦下、不调后端）；
 * - **双击行 = 展开编辑区**（v0.19 起不再跳独立编辑页）；`focus` 深链可聚焦不在本页的凭证；
 * - **跨页选择集**：翻页不丢选择（旧实现每次 load 清空，是用户抱怨过的行为）；
 * - 「按筛选全选 N 张」走只读 id 接口，不把 N 条对象拉回前端。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import type { Voucher } from "../../api/client";
import { api } from "../../api/client";
import LibraryPage from "../LibraryPage.vue";

const h = vi.hoisted(() => ({
  push: vi.fn(),
  replace: vi.fn(),
  route: { value: {} as Record<string, string> },
}));

vi.mock("../../api/client", () => ({
  api: {
    listVouchers: vi.fn(),
    getVoucher: vi.fn(),
    updateVoucher: vi.fn(),
    batchUpdateStatus: vi.fn(),
    deleteVouchers: vi.fn(),
    listVoucherIds: vi.fn(),
    listPeriodLocks: vi.fn(),
    checkVoucherDuplicates: vi.fn(),
    ocrPreviewFields: vi.fn(),
    runOcr: vi.fn(),
    pickAndReplaceImage: vi.fn(),
  },
}));
vi.mock("@tauri-apps/api/core", () => ({ convertFileSrc: (p: string) => `asset://${p}` }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("vue-router", () => ({
  useRoute: () => ({ query: h.route.value }),
  useRouter: () => ({ push: h.push, replace: h.replace }),
}));

const mockApi = vi.mocked(api);

function v(over: Partial<Voucher> = {}): Voucher {
  return {
    id: 1,
    voucherNo: "20260924-002",
    voucherDate: "2026-09-24",
    description: "润博纺织工程款",
    counterparty: "润博纺织",
    amountCents: 2500000,
    status: "exported",
    tags: ["工程款"],
    note: "",
    createdAt: "2026-09-24T10:00:00",
    updatedAt: "2026-09-24T10:00:00",
    ...over,
  } as Voucher;
}

const thumb = { filePath: "2026-09/a.jpg", enhancedPath: null, enhancedSource: "unknown", fileAbs: "/tmp/a.jpg", enhancedAbs: null };

async function mountPage(): Promise<VueWrapper> {
  const w = mount(LibraryPage, { attachTo: document.body });
  await flushPromises();
  return w;
}

beforeEach(() => {
  setActivePinia(createPinia());
  mockApi.listVouchers.mockResolvedValue({
    items: [v(), v({ id: 2, voucherNo: "20260924-001", description: "河北神力工程款", amountCents: 20000000 })],
    total: 2,
    thumbs: { 1: thumb, 2: thumb },
  } as never);
  mockApi.getVoucher.mockResolvedValue({ voucher: v(), attachments: [], snapshots: [] });
  mockApi.updateVoucher.mockImplementation(async (id, u) => v({ id, amountCents: u.amountCents ?? null, description: u.description ?? "", counterparty: u.counterparty ?? "", tags: u.tags ?? [], note: u.note ?? "" }));
  mockApi.listVoucherIds.mockResolvedValue([1, 2]);
  mockApi.batchUpdateStatus.mockResolvedValue(2);
  mockApi.listPeriodLocks.mockResolvedValue([]);
  mockApi.checkVoucherDuplicates.mockResolvedValue([]);
  h.route.value = {};
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("资料库：双行列表与检视器", () => {
  it("默认选中第一行，右栏预填该张字段（金额回填为元）", async () => {
    const w = await mountPage();
    expect(w.find(".inspector").exists()).toBe(true);
    expect(w.find<HTMLInputElement>('[data-field="amount"] input').element.value).toBe("25000");
    expect(mockApi.getVoucher).toHaveBeenCalledWith(1);
  });

  it("改金额并保存 → 调后端（含 clearAmount 语义），列表行同步更新", async () => {
    const w = await mountPage();
    await w.find<HTMLInputElement>('[data-field="amount"] input').setValue("12345.67");
    expect(w.text()).toContain("未保存");
    await w.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(mockApi.updateVoucher).toHaveBeenCalledWith(
      1,
      expect.objectContaining({ amountCents: 1234567, clearAmount: false, description: "润博纺织工程款" }),
    );
    expect(w.text()).not.toContain("未保存");
  });

  it("右栏能改**全部**字段（日期/对方/状态/标签/备注）——原编辑页能力已并入", async () => {
    const w = await mountPage();
    await w.find<HTMLInputElement>('[data-field="date"] input').setValue("2026-09-01");
    await w.find<HTMLInputElement>('[data-field="counterparty"] input').setValue("河北神力");
    await w.find<HTMLSelectElement>('[data-field="status"] select').setValue("reviewed");
    await w.find<HTMLInputElement>('[data-field="tags"] input').setValue("工程款、九月");
    await w.find<HTMLTextAreaElement>('[data-field="note"] textarea').setValue("已核对");
    await w.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(mockApi.updateVoucher).toHaveBeenCalledWith(
      1,
      expect.objectContaining({
        voucherDate: "2026-09-01",
        counterparty: "河北神力",
        status: "reviewed",
        tags: ["工程款", "九月"],
        note: "已核对",
      }),
    );
  });

  it("金额清空 → 提交 clearAmount（显式置空，而不是忽略）", async () => {
    const w = await mountPage();
    await w.find<HTMLInputElement>('[data-field="amount"] input').setValue("");
    await w.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(mockApi.updateVoucher).toHaveBeenCalledWith(1, expect.objectContaining({ amountCents: null, clearAmount: true }));
  });

  it("金额非法（12.34.56）→ 拦下且不调后端", async () => {
    const w = await mountPage();
    await w.find<HTMLInputElement>('[data-field="amount"] input').setValue("12.34.56");
    await w.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(mockApi.updateVoucher).not.toHaveBeenCalled();
  });

  it("双击行 = 展开编辑区（不跳页、不 push 路由）", async () => {
    const w = await mountPage();
    expect(w.find(".split--wide").exists()).toBe(false);
    await w.find(".row").trigger("dblclick");
    await flushPromises();
    expect(w.find(".split--wide").exists()).toBe(true);
    expect(h.push).not.toHaveBeenCalled();
  });

  it("focus 深链：目标不在本页时也能落到右栏，并说明来源", async () => {
    h.route.value = { focus: "9" };
    mockApi.getVoucher.mockResolvedValue({
      voucher: v({ id: 9, voucherNo: "20260901-009", description: "页外凭证" }),
      attachments: [],
      snapshots: [],
    } as never);
    const w = await mountPage();
    expect(mockApi.getVoucher).toHaveBeenCalledWith(9);
    expect(w.find(".inspector").text()).toContain("20260901-009");
    expect(w.text()).toContain("不在当前筛选结果中");
    // 一次性动作：处理完把 focus 从地址栏摘掉
    expect(h.replace).toHaveBeenCalled();
  });
});

describe("资料库：跨页选择集", () => {
  it("「全选筛选结果」走只读 id 接口并显示已选数量", async () => {
    const w = await mountPage();
    await w.findAll("button").find((b) => b.text().includes("全选筛选结果"))!.trigger("click");
    await flushPromises();
    expect(mockApi.listVoucherIds).toHaveBeenCalled();
    expect(w.text()).toContain("已选 2 张");
  });

  it("勾选后**翻页不清空**（旧实现的问题）；显式清除才归零", async () => {
    mockApi.listVouchers.mockResolvedValue({
      items: [v(), v({ id: 2 })],
      total: 60, // 3 页 → 出现翻页按钮
      thumbs: {},
    } as never);
    const w = await mountPage();
    await w.find(".row-check").trigger("click");
    await flushPromises();
    expect(w.text()).toContain("已选 1 张");

    // 翻到下一页（列表重载）——选择集必须保留
    await w.findAll("button").find((b) => b.text() === "下一页")!.trigger("click");
    await flushPromises();
    expect(w.text()).toContain("已选 1 张");

    await w.findAll("button").find((b) => b.text() === "清除")!.trigger("click");
    expect(w.text()).not.toContain("已选");
  });
});
