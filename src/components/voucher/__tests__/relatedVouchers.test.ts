// @vitest-environment jsdom
/**
 * 「相关凭证」建立关联的**完整动线**（用户 2026-10-17 报：「选完明细分类后，没有位置可以关联其他凭证」）。
 *
 * 病因：说明输入框漏了 `rel-note` 类 → 它落在网格第 2 行第 1 列，把 `auto` 列撑到约 400px，
 * 中间的关键词输入框被挤成 0 宽（看不见也点不到）→ 用户以为"没有地方可以关联"。
 * 这里用真实组件把动线锁住：展开 → 输入关键词 → 搜索 → 点候选 → 调 linkVouchers。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";

vi.mock("../../../api/client", () => ({
  api: {
    listVoucherLinks: vi.fn(),
    listVouchers: vi.fn(),
    linkVouchers: vi.fn(),
    unlinkVouchers: vi.fn(),
  },
}));

import RelatedVouchers from "../RelatedVouchers.vue";
import { api } from "../../../api/client";

const candidate = {
  id: 2,
  voucherNo: "20260901-002",
  voucherDate: "2026-09-01",
  description: "发票-办公用品",
  counterparty: "文一办公用品店",
  amountCents: 12345,
  status: "reviewed",
  tags: [],
  note: "",
} as never;

beforeEach(() => {
  vi.mocked(api.listVoucherLinks).mockResolvedValue([] as never);
  vi.mocked(api.listVouchers).mockResolvedValue({ items: [candidate], total: 1, thumbs: [] } as never);
  vi.mocked(api.linkVouchers).mockResolvedValue(undefined as never);
  vi.mocked(api.unlinkVouchers).mockResolvedValue(undefined as never);
});

async function openPicker() {
  const w = mount(RelatedVouchers, { props: { voucherId: 1 } });
  await w.find(".rel-head button").trigger("click");
  return w;
}

describe("相关凭证：建立关联的动线", () => {
  it("展开后四件控件都在，且「说明」带 rel-note（独占整行，不再挤掉关键词输入框）", async () => {
    const w = await openPicker();
    const pick = w.find(".rel-pick");
    expect(pick.exists()).toBe(true);
    expect(pick.findAll(".input").length).toBe(3); // 类型 select（也带 .input 类）+ 关键词 + 说明
    expect(pick.find("select.rel-kind").exists()).toBe(true);
    expect(pick.find("input[aria-label='搜索凭证']").exists()).toBe(true);
    expect(pick.find("button.btn").text()).toContain("搜索");
    // 关键：「说明」必须带 rel-note，CSS 里 `.rel-note { grid-column: 1 / -1 }` 才生效
    const note = pick.find("input[aria-label='关系说明']");
    expect(note.exists()).toBe(true);
    expect(note.classes()).toContain("rel-note");
  });

  it("输入关键词 → 搜索 → 点候选 → 调 linkVouchers（kind / note 原样带上）", async () => {
    const w = await openPicker();
    await w.find("input[aria-label='搜索凭证']").setValue("发票");
    await w.find("input[aria-label='关系说明']").setValue("这笔付款对应的发票");
    await w.find("select.rel-kind").setValue("detail");
    await w.find(".rel-pick button.btn").trigger("click");
    await w.vm.$nextTick();

    expect(api.listVouchers).toHaveBeenCalledWith(
      expect.objectContaining({ keyword: "发票", page: 1, pageSize: 8 }),
    );
    const cand = w.find(".rel-cand button.link");
    expect(cand.exists()).toBe(true);
    expect(cand.text()).toContain("20260901-002");

    await cand.trigger("click");
    expect(api.linkVouchers).toHaveBeenCalledWith(1, 2, "detail", "这笔付款对应的发票");
  });

  it("搜不到时给明确提示（而不是一片空白让人以为坏了）", async () => {
    vi.mocked(api.listVouchers).mockResolvedValue({ items: [], total: 0, thumbs: [] } as never);
    const w = await openPicker();
    await w.find("input[aria-label='搜索凭证']").setValue("不存在的关键词");
    await w.find(".rel-pick button.btn").trigger("click");
    await w.vm.$nextTick();
    expect(w.text()).toContain("没有找到匹配的凭证");
  });
});
