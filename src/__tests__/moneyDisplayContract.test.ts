// @vitest-environment node
/**
 * 金额展示口径守卫：界面上的金额只走 `composables/format.ts` 的 `fmtAmount`
 * （千分位 + 固定两位小数）。
 *
 * 背景（2026-10-16 审查）：同一应用里金额有三套写法——`toFixed(2)`（无千分位）、
 * `toLocaleString("zh-CN", {minimumFractionDigits: 2})`（有千分位，与前者不一致）、
 * `fmtAmount`。列表与卡片上同一笔 12345.60 元会显示成 `12345.60` 与 `12,345.60` 两种样子。
 * 表单**输入框**的填充值（纯数字串）不属于展示口径，允许继续用分转元的纯数字写法。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function src(rel: string): string {
  return readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");
}

describe("金额展示统一走 fmtAmount", () => {
  it("打印中心批次表：金额列用 fmtAmount", () => {
    const page = src("src/pages/PrintCenterPage.vue");
    expect(page).toContain("fmtAmount(v.amountCents)");
    expect(page).not.toContain("(v.amountCents / 100).toFixed(2)");
  });

  it("OCR 结果卡片：金额与 ¥ 前缀由 fmtAmount 提供", () => {
    const card = src("src/components/OcrResultCard.vue");
    expect(card).toContain("fmtAmount");
    expect(card).not.toContain("(extracted.amountCents / 100).toFixed(2)");
  });

  it("凭证卡片：不再自带一份 toLocaleString 实现（空金额仍留白）", () => {
    const card = src("src/components/VoucherCard.vue");
    expect(card).toContain("fmtAmount(props.voucher.amountCents)");
    expect(card).not.toContain('toLocaleString("zh-CN"');
  });

  it("编辑器疑似重复提示：金额用 fmtAmount（不再把 null 当 0.00）", () => {
    const editor = src("src/components/voucher/VoucherEditor.vue");
    expect(editor).toContain("fmtAmount(v.amountCents)");
    expect(editor).not.toContain("(v.amountCents ?? 0) / 100");
  });
});
