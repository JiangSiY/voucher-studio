/**
 * 界面细节打磨第五批的源码判据（node env，直接读文件文本）。
 *
 * 锁三件在渲染里肉眼看不见、但改错了会静默回退的事：
 *   1. 资料库不再有「第二份筛选控件」——原先的折叠行与主行 v-model 同一批 ref，同屏两份；
 *   2. 批量改状态必须过二次确认，且确认文案要说清"选了多少张、其中多少张不在当前列表里"；
 *   3. 两处「选图片」拖放区必须键盘可达（role=button + tabindex + Enter/Space）。
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const read = (rel: string) => readFileSync(join(process.cwd(), rel), "utf8");

describe("资料库筛选与批量操作（打磨第五批）", () => {
  const src = read("src/pages/LibraryPage.vue");

  it("不再有第二份高级筛选行与其死开关", () => {
    expect(src).not.toMatch(/showMore/);
    expect(src).not.toMatch(/advancedCount/);
    expect(src).not.toMatch(/filter-bar advanced/);
    // 那个越权清掉主行筛选的按钮也不该回来（历史注释里提到旧名字是允许的）
    expect(src).not.toMatch(/清除高级筛选/);
  });

  it("列表空态给出可点的「清空全部筛选」，且无筛选时不显示", () => {
    expect(src).toMatch(/const hasAnyFilter = computed\(\(\) => activeFilters\.value\.length > 0\)/);
    expect(src).toMatch(/v-if="hasAnyFilter"[^>]*@click="clearAllFilters\(\)"/);
  });

  it("批量改状态走二次确认，文案含总量与跨页量", () => {
    expect(src).toMatch(/@click="openBatchStatusConfirm\(\)"/);
    expect(src).not.toMatch(/@click="applyBatchStatus\(\)"/);
    expect(src).toMatch(/const selectedOffPage = computed/);
    expect(src).toMatch(/其中 \$\{off\} 张不在当前列表里/);
    expect(src).toMatch(/:visible="confirmBatchStatus"/);
    expect(src).toMatch(/@confirm="applyBatchStatus"/);
  });

  it("批量改状态不是危险操作（默认焦点不落在它上面）", () => {
    const dialog = src.slice(src.indexOf(':visible="confirmBatchStatus"'));
    expect(dialog.slice(0, 400)).toMatch(/:danger="false"/);
  });
});

describe("选图片入口键盘可达（打磨第五批）", () => {
  const cases: [string, string][] = [
    ["src/pages/VoucherFormPage.vue", "createByImage"],
    ["src/components/voucher/AttachmentPanel.vue", "replaceImage"],
  ];

  for (const [file, handler] of cases) {
    it(`${file} 的拖放区是 role=button 且 Enter/Space 可触发`, () => {
      const src = read(file);
      const zone = src.slice(src.indexOf('class="dropzone"'));
      expect(zone.slice(0, 400)).toMatch(/role="button"/);
      expect(zone.slice(0, 400)).toMatch(/tabindex="0"/);
      expect(zone.slice(0, 500)).toMatch(/@keydown\.enter\.prevent=/);
      expect(zone.slice(0, 500)).toMatch(/@keydown\.space\.prevent=/);
      expect(zone.slice(0, 500)).toContain(handler);
    });
  }

  it("筛选栏：吸顶 / 多选下拉 / 去掉导出覆盖与对方精确 / 无「N 张」/ 每页 10 条（2026-10-17）", () => {
    const src = readFileSync(new URL("../LibraryPage.vue", import.meta.url), "utf8");
    // ① 吸顶
    const bar = src.slice(src.indexOf(".filter-bar {"));
    expect(bar.slice(0, 300)).toContain("position: sticky");
    expect(bar.slice(0, 300)).toContain("top: 0");
    // ② 四个筛选都改成可多选下拉
    for (const name of ["状态", "OCR 状态", "缺项", "收支方向"]) {
      expect(src, `缺少多选下拉：${name}`).toContain(`label="${name}"`);
    }
    expect(src).toContain("<MultiSelect");
    expect(src).not.toContain('class="inline-check"');
    // 缺项三选项都在（可组合）
    expect(src).toContain('{ value: "amountEmpty", label: "金额未填" }');
    expect(src).toContain('{ value: "counterpartyEmpty", label: "对方未填" }');
    expect(src).toContain('{ value: "unlinked", label: "未被佐证" }');
    // 状态多选走后端 statuses（IN），OCR 仍是逗号口径
    expect(src).toContain("statuses: statuses.value");
    expect(src).toContain('ocrStatuses.value.join(",")');
    // ③ 去掉「导出覆盖」下拉与「对方账户（精确）」输入框——但 URL 参数仍读（下钻不回退）
    expect(src).not.toContain("导出覆盖度筛选");
    expect(src).not.toContain("对方账户（精确）");
    expect(src).toContain("q.exported as string");
    expect(src).toContain("q.counterparty as string");
    // ④ 无「N 张」；每页 10 条
    expect(src).not.toContain("{{ total }} 张");
    expect(src).toContain("const pageSize = 10;");
  });
});
