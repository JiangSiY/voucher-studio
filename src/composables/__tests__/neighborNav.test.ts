// @vitest-environment jsdom
/**
 * 上下文翻张（⌥←/→ 与右栏箭头）的组合式函数。
 * 重点守三件：位置文案、与列表同口径的查询透传、**在途响应不得覆盖新响应**
 * （快速连按 ⌥→ 时旧请求晚回会把位置写回上一张——2026-09-23 审计 B3/B4 的老毛病）。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { computed, ref } from "vue";

vi.mock("../../api/client", () => ({
  api: { getVoucherNeighbor: vi.fn() },
}));

import { api, type VoucherNeighbor, type VoucherQuery } from "../../api/client";
import { neighborLabel, useNeighborNav } from "../neighborNav";

const QUERY = { page: 1, pageSize: 1, status: "draft" } as unknown as VoucherQuery;

const deferred = <T,>() => {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
};

beforeEach(() => {
  vi.mocked(api.getVoucherNeighbor).mockReset();
});

describe("neighborLabel", () => {
  it("有位置就报「第 n / N 张」", () => {
    expect(neighborLabel({ prevId: 1, nextId: 3, position: 2, total: 12 })).toBe("第 2 / 12 张");
  });

  it("不在筛选结果内要明说（而不是显示 0 / 0）", () => {
    expect(neighborLabel({ prevId: null, nextId: 1, position: 0, total: 3 })).toBe("不在当前筛选内");
    expect(neighborLabel({ prevId: 1, nextId: null, position: 9, total: 3 })).toBe("不在当前筛选内");
  });

  it("没数据就是空串（右栏还没选中时不留半截文案）", () => {
    expect(neighborLabel(null)).toBe("");
  });
});

describe("useNeighborNav", () => {
  it("用当前凭证 id 与列表同口径的查询去取相邻张", async () => {
    const item: VoucherNeighbor = { prevId: null, nextId: 7, position: 1, total: 4 };
    vi.mocked(api.getVoucherNeighbor).mockResolvedValue(item);

    const { neighbor, positionLabel, load } = useNeighborNav({
      currentId: ref<number | null>(5),
      query: computed(() => QUERY),
    });
    await load();

    expect(api.getVoucherNeighbor).toHaveBeenCalledWith(5, QUERY);
    expect(neighbor.value).toEqual(item);
    expect(positionLabel.value).toBe("第 1 / 4 张");
  });

  it("没有选中凭证时不发请求（右栏空着就没有翻张可言）", async () => {
    const { neighbor, load } = useNeighborNav({
      currentId: ref<number | null>(null),
      query: computed(() => QUERY),
    });
    expect(await load()).toBeNull();
    expect(api.getVoucherNeighbor).not.toHaveBeenCalled();
    expect(neighbor.value).toBeNull();
  });

  it("在途响应不覆盖新响应（连按翻张时的顺序保护）", async () => {
    const first = deferred<VoucherNeighbor>();
    const second = deferred<VoucherNeighbor>();
    vi.mocked(api.getVoucherNeighbor)
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    const currentId = ref<number | null>(1);
    const { neighbor, load } = useNeighborNav({ currentId, query: computed(() => QUERY) });
    const p1 = load();
    currentId.value = 2;
    const p2 = load();

    // 后发的先回，先发的后回：写回必须是"最新那次"的结果
    second.resolve({ prevId: 1, nextId: 9, position: 2, total: 4 });
    await p2;
    expect(neighbor.value?.position).toBe(2);
    first.resolve({ prevId: null, nextId: 2, position: 1, total: 4 });
    await p1;
    expect(neighbor.value?.position).toBe(2);
  });

  it("取数失败静默隐藏（翻张是辅助能力，不打断编辑）", async () => {
    vi.mocked(api.getVoucherNeighbor).mockRejectedValue(new Error("boom"));
    const { neighbor, positionLabel, load } = useNeighborNav({
      currentId: ref<number | null>(3),
      query: computed(() => QUERY),
    });
    await expect(load()).resolves.toBeNull();
    expect(neighbor.value).toBeNull();
    expect(positionLabel.value).toBe("");
  });

  it("clear() 收起翻张信息，并让在途响应失效", async () => {
    const pending = deferred<VoucherNeighbor>();
    vi.mocked(api.getVoucherNeighbor).mockReturnValueOnce(pending.promise);
    const { neighbor, load, clear } = useNeighborNav({
      currentId: ref<number | null>(8),
      query: computed(() => QUERY),
    });
    const p = load();
    clear();
    pending.resolve({ prevId: 1, nextId: 2, position: 1, total: 2 });
    await p;
    expect(neighbor.value).toBeNull();
  });
});
