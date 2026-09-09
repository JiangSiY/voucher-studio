// @vitest-environment jsdom
/**
 * VoucherCard 组件测试（v0.14-D 组件测试补至 3 场景，体检 #16）。
 *
 * 覆盖：三种打印风格（standard/clean/ledger）的渲染分支、金额格式化
 * （¥ 千分位 / amountCents 为 null 时不显示金额）、摘要/备注兜底文案、
 * 自定义标题。纯展示组件，无外部依赖，可真实 mount。
 */
import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import VoucherCard from "../VoucherCard.vue";

function fixture(overrides: Partial<{
  voucherNo: string;
  voucherDate: string;
  description: string;
  amountCents: number | null;
  tags: string[];
  note: string;
}> = {}) {
  return {
    voucherNo: "记-2026-0009",
    voucherDate: "2026-09-09",
    description: "差旅费报销",
    amountCents: 123456,
    tags: ["差旅", "报销"],
    note: "附发票 3 张",
    ...overrides,
  };
}

function mountCard(overrides: {
  voucher?: ReturnType<typeof fixture>;
  style?: "standard" | "clean" | "ledger";
  title?: string;
} = {}) {
  return mount(VoucherCard, {
    props: {
      voucher: overrides.voucher ?? fixture(),
      style: overrides.style,
      title: overrides.title,
    },
  });
}

describe("VoucherCard", () => {
  it("standard：默认标题/凭证号/日期与金额千分位格式化", () => {
    const w = mountCard();
    expect(w.find(".vcard.accent").exists()).toBe(true);
    expect(w.find(".vh").text()).toBe("凭证整理单");
    expect(w.find(".vcode").text()).toBe("记-2026-0009");
    expect(w.find(".vdur").text()).toContain("2026-09-09");
    expect(w.find(".vdur").text()).toContain("¥1,234.56");
  });

  it("standard：标签按 · 拼接、备注展示，摘要为空时显示兜底文案", () => {
    const w = mountCard({ voucher: fixture({ description: "", tags: ["差旅"], note: "" }) });
    expect(w.find(".vexp").text()).toBe("（无摘要）");
    // tags.length>0 → tags 行存在且追加 <br>
    expect(w.find(".vnote").text()).toContain("差旅");
    expect(w.find(".vnote").text()).toContain("记账原始凭证 · 归档留存");
  });

  it("amountCents 为 null 时不渲染金额片段", () => {
    const w = mountCard({ voucher: fixture({ amountCents: null }) });
    expect(w.find(".vdur").text()).not.toContain("¥");
    // ledger 风格下金额行（第 2 行）显示占位「—」
    const wl = mountCard({ style: "ledger", voucher: fixture({ amountCents: null }) });
    const rows = wl.findAll(".vll-row");
    expect(rows).toHaveLength(3);
    expect(rows[1].text()).toContain("金额");
    expect(rows[1].text()).toContain("—");
  });

  it("clean：渲染左色条风格与摘要行", () => {
    const w = mountCard({ style: "clean", title: "打印单" });
    expect(w.find(".vcard-clean").exists()).toBe(true);
    expect(w.find(".vcl-title").text()).toBe("打印单");
    expect(w.find(".vcl-code").text()).toBe("记-2026-0009");
    expect(w.find(".vcl-meta").text()).toContain("差旅费报销");
  });

  it("ledger：账簿式展示三行字段（摘要/金额/标签）", () => {
    const w = mountCard({ style: "ledger" });
    expect(w.find(".vcard-ledger").exists()).toBe(true);
    expect(w.find(".vll-code").text()).toBe("记-2026-0009");
    const rows = w.findAll(".vll-row");
    expect(rows).toHaveLength(3);
    expect(rows[0].text()).toContain("差旅费报销");
    expect(rows[1].text()).toContain("¥1,234.56");
    expect(rows[2].text()).toContain("差旅、报销");
  });

  it("ledger：无标签时标签行回退为 —", () => {
    const w = mountCard({ style: "ledger", voucher: fixture({ tags: [], description: "" }) });
    const rows = w.findAll(".vll-row");
    expect(rows[0].text()).toContain("—");
    expect(rows[2].text()).toContain("—");
  });
});
