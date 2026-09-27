// @vitest-environment jsdom
/**
 * 资料库（双行列表 + 检视器）的交互契约（用户 2026-09-27 选定后新增的能力）：
 * - 选中行 → 检视器显示该张，并**预填可就地编辑的字段**；
 * - 就地保存：只改常用字段，金额走统一校验（非法值直接拦下、不调后端）；
 * - **跨页选择集**：翻页不丢选择（旧实现每次 load 清空，是用户抱怨过的行为）；
 * - 「按筛选全选 N 张」走只读 id 接口，不把 N 条对象拉回前端。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import type { Voucher } from "../../api/client";
import { api } from "../../api/client";
import LibraryPage from "../LibraryPage.vue";

const push = vi.fn();

vi.mock("../../api/client", () => ({
  api: {
    listVouchers: vi.fn(),
    getVoucher: vi.fn(),
    updateVoucher: vi.fn(),
    batchUpdateStatus: vi.fn(),
    deleteVouchers: vi.fn(),
    listVoucherIds: vi.fn(),
  },
}));
vi.mock("@tauri-apps/api/core", () => ({ convertFileSrc: (p: string) => `asset://${p}` }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("vue-router", () => ({
  useRoute: () => ({ query: {} }),
  useRouter: () => ({ push }),
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
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("资料库：双行列表与检视器", () => {
  it("默认选中第一行，检视器预填该张的可编辑字段（金额回填为元）", async () => {
    const w = await mountPage();
    expect(w.find(".inspector").exists()).toBe(true);
    const amount = w.find<HTMLInputElement>('.ins-edit .ins-row input');
    expect(amount.element.value).toBe("25000");
    expect(mockApi.getVoucher).toHaveBeenCalledWith(1);
  });

  it("改金额并保存 → 调后端（含 clearAmount 语义），列表行同步更新", async () => {
    const w = await mountPage();
    await w.find<HTMLInputElement>(".ins-edit .ins-row input").setValue("12345.67");
    expect(w.text()).toContain("有未保存的修改");
    await w.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(mockApi.updateVoucher).toHaveBeenCalledWith(
      1,
      expect.objectContaining({ amountCents: 1234567, clearAmount: false, description: "润博纺织工程款" }),
    );
    // 保存后列表行金额随之更新，且不再显示"未保存"
    expect(w.text()).not.toContain("有未保存的修改");
  });

  it("金额清空 → 提交 clearAmount（显式置空，而不是忽略）", async () => {
    const w = await mountPage();
    await w.find<HTMLInputElement>(".ins-edit .ins-row input").setValue("");
    await w.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(mockApi.updateVoucher).toHaveBeenCalledWith(1, expect.objectContaining({ amountCents: null, clearAmount: true }));
  });

  it("金额非法（12.34.56）→ 拦下且不调后端", async () => {
    const w = await mountPage();
    await w.find<HTMLInputElement>(".ins-edit .ins-row input").setValue("12.34.56");
    await w.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(mockApi.updateVoucher).not.toHaveBeenCalled();
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
