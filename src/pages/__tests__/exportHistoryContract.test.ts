// @vitest-environment node
/**
 * 导出历史的「格式可见」守卫。
 *
 * 背景（2026-10-16 审查）：批次格式后端早已写库（`crates/voucher-core/src/services/export.rs`：
 * EXP-X=Excel / EXP-C=CSV，文件路径沿用 `pdf_path` 列），但界面既没有格式列、
 * 操作按钮也一律写「打开 PDF」——Excel/CSV 批次点开才发现不是 PDF。
 * 同一张表上，批次状态（成功/失败）与凭证状态（已导出/已归档）是两套体系，文案须区分。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const page = readFileSync(new URL("../ExportHistoryPage.vue", import.meta.url), "utf8");

describe("导出历史：格式列与可追溯性", () => {
  it("批次列表有「格式」列，取后端 format 字段，三种格式都有标签", () => {
    expect(page).toContain("<th>格式</th>");
    expect(page).toMatch(/FORMAT_LABEL[^=]*=\s*\{\s*pdf:\s*"PDF",\s*xlsx:\s*"Excel",\s*csv:\s*"CSV"\s*\}/);
    expect(page).toContain("formatLabel(b)");
  });

  it("操作按钮的 title 随格式变化（不再一律写「打开 PDF」）", () => {
    expect(page).toMatch(/:title="`打开 \$\{formatLabel\(b\)\} 文件`"/);
    expect(page).not.toContain('title="打开 PDF"');
  });

  it("批次状态用「已完成 / 失败」，与凭证状态词区分", () => {
    expect(page).toContain('b.status === "done" ? "已完成" : "失败"');
  });

  it("过时文案已清：「点击进入编辑页」不存在（v0.19 起编辑在资料库右栏）", () => {
    expect(page).not.toContain("点击进入编辑页");
    expect(page).toContain('title="在资料库中打开"');
  });
});
