// @vitest-environment node
/**
 * 编辑器表单布局契约（2026-10-17 **改写**：随「方案 A」从"一条标签列"改为"标签在输入框上方"）。
 *
 * 病史（保留，避免重蹈）：2026-09-28 用户截图报"输入框左边缘参差不齐"——当时标签列宽度被写在**每一行**上，
 * 每行各算各的宽度（"状态"两字窄、"金额（元）"五字宽）→ 输入框左边缘对不齐。当时的解法是
 * "整个表单一条网格 + 行壳 display:contents"，并由本文件锁住。
 *
 * 2026-10-17 用户选定**方案 A**：字段标签移到**输入框上方**、取消两组并排（日期/金额、对方账户/收支方向）。
 * 于是"左边缘不齐"这一整类 bug 从结构上不可能再发生（所有输入框同起点、同宽），
 * 老契约随之**退休**，换成本文这三条新契约。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const src = readFileSync(new URL("../VoucherEditor.vue", import.meta.url), "utf8");

describe("凭证编辑器：标签在输入框上方（方案 A 契约）", () => {
  it("表单是单列堆叠，且**不再**有左侧标签列（不得出现 max-content 标签列）", () => {
    expect(src).toMatch(/\.editor-form\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)/);
    expect(src).not.toContain("max-content");
    // 旧的成对行实现已退休
    expect(src).not.toContain("ed-pair");
    expect(src).not.toContain('class="ed-row"');
  });

  it("每个字段都是 .ed-field：标签行在上、控件行在下（纵排，间距 4px）", () => {
    expect(src).toMatch(/\.ed-field\s*\{[^}]*display:\s*grid[^}]*gap:\s*4px/);
    expect(src).toMatch(/\.ed-field-label\s*\{[^}]*font:\s*var\(--type-caption\)/);
    // 模板里每个字段都先给标签、再给控件（7 个字段：摘要/日期/金额/状态/对方/收支方向/备注）
    const fields = [
      ...src.matchAll(
        /class="ed-field"[^>]*data-field="([a-z]+)"[\s\S]{0,400}?class="ed-field-label"/g,
      ),
    ].map((m) => m[1]);
    expect(fields).toEqual([
      "description",
      "date",
      "amount",
      "status",
      "counterparty",
      "tags",
      "note",
    ]);
  });

  it("摘要行仍隐藏子组件自带标签（否则「摘要」出现两遍）", () => {
    expect(src).toMatch(/\.desc-no-label\s*:deep\(label\)\s*\{\s*display:\s*none/);
  });
});
