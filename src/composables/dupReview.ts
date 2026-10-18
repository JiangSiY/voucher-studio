/**
 * 疑似重复核查的纯函数：置信度文案、字段对照、处置结论摘要。
 *
 * 页面只做取数与交互，展示口径收敛在这里（便于单测，也避免"页面里各写一份"）。
 * 术语（全仓"重复"至少三义，勿混）：
 * - **内容重复**：同一张图（导入期 MD5，阻断式）
 * - **疑似重复**：同日期 + 同金额（本页处置的对象，提示式；对方账户只作证据不是硬条件）
 * - **编号重复**：`voucher_no` UNIQUE 冲突（表单校验）
 */
import type { DupDecision, PairEvidence, Voucher } from "../api/client";
import { STATUS_META, fmtAmount } from "./format";

/** 置信度档位的展示口径（**只影响排序与提示强弱，不做过滤**） */
export const EVIDENCE_META: Record<
  PairEvidence["level"],
  { label: string; tag: string; advice: string }
> = {
  high: { label: "高", tag: "tag-danger", advice: "证据较强，建议优先核对" },
  medium: { label: "中", tag: "tag-warning", advice: "建议比对图片与摘要" },
  low: { label: "低", tag: "tag", advice: "也可能只是同日两笔相同金额" },
};

/** 档位未知（后端将来新增档位）时的兜底，避免界面出现空白标签 */
export function evidenceMeta(level: string): { label: string; tag: string; advice: string } {
  return EVIDENCE_META[level as PairEvidence["level"]] ?? EVIDENCE_META.low;
}

/** 这对"像在哪"的人话说明（空 → 只有判定口径本身） */
export function evidenceReasons(ev: PairEvidence): string[] {
  const out: string[] = [];
  if (ev.sameImage) out.push("图片内容完全相同");
  if (ev.sameCounterparty) out.push("对方账户相同");
  if (ev.sameDescription) out.push("摘要相同");
  if (out.length === 0) out.push("仅同日期同金额");
  return out;
}

const EMPTY = "—";

function text(s: string | null | undefined): string {
  const t = (s ?? "").trim();
  return t === "" ? EMPTY : t;
}

/** 旁列对照的一行 */
export interface CompareRow {
  key: string;
  label: string;
  left: string;
  right: string;
  /** 两侧显示文本是否一致（用于高亮差异） */
  same: boolean;
}

/**
 * 旁列对照的字段行（顺序固定）。
 * 日期与金额**必然相同**——这是判定口径本身，一并列出是让用户确认"确实同日同额"。
 */
export function compareRows(a: Voucher, b: Voucher): CompareRow[] {
  const pairs: Array<[string, string, string, string]> = [
    ["voucherNo", "凭证编号", text(a.voucherNo), text(b.voucherNo)],
    ["voucherDate", "凭证日期", text(a.voucherDate), text(b.voucherDate)],
    ["amountCents", "金额", fmtAmount(a.amountCents), fmtAmount(b.amountCents)],
    ["description", "摘要", text(a.description), text(b.description)],
    ["counterparty", "对方账户", text(a.counterparty), text(b.counterparty)],
    [
      "status",
      "状态",
      STATUS_META[a.status]?.label ?? text(a.status),
      STATUS_META[b.status]?.label ?? text(b.status),
    ],
    ["tags", "标签", text(a.tags?.join("、")), text(b.tags?.join("、"))],
    ["note", "备注", text(a.note), text(b.note)],
  ];
  return pairs.map(([key, label, left, right]) => ({
    key,
    label,
    left,
    right,
    same: left === right,
  }));
}

/** 差异行数（"8 项里 3 项不同"） */
export function diffCount(rows: CompareRow[]): number {
  return rows.filter((r) => !r.same).length;
}

/** 处置结论摘要（已处理页签一行说清"谁留谁删"） */
export function decisionSummary(d: DupDecision): string {
  if (d.decision === "not_duplicate") return "不是重复（未改动凭证）";
  const removedNo =
    d.removedPayload?.voucher?.voucherNo ?? (d.removedId != null ? `#${d.removedId}` : "?");
  const keptNo = d.removedPayload?.keptVoucherNo ?? (d.keepId != null ? `#${d.keepId}` : "?");
  return `保留 ${keptNo} · 删除 ${removedNo}`;
}

/** 处置结论标签（tag 配色与页签一致） */
export function decisionTag(d: DupDecision): { label: string; tag: string } {
  return d.decision === "not_duplicate"
    ? { label: "不是重复", tag: "tag" }
    : { label: "已删除重复", tag: "tag-danger" };
}

/** 组头文案：`2026-08-05 · ¥1,234.56 × 2 张` */
export function groupHeadline(g: {
  voucherDate: string;
  amountCents: number;
  vouchers: Voucher[];
}): string {
  return `${g.voucherDate} · ¥${fmtAmount(g.amountCents)} × ${g.vouchers.length} 张`;
}

/** 对的键（选中态与列表 key 共用；与后端 `normalize_pair` 同口径：小的在前） */
export function pairKey(x: number, y: number): string {
  return `${Math.min(x, y)}-${Math.max(x, y)}`;
}
