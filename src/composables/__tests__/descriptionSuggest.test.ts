import { describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { filterSuggestions, useDescriptionSuggest } from "../descriptionSuggest";

// 隔离 Tauri invoke（测试环境无桌面宿主）
vi.mock("../../api/client", () => ({
  api: {
    frequentDescriptions: vi.fn().mockResolvedValue([
      { description: "办公用品采购", count: 5 },
      { description: "差旅住宿费", count: 2 },
    ]),
  },
}));

describe("filterSuggestions（纯函数）", () => {
  const items = [
    { description: "办公用品采购", count: null, pinned: true },
    { description: "差旅住宿费", count: null, pinned: true },
    { description: "办公用品采购", count: 5, pinned: false },
    { description: "市内交通费", count: 1, pinned: false },
    { description: "  ", count: 1, pinned: false },
  ];

  it("空输入 = 常用速选：模板在前、历史按频次、去重去空", () => {
    const out = filterSuggestions(items, "", 10);
    expect(out.map((o) => o.description)).toEqual([
      "办公用品采购", // 模板
      "差旅住宿费", // 模板
      "市内交通费", // 历史（模板重复项被去重）
    ]);
  });

  it("前缀过滤：仅保留包含输入的项", () => {
    const out = filterSuggestions(items, "办公", 10);
    expect(out.map((o) => o.description)).toEqual(["办公用品采购"]);
  });

  it("与输入完全相等的项不出现（已在输入框里）", () => {
    const out = filterSuggestions(items, "办公用品采购", 10);
    expect(out).toHaveLength(0);
  });

  it("limit 截断", () => {
    const out = filterSuggestions(items, "", 2);
    expect(out).toHaveLength(2);
  });

  it("pinned 标记与次数展示字段", () => {
    const out = filterSuggestions(items, "", 10);
    expect(out[0].pinned).toBe(true);
    expect(out[0].count).toBeNull();
    expect(out[2].pinned).toBe(false);
    expect(out[2].count).toBe(1);
  });
});

describe("useDescriptionSuggest", () => {
  it("模板 + 历史合并：模板置顶、历史带次数", async () => {
    const templates = ref(["固定摘要"]);
    const { load, suggest } = useDescriptionSuggest({ templates });
    await load();
    const out = suggest("", 10);
    expect(out.map((o) => o.description)).toEqual(["固定摘要", "办公用品采购", "差旅住宿费"]);
    expect(out[0].pinned).toBe(true);
    expect(out[1].count).toBe(5);
  });

  it("模板响应式变化即时生效", async () => {
    const templates = ref<string[]>([]);
    const { load, suggest } = useDescriptionSuggest({ templates });
    await load();
    expect(suggest("", 10)).toHaveLength(2); // 仅历史
    templates.value = ["新模板"];
    expect(suggest("", 10)[0].description).toBe("新模板");
  });
});
