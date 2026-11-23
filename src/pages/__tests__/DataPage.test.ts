// @vitest-environment jsdom
/**
 * 数据页「人工勾已入账」的**交互测试**（挂载真实页面 + mock 后端）。
 *
 * 能证什么：B5 退役台账页时丢掉的 `set_ledger_booked` 入口已接回数据页——
 * 点击只发一次命令、成功后只就地改这一行、「待入账」语义不被破坏、
 * 失败保留原状态并给可见提示、行内点击不误触发行展开（`@click.stop`）。
 * 不能证什么：真实写库后的持久化（那需要桌面宿主 + Rust）与视觉细节。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import type { Voucher } from "../../api/client";
import { api } from "../../api/client";
import { useToastStore } from "../../stores/toast";
import DataPage from "../DataPage.vue";

vi.mock("../../api/client", () => ({
  api: {
    listVouchers: vi.fn(),
    getVoucher: vi.fn(),
    listImportBatches: vi.fn(),
    setLedgerBooked: vi.fn(),
  },
  isTauri: false,
}));

// 无桌面宿主：资源地址与路由都 mock 掉
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (p: string) => `asset://localhost/${p}`,
}));
vi.mock("vue-router", () => ({
  useRoute: () => ({ query: {} }),
  useRouter: () => ({ push: vi.fn(), replace: vi.fn() }),
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

/** 一次列表返回：1 行，行级 meta 可指定 booked */
function listOnce(booked = false) {
  mockApi.listVouchers.mockResolvedValue({
    items: [v()],
    total: 1,
    thumbs: {},
    meta: { 1: { booked, batchId: null, attachmentRequired: true } },
  });
}

async function mountPage() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const wrapper = mount(DataPage, {
    global: { plugins: [pinia], stubs: { VoucherEditor: true } },
  });
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  vi.clearAllMocks();
  mockApi.getVoucher.mockResolvedValue(null as never);
  mockApi.listImportBatches.mockResolvedValue([]);
  listOnce();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("人工勾已入账（B5 遗留补收尾）", () => {
  it("点「未入」只发一次 set_ledger_booked，标签变「已入」", async () => {
    mockApi.setLedgerBooked.mockResolvedValue(undefined);
    const wrapper = await mountPage();

    const btn = wrapper.find(".data-booked");
    expect(btn.text()).toBe("未入");
    await btn.trigger("click");
    await flushPromises();

    expect(mockApi.setLedgerBooked).toHaveBeenCalledTimes(1);
    expect(mockApi.setLedgerBooked).toHaveBeenCalledWith(1, true);
    expect(wrapper.find(".data-booked").text()).toBe("已入");
    expect(wrapper.find(".data-booked").classes()).toContain("is-on");
  });

  it("再点一次取消：入参翻回 false，标签变回「未入」", async () => {
    mockApi.setLedgerBooked.mockResolvedValue(undefined);
    const wrapper = await mountPage();

    await wrapper.find(".data-booked").trigger("click");
    await flushPromises();
    await wrapper.find(".data-booked").trigger("click");
    await flushPromises();

    expect(mockApi.setLedgerBooked).toHaveBeenCalledTimes(2);
    expect(mockApi.setLedgerBooked).toHaveBeenLastCalledWith(1, false);
    expect(wrapper.find(".data-booked").text()).toBe("未入");
  });

  it("请求在途时按钮禁用，连点不会重复写库", async () => {
    let resolve: (() => void) | null = null;
    mockApi.setLedgerBooked.mockReturnValue(
      new Promise<void>((r) => {
        resolve = r;
      }),
    );
    const wrapper = await mountPage();

    await wrapper.find(".data-booked").trigger("click");
    await wrapper.find(".data-booked").trigger("click");
    expect(mockApi.setLedgerBooked).toHaveBeenCalledTimes(1);
    expect((wrapper.find(".data-booked").element as HTMLButtonElement).disabled).toBe(true);

    resolve!();
    await flushPromises();
    expect(wrapper.find(".data-booked").text()).toBe("已入");
  });

  it("失败时标签保持「未入」并给可见错误提示", async () => {
    mockApi.setLedgerBooked.mockRejectedValue(new Error("写库失败"));
    const wrapper = await mountPage();
    const toast = useToastStore();

    await wrapper.find(".data-booked").trigger("click");
    await flushPromises();

    expect(wrapper.find(".data-booked").text()).toBe("未入");
    expect(wrapper.find(".data-booked").classes()).not.toContain("is-on");
    expect(toast.items.map((t) => t.text)).toContain("写库失败");
  });

  it("点行内入账控件不展开详情行（@click.stop）", async () => {
    mockApi.setLedgerBooked.mockResolvedValue(undefined);
    const wrapper = await mountPage();

    await wrapper.find(".data-booked").trigger("click");
    await flushPromises();

    expect(wrapper.find(".data-expand").exists()).toBe(false);
    expect(mockApi.getVoucher).not.toHaveBeenCalled();
  });

  it("移动卡片的入账标记也能改（卡片本体不再是 button，避免嵌套控件）", async () => {
    mockApi.setLedgerBooked.mockResolvedValue(undefined);
    const wrapper = await mountPage();

    const cardFlag = wrapper.find(".data-card-flag.is-action");
    expect(cardFlag.text()).toBe("未入账");
    await cardFlag.trigger("click");
    await flushPromises();

    expect(mockApi.setLedgerBooked).toHaveBeenCalledWith(1, true);
    expect(wrapper.find(".data-card-flag.is-action").text()).toBe("已入账");
  });
});
