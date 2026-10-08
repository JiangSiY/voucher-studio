// @vitest-environment node
/**
 * 编辑器布局契约（2026-09-28 用户截图报"编辑框未对齐"后补的守卫）。
 *
 * 病灶回放：标签列宽度曾写在**每一行**上（`.ed-row { grid-template-columns: max-content … }`），
 * 于是每行各算各的宽度——"状态"两字窄、"金额（元）"五字宽 → 各行的输入框左边缘参差不齐。
 * 正确做法是**整个表单一条网格**：`.editor-form` 定义两列、行壳 `display: contents`
 * 让标签与控件直接落进同一条标签列。
 *
 * 布局在 jsdom 里测不出来（没有排版引擎），故把这条结构约定固化成源码断言——
 * 谁把 `max-content` 挪回行上，这里立刻变红。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const src = readFileSync(new URL("../VoucherEditor.vue", import.meta.url), "utf8");

describe("凭证编辑器：表单对齐（一条标签列）", () => {
  it("表单容器声明两列网格，标签列按内容自适应（字号可调，不用固定 px）", () => {
    expect(src).toMatch(
      /\.editor-form\s*\{[^}]*grid-template-columns:\s*max-content\s+minmax\(0,\s*1fr\)/,
    );
  });

  it("行壳 display: contents（标签与控件落进同一条列），且行上**不得**再定义列宽", () => {
    expect(src).toMatch(/\.ed-row\s*\{\s*display:\s*contents;?\s*\}/);
    const rowRules = [...src.matchAll(/\.ed-row[^{]*\{[^}]*\}/g)].map((m) => m[0]);
    for (const rule of rowRules) {
      expect(rule, `行上不应再写 grid-template-columns：${rule}`).not.toContain("grid-template-columns");
    }
  });

  it("摘要行隐藏子组件自带标签（否则「摘要」出现两遍）", () => {
    expect(src).toMatch(/\.desc-no-label\s*:deep\(label\)\s*\{\s*display:\s*none/);
  });
});
