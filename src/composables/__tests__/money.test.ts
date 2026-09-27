import { describe, expect, it } from "vitest";
import { centsToYuanInput, parseTags, parseYuanToCents } from "../money";

describe("parseYuanToCents", () => {
  it("空串合法且表示不填金额", () => {
    expect(parseYuanToCents("")).toEqual({ ok: true, cents: null });
    expect(parseYuanToCents("   ")).toEqual({ ok: true, cents: null });
  });

  it("整数 / 一位小数 / 两位小数", () => {
    expect(parseYuanToCents("123")).toEqual({ ok: true, cents: 12300 });
    expect(parseYuanToCents("123.4")).toEqual({ ok: true, cents: 12340 });
    expect(parseYuanToCents("123.45")).toEqual({ ok: true, cents: 12345 });
  });

  it("负数（退款/红字凭证）", () => {
    expect(parseYuanToCents("-123.45")).toEqual({ ok: true, cents: -12345 });
    expect(parseYuanToCents("-0.5")).toEqual({ ok: true, cents: -50 });
  });

  it("浮点陷阱不出现（0.07 元应为 7 分，而不是 6.999…）", () => {
    expect(parseYuanToCents("0.07")).toEqual({ ok: true, cents: 7 });
    expect(parseYuanToCents("0.29")).toEqual({ ok: true, cents: 29 });
    expect(parseYuanToCents("19.99")).toEqual({ ok: true, cents: 1999 });
  });

  it("拒绝宽松解析（这是历史审计点名的输入校验）", () => {
    for (const bad of ["12.34.56", "1e3", "1,234", "abc", "123.456", "--1", "1.2.3", "+5"]) {
      const r = parseYuanToCents(bad);
      expect(r.ok, `应拒绝：${bad}`).toBe(false);
      if (!r.ok) expect(r.reason).toContain("最多两位小数");
    }
  });
});

describe("centsToYuanInput", () => {
  it("回填时可被 parseYuanToCents 原样解析回同一个值（往返一致）", () => {
    for (const cents of [0, 5, 50, 100, 12345, -12345, 1999]) {
      const text = centsToYuanInput(cents);
      expect(parseYuanToCents(text)).toEqual({ ok: true, cents });
    }
  });

  it("null / undefined → 空串（表示未填）", () => {
    expect(centsToYuanInput(null)).toBe("");
    expect(centsToYuanInput(undefined)).toBe("");
  });

  it("整数金额不带小数位；非整数补齐到两位（金额输入的常规呈现）", () => {
    expect(centsToYuanInput(200000)).toBe("2000");
    expect(centsToYuanInput(12340)).toBe("123.40");
    expect(centsToYuanInput(12304)).toBe("123.04");
  });
});

describe("parseTags", () => {
  it("中英文逗号/顿号都作分隔，去空去重保序", () => {
    expect(parseTags("差旅, 报销、差旅，，交通")).toEqual(["差旅", "报销", "交通"]);
    expect(parseTags("  ")).toEqual([]);
  });
});
