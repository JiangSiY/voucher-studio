/**
 * 编辑页上下文导航（v0.9 动线提取）：解析路由带来的列表筛选上下文，
 * 查询同筛选/同排序下的相邻凭证与位置（「上一张 / 第 n/N 张 / 下一张」）。
 * query 解析是纯函数（parseNavContext），可直接单测。
 */
import { computed, ref, type ComputedRef, type Ref } from "vue";
import { api, type VoucherNeighbor, type VoucherStatus } from "../api/client";

export interface NavContext {
  status: VoucherStatus | null;
  keyword: string;
  dateFrom: string | null;
  dateTo: string | null;
  /** 从搜索页进入（翻张按搜索结果同口径含 OCR，返回回搜索页） */
  fromSearch: boolean;
}

/** 从路由 query 解析导航上下文（非法值一律忽略 → 退化为全量顺序） */
export function parseNavContext(
  query: Record<string, unknown>,
): NavContext {
  const s = query.status;
  const status: VoucherStatus | null =
    s === "draft" || s === "reviewed" || s === "exported" || s === "archived"
      ? s
      : null;
  const keyword = typeof query.keyword === "string" ? query.keyword : "";
  const dateFrom =
    typeof query.dateFrom === "string" && query.dateFrom
      ? query.dateFrom
      : null;
  const dateTo =
    typeof query.dateTo === "string" && query.dateTo ? query.dateTo : null;
  return { status, keyword, dateFrom, dateTo, fromSearch: query.from === "search" };
}

/** 序列化为返回时还原视图的 query 对象（空值不携带） */
export function navContextToQuery(ctx: NavContext): Record<string, string> {
  const q: Record<string, string> = {};
  if (ctx.status) q.status = ctx.status;
  if (ctx.keyword) q.keyword = ctx.keyword;
  if (ctx.dateFrom) q.dateFrom = ctx.dateFrom;
  if (ctx.dateTo) q.dateTo = ctx.dateTo;
  return q;
}

export function useNeighborNav(opts: {
  editId: Ref<number | null>;
  ctx: ComputedRef<NavContext>;
}) {
  const neighbor = ref<VoucherNeighbor | null>(null);

  const positionLabel = computed(() => {
    const n = neighbor.value;
    if (!n) return "";
    if (n.position > n.total) return "不在当前筛选内";
    return `第 ${n.position} / ${n.total} 张`;
  });

  /** 请求序号：连续翻张/切换记录会有多个请求在途，旧响应不得覆盖新响应（2026-09-23 审计 B3/B4） */
  let seq = 0;

  /**
   * 拉取相邻凭证（失败静默隐藏导航条——导航是辅助能力，不阻断编辑）。
   * 返回**本次请求**的结果（调用方据此翻张，避免读到别的请求写进去的值）；
   * `neighbor` 响应式值只在「本次请求仍是最新」时才更新。
   */
  async function loadNeighbor(): Promise<VoucherNeighbor | null> {
    if (!opts.editId.value) {
      seq += 1; // 让在途响应失效
      neighbor.value = null;
      return null;
    }
    const c = opts.ctx.value;
    const my = ++seq;
    try {
      const n = await api.getVoucherNeighbor(opts.editId.value, {
        page: 1,
        pageSize: 1,
        status: c.status,
        keyword: c.keyword || null,
        dateFrom: c.dateFrom,
        dateTo: c.dateTo,
        // 搜索页进入：翻张与搜索结果同口径（含 OCR 命中），连续核对不跳张
        includeOcr: c.fromSearch,
      });
      if (my !== seq) return n; // 已被更晚的请求取代：不写回，但把本次结果交给调用方
      neighbor.value = n;
      return n;
    } catch {
      if (my === seq) neighbor.value = null;
      return null;
    }
  }

  return { neighbor, positionLabel, loadNeighbor };
}
