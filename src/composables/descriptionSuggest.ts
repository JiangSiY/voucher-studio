/**
 * v0.10 摘要快速录入：编辑页摘要输入框的联想建议。
 * 数据源 = 用户自定义模板（config.voucher.descriptionTemplates，排前）
 *        + 历史高频摘要（frequent_descriptions，按使用次数降序），
 * 合并去重后按「模板优先、历史按频次」排序。
 * filterSuggestions 是纯函数（输入前缀过滤 + 上限），可直接单测。
 */
import { ref, type Ref } from "vue";
import { api, type DescriptionRow } from "../api/client";

/** 联想项：模板项 pinned=true 排在历史项之前 */
export interface SuggestionItem {
  description: string;
  /** 使用次数（模板项为 null——不展示次数） */
  count: number | null;
  pinned: boolean;
}

/** 过滤 + 排序 + 截断（纯函数）：
 *  - pinned（模板）在前，历史按 count 降序
 *  - 与当前输入完全相等的项不出现（已在输入框里，再列没有意义）
 *  - 前缀不匹配的过滤掉（空输入 = 全部，作为「常用摘要」速选）
 */
export function filterSuggestions(
  items: SuggestionItem[],
  input: string,
  limit: number,
): SuggestionItem[] {
  const q = input.trim().toLowerCase();
  const seen = new Set<string>();
  const out: SuggestionItem[] = [];
  for (const it of items) {
    if (out.length >= limit) break;
    const d = it.description.trim();
    if (!d || seen.has(d)) continue;
    if (q && !d.toLowerCase().includes(q)) continue;
    if (q && d.toLowerCase() === q) continue;
    seen.add(d);
    out.push({ ...it, description: d });
  }
  return out;
}

export function useDescriptionSuggest(opts: {
  /** 用户自定义模板（响应式；来自 config store） */
  templates: Ref<string[]>;
}) {
  const history = ref<DescriptionRow[]>([]);
  const loaded = ref(false);

  /** 全量候选（模板 + 历史，模板在前） */
  const all = (): SuggestionItem[] => [
    ...opts.templates.value
      .map((t) => t.trim())
      .filter(Boolean)
      .map((description) => ({ description, count: null, pinned: true })),
    ...history.value.map((r) => ({
      description: r.description,
      count: r.count,
      pinned: false,
    })),
  ];

  /** 拉取历史高频摘要（失败静默——联想是辅助能力，不阻断录入） */
  async function load() {
    if (loaded.value) return;
    try {
      history.value = await api.frequentDescriptions(8);
      loaded.value = true;
    } catch {
      // 下次聚焦输入框时再试
    }
  }

  function suggest(input: string, limit = 8): SuggestionItem[] {
    return filterSuggestions(all(), input, limit);
  }

  return { load, suggest };
}
