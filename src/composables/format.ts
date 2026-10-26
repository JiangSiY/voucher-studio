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

/**
 * v0.23.1：识别引擎的中文名（附件面板与识别结果卡共用一套口径）。
 * `deepseek` = 云端 DeepSeek 视觉；`cloud` = 自建旧协议；`local` = Apple Vision。
 */
export const OCR_ENGINE_LABELS: Record<string, string> = {
  deepseek: "云端 DeepSeek 视觉",
  cloud: "云端自建接口",
  local: "本地 Apple Vision",
};

/** 附件面板用的短名：一行里还要放状态与来源，长名会换行（完整名放 title） */
export const OCR_ENGINE_SHORT: Record<string, string> = {
  deepseek: "DeepSeek",
  cloud: "云端自建",
  // v0.23 之前的本地引擎是 PP-OCRv5、缓存里同样记 `local`：短名只写「本地」，
  // 避免把历史记录错标成 Vision（完整名在结果卡与 title 里给）
  local: "本地",
};

/** 引擎中文名（未知值原样返回，空值返回空串） */
export function ocrEngineLabel(engine: string | null | undefined): string {
  if (!engine) return "";
  return OCR_ENGINE_LABELS[engine] ?? engine;
}

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
