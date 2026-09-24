/**
 * 时间窗预设 → 查询参数（工作台口径控制器的**唯一真源**）。
 *
 * 为什么单独成模块：旧仪表盘的病灶是"同一屏三套时间口径且互不说明"（统计卡全库、
 * 状态分布只算 6 个月窗口、Top 是全历史前 10）。新工作台要求所有块共用同一个窗口，
 * 而"本月 / 近 3 月 / 近 6 月 / 今年 / 自定义"到具体日期的换算是唯一容易算错的地方
 * （月初月末、跨年、闰月），故抽成纯函数并逐条锁用例。
 *
 * **口径约定**（与后端 `dashboard_overview` 一致）：
 * - 区间为**闭区间** `[dateFrom, dateTo]`，两端都是 `YYYY-MM-DD`；
 * - "本月" = 当月 1 日 ~ 当月最后一天；"近 N 月" = 含当月在内的 N 个自然月
 *   （即从 N-1 个月前的 1 日到当月最后一天）——这样与"月度趋势"的 N 根柱子数量一致；
 * - "今年" = 当年 1 月 1 日 ~ 当年最后一天（不是"至今"，避免月初看到 1 天的窗口）；
 * - 自定义：用户选的两端原样透传（前端只做格式校验）。
 */

export type RangePreset = "thisMonth" | "last3" | "last6" | "thisYear" | "custom";

export interface DateRange {
  dateFrom: string;
  dateTo: string;
}

export const RANGE_PRESETS: { key: RangePreset; label: string }[] = [
  { key: "thisMonth", label: "本月" },
  { key: "last3", label: "近 3 月" },
  { key: "last6", label: "近 6 月" },
  { key: "thisYear", label: "今年" },
  { key: "custom", label: "自定义" },
];

/** `YYYY-MM-DD` 格式化（本地时区，不做 UTC 转换）。
 *  注意与 `composables/format.ts` 的 `fmtDate`（吃 ISO 字符串、用于展示）区分，故命名 `fmtYmd`。 */
export function fmtYmd(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** 某年某月的天数（`month` 为 1–12；用"下月 0 日"技巧，自动处理闰年） */
export function daysInMonth(year: number, month: number): number {
  return new Date(year, month, 0).getDate();
}

/** 当月闭区间 */
export function monthRange(today: Date, offsetMonths = 0): DateRange {
  const anchor = new Date(today.getFullYear(), today.getMonth() + offsetMonths, 1);
  const y = anchor.getFullYear();
  const m = anchor.getMonth() + 1;
  return {
    dateFrom: fmtYmd(new Date(y, m - 1, 1)),
    dateTo: fmtYmd(new Date(y, m - 1, daysInMonth(y, m))),
  };
}

/**
 * 预设 → 区间。`today` 由调用方注入（便于用例固定"今天"）。
 * `custom` 需要外部给出区间，本函数返回 null 表示"由调用方提供"。
 */
export function rangeOfPreset(preset: RangePreset, today: Date): DateRange | null {
  switch (preset) {
    case "thisMonth":
      return monthRange(today, 0);
    case "last3":
      return {
        dateFrom: monthRange(today, -2).dateFrom,
        dateTo: monthRange(today, 0).dateTo,
      };
    case "last6":
      return {
        dateFrom: monthRange(today, -5).dateFrom,
        dateTo: monthRange(today, 0).dateTo,
      };
    case "thisYear":
      return {
        dateFrom: fmtYmd(new Date(today.getFullYear(), 0, 1)),
        dateTo: fmtYmd(new Date(today.getFullYear(), 11, 31)),
      };
    case "custom":
      return null;
  }
}

/** 由区间反推最贴近的预设（用于"从列表页返回工作台时回显口径"；都不像则 custom） */
export function presetOfRange(range: DateRange, today: Date): RangePreset {
  for (const p of ["thisMonth", "last3", "last6", "thisYear"] as RangePreset[]) {
    const r = rangeOfPreset(p, today);
    if (r && r.dateFrom === range.dateFrom && r.dateTo === range.dateTo) return p;
  }
  return "custom";
}

/** 口径说明文案（块标题旁标注用）：如「本月 · 2026-09-01 ~ 2026-09-30 · 全部状态」 */
export function rangeLabel(range: DateRange, statusLabel: string): string {
  return `${range.dateFrom} ~ ${range.dateTo} · ${statusLabel}`;
}
