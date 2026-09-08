import { describe, expect, it, vi } from "vitest";
import { computed, ref } from "vue";
import { navContextToQuery, parseNavContext, useNeighborNav } from "../neighborNav";

// 隔离 Tauri invoke（测试环境无桌面宿主）
// 注意路径：测试位于 composables/__tests__/，真模块在 src/api/client
vi.mock("../../api/client", () => ({
  api: {
    getVoucherNeighbor: vi.fn(),
  },
}));

describe("parseNavContext", () => {
  it("合法 query 全量解析", () => {
    const ctx = parseNavContext({
      status: "draft",
      keyword: "美团",
      dateFrom: "2026-08-01",
      dateTo: "2026-08-31",
      from: "search",
    });
    expect(ctx).toEqual({
      status: "draft",
      keyword: "美团",
      dateFrom: "2026-08-01",
      dateTo: "2026-08-31",
      fromSearch: true,
    });
  });

  it("非法/缺失值退化为全量顺序", () => {
    expect(parseNavContext({ status: "bogus", keyword: 123, from: "elsewhere" })).toEqual({
      status: null,
      keyword: "",
      dateFrom: null,
      dateTo: null,
      fromSearch: false,
    });
    expect(parseNavContext({})).toEqual({
      status: null,
      keyword: "",
      dateFrom: null,
      dateTo: null,
      fromSearch: false,
    });
  });

  it("navContextToQuery 只携带非空值（与 parseNavContext 互逆）", () => {
    const ctx = parseNavContext({ status: "draft", keyword: "美团" });
    expect(navContextToQuery(ctx)).toEqual({ status: "draft", keyword: "美团" });
    expect(navContextToQuery(parseNavContext({}))).toEqual({});
  });
});

describe("useNeighborNav", () => {
  it("非编辑模式：不请求，neighbor 为空", async () => {
    const editId = ref<number | null>(null);
    const ctx = computed(() => parseNavContext({}));
    const { neighbor, positionLabel, loadNeighbor } = useNeighborNav({ editId, ctx });
    await loadNeighbor();
    expect(neighbor.value).toBeNull();
    expect(positionLabel.value).toBe("");
  });

  it("编辑模式：加载邻居并显示位置", async () => {
    const { api } = await import("../../api/client");
    vi.mocked(api.getVoucherNeighbor).mockResolvedValue({
      prevId: 1,
      nextId: 3,
      position: 2,
      total: 5,
    });
    const editId = ref<number | null>(2);
    const ctx = computed(() => parseNavContext({ status: "draft", keyword: "x" }));
    const { neighbor, positionLabel, loadNeighbor } = useNeighborNav({ editId, ctx });
    await loadNeighbor();
    expect(neighbor.value?.position).toBe(2);
    expect(positionLabel.value).toBe("第 2 / 5 张");
    // 搜索页口径透传 includeOcr
    const call = vi.mocked(api.getVoucherNeighbor).mock.calls[0];
    expect(call[1]).toMatchObject({ status: "draft", keyword: "x", includeOcr: false });
  });

  it("搜索页进入：includeOcr=true；超出筛选范围显示「不在当前筛选内」", async () => {
    const { api } = await import("../../api/client");
    vi.mocked(api.getVoucherNeighbor).mockResolvedValue({
      prevId: null,
      nextId: 9,
      position: 6,
      total: 5,
    });
    const editId = ref<number | null>(2);
    const ctx = computed(() => parseNavContext({ keyword: "出租车", from: "search" }));
    const { positionLabel, loadNeighbor } = useNeighborNav({ editId, ctx });
    await loadNeighbor();
    expect(positionLabel.value).toBe("不在当前筛选内");
    // mock.calls 跨同文件测试累积：取本测试的最后一次调用
    expect(vi.mocked(api.getVoucherNeighbor).mock.calls.at(-1)?.[1]).toMatchObject({
      includeOcr: true,
    });
  });

  it("请求失败：静默隐藏导航（不打扰编辑）", async () => {
    const { api } = await import("../../api/client");
    vi.mocked(api.getVoucherNeighbor).mockRejectedValue(new Error("db locked"));
    const editId = ref<number | null>(2);
    const ctx = computed(() => parseNavContext({}));
    const { neighbor, positionLabel, loadNeighbor } = useNeighborNav({ editId, ctx });
    await loadNeighbor();
    expect(neighbor.value).toBeNull();
    expect(positionLabel.value).toBe("");
  });
});
