// @vitest-environment node
/**
 * 对账契约（v0.27 B4）：唯一入口，只读比对，六类差异。
 */
import { describe, expect, it } from "vitest";
import { repoFile } from "../../__tests__/helpers/designGuard";

const src = repoFile("src/pages/ReconcilePage.vue");

describe("台账比对", () => {
  it("接 ledger_check / ledger_check_export，不再用旧银行对账命令", () => {
    expect(src).toContain("api.pickLedgerCsv");
    expect(src).toContain("api.ledgerCheck");
    expect(src).toContain("api.ledgerCheckExport");
    expect(src).not.toContain("pickAndReconcile");
  });

  it("六类差异都有中文标签", () => {
    for (const label of [
      "缺凭证",
      "编号缺失",
      "编号重复",
      "金额不符",
      "日期不符",
      "对方不符",
      "多余凭证",
    ]) {
      expect(src).toContain(label);
    }
  });

  it("差异可导出，空结果有说明", () => {
    expect(src).toContain("导出差异");
    expect(src).toContain("两边完全对得上");
  });
});
