// @vitest-environment jsdom
/**
 * 重复核查页的**交互测试**（挂载真实组件 + mock 后端）。
 *
 * 能证什么：页面按契约调用命令、动作按钮到命令的接线正确、危险动作必须过二次确认、
 * 处置后队列确实重取（用户报的"处置完还挂着"不能再出现）、失败走错误提示不静默。
 * 不能证什么：视觉呈现（缩略图是否真的画出来、旁列对比的像素布局）——那需要浏览器/真机截图。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import type { ReviewGroup, Voucher } from "../../api/client";
import { api } from "../../api/client";
import { useToastStore } from "../../stores/toast";
import DuplicateReviewPage from "../DuplicateReviewPage.vue";

vi.mock("../../api/client", () => ({
  api: {
    scanDuplicateReview: vi.fn(),
    duplicatePendingSummary: vi.fn(),
    listDuplicateDecisions: vi.fn(),
    resolveDuplicatePair: vi.fn(),
    removeDuplicateVoucher: vi.fn(),
    getVoucher: vi.fn(),
  },
}));

// 无桌面宿主：asset 转换与事件监听都 mock 掉
vi.mock("@tauri-apps/api/core", () => ({ convertFileSrc: (p: string) => `asset://localhost/${p}` }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("vue-router", () => ({
  useRoute: () => ({ query: {} }),
  useRouter: () => ({ push: vi.fn() }),
}));

const mockApi = vi.mocked(api);

function v(over: Partial<Voucher> = {}): Voucher {
  return {
    id: 1,
    voucherNo: "记-2026-08-020",
    voucherDate: "2026-08-05",
    description: "租金",
    counterparty: "甲物业",
    amountCents: 123_456,
    status: "draft",
    tags: [],
    note: "",
    createdAt: "2026-08-05T10:00:00",
    updatedAt: "2026-08-05T10:00:00",
    ...over,
  } as Voucher;
}

const group: ReviewGroup = {
  voucherDate: "2026-08-05",
  amountCents: 123_456,
  vouchers: [v(), v({ id: 2, voucherNo: "记-2026-08-021", description: "水电" })],
  pending: [
    {
      a: v(),
      b: v({ id: 2, voucherNo: "记-2026-08-021", description: "水电" }),
      evidence: { sameImage: true, sameCounterparty: false, sameDescription: false, level: "high" },
    },
  ],
  decidedPairs: 0,
};

async function mountPage(): Promise<VueWrapper> {
  const w = mount(DuplicateReviewPage, { attachTo: document.body });
  await flushPromises();
  return w;
}

beforeEach(() => {
  setActivePinia(createPinia());
  mockApi.scanDuplicateReview.mockResolvedValue([structuredClone(group)]);
  mockApi.duplicatePendingSummary.mockResolvedValue({ groups: 1, pairs: 1 });
  mockApi.listDuplicateDecisions.mockResolvedValue([]);
  mockApi.getVoucher.mockResolvedValue({
    voucher: v(),
    attachments: [
      {
        id: 9,
        voucherId: 1,
        fileName: "a.jpg",
        filePath: "2026-08/a.jpg",
        fileHash: "h1",
        fileSize: 10,
        mimeType: "image/jpeg",
        ocrStatus: "done",
        ocrText: null,
        enhancedPath: null,
        fileAbs: "/tmp/att/2026-08/a.jpg",
        enhancedAbs: null,
        enhancedSource: "unknown",
        createdAt: "2026-08-05T10:00:00",
      },
    ],
    snapshots: [],
  });
  mockApi.resolveDuplicatePair.mockResolvedValue(1);
  mockApi.removeDuplicateVoucher.mockResolvedValue(1);
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("待处理队列", () => {
  it("按组渲染：组头含日期/金额/张数，对行给出置信度与「像在哪」", async () => {
    const w = await mountPage();
    const text = w.text();
    expect(text).toContain("2026-08-05");
    expect(text).toContain("1,234.56");
    expect(text).toContain("× 2 张");
    expect(text).toContain("置信度 高");
    expect(text).toContain("图片内容完全相同");
    expect(mockApi.scanDuplicateReview).toHaveBeenCalledTimes(1);
  });

  it("没有待核查项时给出空状态（而不是空白面板）", async () => {
    mockApi.scanDuplicateReview.mockResolvedValue([]);
    const w = await mountPage();
    expect(w.text()).toContain("没有待核查的疑似重复");
  });

  it("「稍后」只把该对沉到组尾（不写库、不调后端）", async () => {
    const w = await mountPage();
    const later = w.findAll("button").find((b) => b.text().includes("稍后"));
    expect(later).toBeTruthy();
    await later!.trigger("click");
    await flushPromises();
    expect(mockApi.resolveDuplicatePair).not.toHaveBeenCalled();
    expect(mockApi.removeDuplicateVoucher).not.toHaveBeenCalled();
    expect(w.text()).toContain("已稍后");
  });
});

describe("旁列对比与两个动作", () => {
  it("点「核对」拉两张详情并显示字段对照（含差异计数）", async () => {
    const w = await mountPage();
    const btn = w.findAll("button").find((b) => b.text().includes("核对"))!;
    await btn.trigger("click");
    await flushPromises();
    expect(mockApi.getVoucher).toHaveBeenCalledWith(1);
    expect(mockApi.getVoucher).toHaveBeenCalledWith(2);
    const text = w.text();
    expect(text).toContain("核对这一对");
    expect(text).toContain("凭证编号");
    expect(text).toMatch(/\d+ 项字段中 \d+ 项不同/);
  });

  it("「不是重复」带上备注调用后端，并重取队列（处置完不该还挂着）", async () => {
    const w = await mountPage();
    await w.findAll("button").find((b) => b.text().includes("核对"))!.trigger("click");
    await flushPromises();
    await w.find(".dup-actions input").setValue("同一天两笔正常付款");
    const btn = w.findAll("button").find((b) => b.text().includes("不是重复"))!;
    await btn.trigger("click");
    await flushPromises();
    expect(mockApi.resolveDuplicatePair).toHaveBeenCalledWith(1, 2, "同一天两笔正常付款");
    expect(mockApi.scanDuplicateReview).toHaveBeenCalledTimes(2);
  });

  it("「保留左 · 删除右」必须先过二次确认，确认文案要说清不可恢复与留痕", async () => {
    const w = await mountPage();
    await w.findAll("button").find((b) => b.text().includes("核对"))!.trigger("click");
    await flushPromises();
    const delBtn = w.findAll("button").find((b) => b.text().includes("保留左"))!;
    await delBtn.trigger("click");
    await flushPromises();
    // 未确认前不得调用删除
    expect(mockApi.removeDuplicateVoucher).not.toHaveBeenCalled();
    const body = document.body.textContent ?? "";
    expect(body).toContain("不可恢复");
    expect(body).toContain("留痕");
    // 确认后才真的删（保留 1、删除 2）
    await w.findComponent({ name: "ConfirmDialog" }).vm.$emit("confirm");
    await flushPromises();
    expect(mockApi.removeDuplicateVoucher).toHaveBeenCalledWith(1, 2, "");
  });

  it("删除失败走错误提示（不静默吞掉）", async () => {
    mockApi.removeDuplicateVoucher.mockRejectedValue(new Error("已封账：2026-08 不可修改"));
    const w = await mountPage();
    await w.findAll("button").find((b) => b.text().includes("核对"))!.trigger("click");
    await flushPromises();
    await w.findAll("button").find((b) => b.text().includes("保留左"))!.trigger("click");
    await flushPromises();
    await w.findComponent({ name: "ConfirmDialog" }).vm.$emit("confirm");
    await flushPromises();
    const items = useToastStore().items;
    expect(items.some((t) => t.kind === "danger" && t.text.includes("封账"))).toBe(true);
  });
});

describe("已处理页签", () => {
  it("按需拉取处置历史，并显示结论与谁留谁删", async () => {
    mockApi.listDuplicateDecisions.mockResolvedValue([
      {
        id: 7,
        aId: 1,
        bId: 2,
        decision: "duplicate_removed",
        keepId: 1,
        removedId: 2,
        removedPayload: { voucher: v({ id: 2, voucherNo: "记-2026-08-021" }), keptVoucherNo: "记-2026-08-020" },
        note: "重复上传",
        createdAt: "2026-08-06T09:00:00",
      },
    ]);
    const w = await mountPage();
    const tab = w.findAll("button").find((b) => b.text().includes("已处理"))!;
    await tab.trigger("click");
    await flushPromises();
    expect(mockApi.listDuplicateDecisions).toHaveBeenCalled();
    const text = w.text();
    expect(text).toContain("已删除重复");
    expect(text).toContain("保留 记-2026-08-020 · 删除 记-2026-08-021");
    expect(text).toContain("重复上传");
  });
});
