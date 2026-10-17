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

  it("筛选栏吸顶、三个勾选合并为「缺项」下拉、去掉「N 张」、每页 10 条（2026-10-17）", () => {
    const src = readFileSync(new URL("../LibraryPage.vue", import.meta.url), "utf8");
    // ① 吸顶：.filter-bar 声明 sticky（滚动列表/编辑器时筛选不动）
    const bar = src.slice(src.indexOf(".filter-bar {"));
    expect(bar.slice(0, 300)).toContain("position: sticky");
    expect(bar.slice(0, 300)).toContain("top: 0");
    // ② 三复选框 → 一个「缺项」下拉（三个 option 都在，互斥由 pendingFilter 保证）
    expect(src).not.toContain('class="inline-check"');
    expect(src).toContain('<option value="">缺项</option>');
    expect(src).toContain('<option value="amountEmpty">金额未填</option>');
    expect(src).toContain('<option value="counterpartyEmpty">对方未填</option>');
    expect(src).toContain('<option value="unlinked">未被佐证</option>');
    expect(src).toContain('amountEmpty: pendingFilter.value === "amountEmpty"');
    expect(src).toContain('counterpartyEmpty: pendingFilter.value === "counterpartyEmpty"');
    // ③ 去掉筛选栏右侧的「N 张」（选择计数「已选 N 张」保留）
    expect(src).not.toContain('{{ total }} 张');
    // ④ 每页 10 条
    expect(src).toContain("const pageSize = 10;");
  });
});
