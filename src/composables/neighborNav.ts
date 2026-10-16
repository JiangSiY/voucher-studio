/**
 * 列表页「上一张 / 下一张」上下文翻张（v0.9 编辑页动线的回归；2026-10-16 用户要求前端实装）。
 *
 * 与列表内 ↑↓ 的**分工**：↑↓ 只在当前页内移动；这里按**整份筛选结果**翻张——可以跨页，
 * 且与列表同一份筛选口径（否则"下一张"会跳出用户眼前的筛选）。位置显示「第 n / N 张」。
 *
 * 顺序由后端 `get_voucher_neighbor` 决定：`voucher_date DESC, id DESC`，即**日期序**，
 * 与列表当前的排序选择无关（UI 上用 title 说明，免得用户以为按金额排序也能按金额翻）。
 */
import { computed, ref, type ComputedRef, type Ref } from "vue";
import { api, type VoucherNeighbor, type VoucherQuery } from "../api/client";

/** 位置文案（纯函数，便于单测）：没数据 → 空；不在筛选结果内 → 明确说明而不是显示 0/0 */
export function neighborLabel(n: VoucherNeighbor | null): string {
  if (!n) return "";
  if (n.position <= 0 || n.position > n.total) return "不在当前筛选内";
  return `第 ${n.position} / ${n.total} 张`;
}

export function useNeighborNav(opts: {
  /** 当前显示的凭证 id（null = 右栏空着，不需要翻张信息） */
  currentId: Ref<number | null>;
  /** 与列表同口径的筛选（含排序字段；后端忽略分页） */
  query: ComputedRef<VoucherQuery>;
}) {
  const neighbor = ref<VoucherNeighbor | null>(null);
  const positionLabel = computed(() => neighborLabel(neighbor.value));

  /** 请求序号：快速连续翻张会有多个请求在途，旧响应不得覆盖新响应（2026-09-23 审计 B3/B4） */
  let seq = 0;

  /**
   * 拉相邻张。返回**本次请求**的结果（调用方据此决定跳哪一张，避免读到别的请求写进去的值）；
   * 写回响应式值只在「本次请求仍是最新」时发生。
   * 失败静默隐藏——翻张是辅助能力，不该打断正在编辑的人。
   */
  async function load(): Promise<VoucherNeighbor | null> {
    const id = opts.currentId.value;
    if (!id) {
      seq += 1;
      neighbor.value = null;
      return null;
    }
    const my = ++seq;
    try {
      const n = await api.getVoucherNeighbor(id, opts.query.value);
      if (my === seq) neighbor.value = n;
      return n;
    } catch {
      if (my === seq) neighbor.value = null;
      return null;
    }
  }

  /** 右栏清空/离开列表时收起翻张信息（同时让在途响应失效） */
  function clear() {
    seq += 1;
    neighbor.value = null;
  }

  return { neighbor, positionLabel, load, clear };
}
