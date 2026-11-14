// @vitest-environment jsdom
/**
 * 台账页（v13）的**交互测试**（挂载真实组件 + mock 后端）。
 *
 * 能证什么：三段列表按契约取数、主操作到命令的接线正确、撤销必须过二次确认、
 * 撤销后确实重取（不然用户会看到已删的凭证还挂在表里）、勾「已入账」后就从待入账消失。
 * 不能证什么：视觉呈现（表格列宽、窄窗表现）——需要真机截图。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import type { ImportBatch, LedgerCounts, LedgerItem } from "../../api/client";
import { api } from "../../api/client";
import LedgerPage from "../LedgerPage.vue";

vi.mock("../../api/client", () => ({
  api: {
    ledgerCounts: vi.fn(),
    listLedgerItems: vi.fn(),
    listImportBatches: vi.fn(),
    pickAndAttachImage: vi.fn(),
    setLedgerBooked: vi.fn(),
    undoImportBatch: vi.fn(),
    pickLedgerCsv: vi.fn(),
    importLedgerCsv: vi.fn(),
  },
}));

vi.mock("vue-router", () => ({
  useRoute: () => ({ query: {} }),
  useRouter: () => ({ push: vi.fn() }),
}));

const mockApi = vi.mocked(api);

function item(over: Partial<LedgerItem> = {}): LedgerItem {
  return {
    voucherId: 1,
    voucherNo: "2026-10-001",
    voucherDate: "2026-10-01",
    ledgerRef: "SK-1",
    ledgerKind: "shoukuan",
    amountCents: 123_456,
    counterparty: "宋大江",
    description: "宋大江-凯诗兰德-SK-1",
    hasAttachment: false,
    booked: false,
    importBatchId: 7,
    ...over,
  };
}

function batch(over: Partial<ImportBatch> = {}): ImportBatch {
  return {
    id: 7,
    createdAt: "2026-10-31T09:00:00",
    kind: "shoukuan",
    fileName: "收款清单.csv",
    filePath: "/tmp/收款清单.csv",
    rowsTotal: 3,
    created: 3,
    matched: 0,
    skipped: 0,
    errorRows: 0,
    undoneAt: null,
    ...over,
  };
}

const counts: LedgerCounts = { unattached: 2, unbooked: 1, activeBatches: 1 };

async function mountPage(): Promise<VueWrapper> {
  const w = mount(LedgerPage, { attachTo: document.body });
  await flushPromises();
  return w;
}

function btn(w: VueWrapper, label: string) {
  return w.findAll("button").find((b) => b.text().trim() === label);
}

beforeEach(() => {
  setActivePinia(createPinia());
  mockApi.ledgerCounts.mockResolvedValue({ ...counts });
  mockApi.listLedgerItems.mockImplementation(async (stage) =>
    stage === "unattached"
      ? [item(), item({ voucherId: 2, voucherNo: "2026-10-002", ledgerRef: "SK-2" })]
      : [item({ voucherId: 3, voucherNo: "2026-10-003", ledgerRef: "JD-1", hasAttachment: true })],
  );
  mockApi.listImportBatches.mockResolvedValue([batch()]);
  // 挂附件走的也是 import_files：返回一个最小 ImportSummary（页面只看 created/skipped）
  mockApi.pickAndAttachImage.mockResolvedValue({
    vouchers: [],
    skippedDuplicates: [],
    failed: [],
    created: 1,
    matched: 0,
    skipped: 0,
    errors: [],
  });
  mockApi.setLedgerBooked.mockResolvedValue(undefined);
  mockApi.undoImportBatch.mockResolvedValue({ batchId: 7, removed: 2, kept: [] });
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("台账页：三段列表", () => {
  it("首屏是「待补附件」：概览数字与表格同源，主操作是导入清单", async () => {
    const w = await mountPage();
    expect(mockApi.ledgerCounts).toHaveBeenCalled();
    expect(mockApi.listLedgerItems).toHaveBeenCalledWith("unattached");

    const text = w.text();
    expect(text).toContain("待补附件");
    expect(text).toContain("待入账");
    expect(text).toContain("SK-1");
    expect(text).toContain("1,234.56");
    expect(btn(w, "导入清单")).toBeTruthy();
  });

  it("切到「待入账」按 stage 重新取数（不是本地过滤）", async () => {
    const w = await mountPage();
    await w
      .findAll(".seg-item")
      .find((b) => b.text().includes("待入账"))!
      .trigger("click");
    await flushPromises();
    expect(mockApi.listLedgerItems).toHaveBeenLastCalledWith("unbooked");
    expect(w.text()).toContain("JD-1");
  });

  it("切到「批次」取批次列表，不再取条目", async () => {
    const w = await mountPage();
    mockApi.listLedgerItems.mockClear();
    await w
      .findAll(".seg-item")
      .find((b) => b.text().includes("批次"))!
      .trigger("click");
    await flushPromises();
    expect(mockApi.listImportBatches).toHaveBeenCalled();
    expect(mockApi.listLedgerItems).not.toHaveBeenCalled();
    expect(w.text()).toContain("收款清单.csv");
  });

  it("空列表给空状态与下一步动作（不是空白面板）", async () => {
    mockApi.listLedgerItems.mockResolvedValue([]);
    const w = await mountPage();
    expect(w.text()).toContain("没有待补附件的凭证");
  });
});

describe("待补附件：挂附件", () => {
  it("点「挂附件」对该凭证调 pick_and_attach_image 并重取列表", async () => {
    const w = await mountPage();
    mockApi.listLedgerItems.mockClear();
    await btn(w, "挂附件")!.trigger("click");
    await flushPromises();
    expect(mockApi.pickAndAttachImage).toHaveBeenCalledWith(1, true);
    expect(mockApi.listLedgerItems).toHaveBeenCalled();
  });

  it("用户取消对话框（返回 null）不提示、不重取", async () => {
    mockApi.pickAndAttachImage.mockResolvedValue(null);
    const w = await mountPage();
    mockApi.listLedgerItems.mockClear();
    await btn(w, "挂附件")!.trigger("click");
    await flushPromises();
    expect(mockApi.listLedgerItems).not.toHaveBeenCalled();
  });
});

describe("待入账：勾「已入账」", () => {
  it("勾选后调 set_ledger_booked 并重取（该行随即从待入账消失）", async () => {
    const w = await mountPage();
    await w
      .findAll(".seg-item")
      .find((b) => b.text().includes("待入账"))!
      .trigger("click");
    await flushPromises();
    await btn(w, "已入账")!.trigger("click");
    await flushPromises();
    expect(mockApi.setLedgerBooked).toHaveBeenCalledWith(3, true);
  });
});

describe("批次：撤销", () => {
  it("「撤销」先弹二次确认，确认前不调后端", async () => {
    const w = await mountPage();
    await w
      .findAll(".seg-item")
      .find((b) => b.text().includes("批次"))!
      .trigger("click");
    await flushPromises();

    await btn(w, "撤销")!.trigger("click");
    await flushPromises();
    expect(mockApi.undoImportBatch).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("撤销这批导入？");
  });

  it("确认后调 undo_import_batch，并把保留的凭证与原因摊给用户看", async () => {
    mockApi.undoImportBatch.mockResolvedValue({
      batchId: 7,
      removed: 1,
      kept: [{ voucherNo: "2026-10-009", ledgerRef: "SK-9", reason: "有附件" }],
    });
    const w = await mountPage();
    await w
      .findAll(".seg-item")
      .find((b) => b.text().includes("批次"))!
      .trigger("click");
    await flushPromises();
    await btn(w, "撤销")!.trigger("click");
    await flushPromises();

    // ConfirmDialog 走 Teleport 到 body，不在组件子树里，得从 document 找
    const confirm = [...document.body.querySelectorAll("button")].find(
      (b) => b.textContent?.includes("撤销这批") && b.className.includes("btn-danger"),
    );
    confirm!.click();
    await flushPromises();

    expect(mockApi.undoImportBatch).toHaveBeenCalledWith(7);
    expect(document.body.textContent).toContain("SK-9");
    expect(document.body.textContent).toContain("有附件");
  });

  it("本批一张都没建成（created=0）时不给撤销按钮", async () => {
    mockApi.listImportBatches.mockResolvedValue([batch({ created: 0, matched: 3 })]);
    const w = await mountPage();
    await w
      .findAll(".seg-item")
      .find((b) => b.text().includes("批次"))!
      .trigger("click");
    await flushPromises();
    expect(btn(w, "撤销")).toBeUndefined();
  });
});
