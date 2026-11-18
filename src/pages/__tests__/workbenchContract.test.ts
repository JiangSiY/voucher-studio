// @vitest-environment node
/**
 * 工作台契约（v0.27 B3）：关键数据 + 图表 + 可下钻待办。
 *
 * 不变量：
 * - 数字来自真实命令，不再写死样例；
 * - 待办只做下钻（页签/弹层），工作台本身不出现新建/导入；
 * - 最近条目点开走 `?focus=`，导出历史走 `?modal=export&tab=history`。
 */
import { describe, expect, it } from "vitest";
import { repoFile } from "../../__tests__/helpers/designGuard";

const src = repoFile("src/pages/WorkbenchPage.vue");

describe("工作台真实数据", () => {
  it("KPI/图表/待办/最近都接真实命令", () => {
    for (const cmd of [
      "dashboardOverview",
      "monthlySummary",
      "topCounterparties",
      "tagStats",
      "dataHealth",
      "listVouchers",
      "listExportBatches",
      "exportCoverage",
    ]) {
      expect(src).toContain(`api.${cmd}`);
    }
    expect(src).not.toContain("const customers = [");
    expect(src).not.toContain("const categories = [");
  });

  it("待办下钻到既有视图或弹层，不复制动作按钮", () => {
    expect(src).toContain("/data?view=missing");
    expect(src).toContain("/data?view=legacy");
    expect(src).toContain("/data?ocr=pending,failed");
    expect(src).toContain("/data?amountEmpty=1");
    expect(src).toContain("/data?stale=7");
    expect(src).toContain("/accounting");
    expect(src).toContain('modal: "duplicates"');
    expect(src).not.toContain("新建条目");
    expect(src).not.toContain("pickAndImport");
  });

  it("最近条目与导出走深链/弹层", () => {
    expect(src).toContain("/data?focus=${r.id}");
    expect(src).toContain("open('export', { tab: 'history' })");
  });
});
