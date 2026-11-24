/**
 * 冷却截止时间显示归一（S0-08）。
 *
 * 真源：`crates/voucher-core/src/services/wps_w2.rs`。
 * - 库内 `wps.rate_limit_until` 与行内错误用 `to_rfc3339_opts(SecondsFormat::Secs, true)` → `…Z`；
 * - preflight / initialize 拒绝文案用 `to_rfc3339()` → `…+00:00`（可能带纳秒）。
 * 前端必须把同一时刻的两种写法归一成同一字符串，否则 C4 人工比对三处会误判不一致。
 */
import { describe, expect, it } from "vitest";
import { fmtUtcSecs, fmtWpsMessage } from "../utils/wpsTime";

describe("fmtUtcSecs", () => {
  it("同一时刻的 Z 写法与 +00:00 纳秒写法归一后完全相等", () => {
    const fromDb = "2026-11-22T11:17:48Z";
    const fromPreflight = "2026-11-22T11:17:48.123456789+00:00";
    expect(fmtUtcSecs(fromPreflight)).toBe("2026-11-22T11:17:48Z");
    expect(fmtUtcSecs(fromPreflight)).toBe(fmtUtcSecs(fromDb));
  });

  it("非 UTC 偏移按同一时刻折算成 UTC 秒级", () => {
    expect(fmtUtcSecs("2026-11-22T19:17:48+08:00")).toBe("2026-11-22T11:17:48Z");
  });

  it("空值显示「—」，无时区或不可解析的原样返回（不猜本地时区）", () => {
    expect(fmtUtcSecs(null)).toBe("—");
    expect(fmtUtcSecs("")).toBe("—");
    expect(fmtUtcSecs("2026-11-22T11:17:48")).toBe("2026-11-22T11:17:48");
    expect(fmtUtcSecs("not-a-time")).toBe("not-a-time");
  });
});

describe("fmtWpsMessage", () => {
  it("把文案里内嵌的每个时间串都归一，不残留 +00:00/纳秒", () => {
    const raw =
      "WPS 限流冷却中，至 2026-11-22T11:17:48.987654321+00:00（库内 2026-11-22T11:17:48Z）";
    const shown = fmtWpsMessage(raw);
    expect(shown).not.toContain("+00:00");
    expect(shown).not.toContain("987654321");
    expect(shown).toContain("至 2026-11-22T11:17:48Z");
    expect(shown).toContain("库内 2026-11-22T11:17:48Z");
  });

  it("普通错误码文案原样保留", () => {
    expect(fmtWpsMessage("WPS_E_AUTH: 授权失效（400006）")).toBe("WPS_E_AUTH: 授权失效（400006）");
    expect(fmtWpsMessage(null)).toBe("");
  });
});
