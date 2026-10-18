/** 凭证状态 → 徽章/文案映射（多页共用） */
import type { VoucherStatus } from "../api/client";

export const STATUS_META: Record<VoucherStatus, { label: string; tag: string }> = {
  draft: { label: "草稿", tag: "tag-warning" },
  reviewed: { label: "已复核", tag: "tag-info" },
  exported: { label: "已导出", tag: "tag-success" },
  archived: { label: "已归档", tag: "tag-muted" },
};

export const OCR_META: Record<string, { label: string; tag: string }> = {
  pending: { label: "待识别", tag: "tag-muted" },
  running: { label: "识别中", tag: "tag-info" },
  done: { label: "已识别", tag: "tag-success" },
  failed: { label: "识别失败", tag: "tag-danger" },
};

export function fmtAmount(cents: number | null): string {
  if (cents == null) return "—";
  return (cents / 100).toLocaleString("zh-CN", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
}

export function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function fmtDate(iso: string): string {
  return iso?.replace("T", " ").replace(/\.\d+Z$/, "") ?? "";
}
