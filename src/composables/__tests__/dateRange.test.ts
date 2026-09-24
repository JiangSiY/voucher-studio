import { describe, expect, it } from "vitest";
import {
  daysInMonth,
  fmtYmd,
  monthRange,
  presetOfRange,
  rangeLabel,
  rangeOfPreset,
} from "../dateRange";

// 固定"今天"，保证用例与运行时间无关（跨年/跨月也不会飘）
const TODAY = new Date(2026, 8, 24); // 2026-09-24

describe("daysInMonth", () => {
  it("常规月份与闰年 2 月", () => {
    expect(daysInMonth(2026, 9)).toBe(30);
    expect(daysInMonth(2026, 8)).toBe(31);
    expect(daysInMonth(2026, 2)).toBe(28);
    expect(daysInMonth(2024, 2)).toBe(29); // 闰年
    expect(daysInMonth(2000, 2)).toBe(29); // 400 年闰
    expect(daysInMonth(1900, 2)).toBe(28); // 百年不闰
  });
});

describe("rangeOfPreset", () => {
  it("本月 = 当月首日到末日（闭区间）", () => {
    expect(rangeOfPreset("thisMonth", TODAY)).toEqual({
      dateFrom: "2026-09-01",
      dateTo: "2026-09-30",
    });
  });

  it("近 3 月 = 含当月在内 3 个自然月（与趋势图的 3 根柱子对齐）", () => {
    expect(rangeOfPreset("last3", TODAY)).toEqual({
      dateFrom: "2026-07-01",
      dateTo: "2026-09-30",
    });
  });

  it("近 6 月 = 含当月在内 6 个自然月", () => {
    expect(rangeOfPreset("last6", TODAY)).toEqual({
      dateFrom: "2026-04-01",
      dateTo: "2026-09-30",
    });
  });

  it("今年 = 1 月 1 日到 12 月 31 日（不是「至今」，月初不会只看到 1 天）", () => {
    expect(rangeOfPreset("thisYear", TODAY)).toEqual({
      dateFrom: "2026-01-01",
      dateTo: "2026-12-31",
    });
  });

  it("自定义由调用方给区间（本函数返回 null，不猜测）", () => {
    expect(rangeOfPreset("custom", TODAY)).toBeNull();
  });

  it("跨年边界：1 月里「近 3 月」应回推到去年 11 月", () => {
    const jan = new Date(2026, 0, 15);
    expect(rangeOfPreset("last3", jan)).toEqual({
      dateFrom: "2025-11-01",
      dateTo: "2026-01-31",
    });
  });

  it("月初/月末：本月窗口不随「今天」变化（同月内任一天结果相同）", () => {
    const first = new Date(2026, 8, 1);
    const last = new Date(2026, 8, 30);
    expect(rangeOfPreset("thisMonth", first)).toEqual(rangeOfPreset("thisMonth", last));
  });
});

describe("monthRange", () => {
  it("偏移跨年正确", () => {
    expect(monthRange(TODAY, -9)).toEqual({
      dateFrom: "2025-12-01",
      dateTo: "2025-12-31",
    });
    expect(monthRange(TODAY, 4)).toEqual({
      dateFrom: "2027-01-01",
      dateTo: "2027-01-31",
    });
  });
});

describe("presetOfRange", () => {
  it("已知区间能反推回预设（列表页返回工作台时回显口径）", () => {
    expect(presetOfRange({ dateFrom: "2026-09-01", dateTo: "2026-09-30" }, TODAY)).toBe("thisMonth");
    expect(presetOfRange({ dateFrom: "2026-01-01", dateTo: "2026-12-31" }, TODAY)).toBe("thisYear");
  });

  it("不匹配任何预设 → custom（不硬套近似预设）", () => {
    expect(presetOfRange({ dateFrom: "2026-09-02", dateTo: "2026-09-29" }, TODAY)).toBe("custom");
  });
});

describe("fmtYmd / rangeLabel", () => {
  it("补零且不受时区影响", () => {
    expect(fmtYmd(new Date(2026, 0, 5))).toBe("2026-01-05");
    expect(fmtYmd(new Date(2026, 11, 31))).toBe("2026-12-31");
  });

  it("口径说明含区间与状态", () => {
    expect(rangeLabel({ dateFrom: "2026-09-01", dateTo: "2026-09-30" }, "全部状态")).toBe(
      "2026-09-01 ~ 2026-09-30 · 全部状态",
    );
  });
});
