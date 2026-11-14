// @vitest-environment jsdom
/**
 * 导入向导（v13）的**交互测试**：选文件后自动预演、写入按钮的启用条件、结果态。
 *
 * 关键契约：预演（dryRun=true）与写入（dryRun=false）是两次不同调用；
 * 全部命中（新建 0 张）时写入按钮必须禁用——否则用户会以为"导入了"其实什么都没发生。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import type { ImportSummary } from "../../api/client";
import { api } from "../../api/client";
import LedgerImportDialog from "../ledger/LedgerImportDialog.vue";

vi.mock("../../api/client", () => ({
  api: {
    pickLedgerCsv: vi.fn(),
    importLedgerCsv: vi.fn(),
  },
}));

const mockApi = vi.mocked(api);

function summary(over: Partial<ImportSummary> = {}): ImportSummary {
  return {
    vouchers: [],
    skippedDuplicates: [],
    failed: [],
    created: 3,
    matched: 0,
    skipped: 0,
    errors: [],
    batchId: 7,
    ...over,
  } as unknown as ImportSummary;
}

function mountDialog() {
  return mount(LedgerImportDialog, { props: { visible: true }, attachTo: document.body });
}

/** 浮层走 Teleport 到 body：按钮与文案都得从 document 里找，`w.findAll` 只能看到组件子树 */
function btn(text: string): HTMLButtonElement | undefined {
  return [...document.body.querySelectorAll<HTMLButtonElement>("button")].find((b) =>
    (b.textContent ?? "").includes(text),
  );
}

function seg(text: string): HTMLButtonElement | undefined {
  return [...document.body.querySelectorAll<HTMLButtonElement>(".seg-item")].find(
    (b) => (b.textContent ?? "").trim() === text,
  );
}

function bodyText(): string {
  return document.body.textContent ?? "";
}

async function click(el: HTMLElement | undefined) {
  expect(el).toBeTruthy();
  el!.click();
  await flushPromises();
}

beforeEach(() => {
  setActivePinia(createPinia());
  mockApi.pickLedgerCsv.mockResolvedValue("/tmp/收款清单.csv");
  mockApi.importLedgerCsv.mockResolvedValue(summary());
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("导入向导", () => {
  it("选文件后自动按所选类型预演（dryRun=true）并给出三个数字", async () => {
    mockApi.importLedgerCsv.mockResolvedValueOnce(summary({ created: 3, matched: 4 }));
    mountDialog();
    await click(btn("选择 CSV 文件…"));

    expect(mockApi.pickLedgerCsv).toHaveBeenCalled();
    expect(mockApi.importLedgerCsv).toHaveBeenCalledWith("/tmp/收款清单.csv", "shoukuan", true);
    const text = bodyText();
    expect(text).toContain("收款清单.csv");
    expect(text).toContain("新建");
    expect(text).toContain("写入 3 张");
  });

  it("写入是真写（dryRun=false），完成后给结果并回传 batchId", async () => {
    const w = mountDialog();
    await click(btn("选择 CSV 文件…"));
    mockApi.importLedgerCsv.mockClear();
    await click(btn("写入 3 张"));

    expect(mockApi.importLedgerCsv).toHaveBeenCalledWith("/tmp/收款清单.csv", "shoukuan", false);
    expect(bodyText()).toContain("已写入");
    await click(btn("完成"));
    expect(w.emitted("done")?.[0]).toEqual([7]);
  });

  it("全部命中（新建 0 张）时写入按钮禁用并说明原因", async () => {
    mockApi.importLedgerCsv.mockResolvedValueOnce(summary({ created: 0, matched: 100 }));
    mountDialog();
    await click(btn("选择 CSV 文件…"));

    expect(bodyText()).toContain("不会新建任何凭证");
    expect(btn("写入")!.disabled).toBe(true);
  });

  it("坏行可以在浮层里展开逐行看原因", async () => {
    mockApi.importLedgerCsv.mockResolvedValueOnce(
      summary({
        created: 1,
        skipped: 1,
        errors: [{ line: 3, ledgerRef: "SK-3", reason: "日期为空" }],
      }),
    );
    mountDialog();
    await click(btn("选择 CSV 文件…"));

    await click(btn("查看"));
    const text = bodyText();
    expect(text).toContain("第 3 行");
    expect(text).toContain("日期为空");
  });

  it("换类型会按新类型重新预演（表头不同，结果会变）", async () => {
    mountDialog();
    await click(btn("选择 CSV 文件…"));
    mockApi.importLedgerCsv.mockClear();

    await click(seg("计提"));
    expect(mockApi.importLedgerCsv).toHaveBeenCalledWith("/tmp/收款清单.csv", "jiti", true);
  });

  it("选文件取消（返回 null）时什么都不发生", async () => {
    mockApi.pickLedgerCsv.mockResolvedValue(null);
    mountDialog();
    await click(btn("选择 CSV 文件…"));
    expect(mockApi.importLedgerCsv).not.toHaveBeenCalled();
  });
});
