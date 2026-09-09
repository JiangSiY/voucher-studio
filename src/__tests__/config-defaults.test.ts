/**
 * 配置默认值漂移守卫（体检 B2#8）。
 *
 * 前端默认常量单一真源 src/config-defaults.ts 必须与
 * crates/voucher-core/src/utils/config.rs 的 AppConfig::default() 逐字段对齐。
 * 这里把「曾经双份真源分歧」过的两处关键默认值固化为断言：
 *   1. naming.template：前端旧值 `{date}-{seq}` → 对齐 Rust `{date-}-{n}`（2026-08-19-01）；
 *   2. print.title：前端旧值 `凭证日报` → 对齐 Rust `原始凭证`。
 * 若未来 Rust 侧调整默认值，需同步本文件与 src/config-defaults.ts（长期方案：ts-rs 生成 TS 类型）。
 */
import { describe, expect, it } from "vitest";
import { DEFAULT_NAMING_TEMPLATE, DEFAULT_PRINT_TITLE, defaultConfig } from "../config-defaults";

describe("config-defaults 与 Rust AppConfig::default() 对齐", () => {
  it("命名模板默认值应为 {date-}-{n}（2026-08-19-01，而非旧的 {date}-{seq}）", () => {
    expect(DEFAULT_NAMING_TEMPLATE).toBe("{date-}-{n}");
    expect(defaultConfig().naming.template).toBe("{date-}-{n}");
    expect(defaultConfig().naming.template).not.toBe("{date}-{seq}");
  });

  it("打印整理单标题默认值应为 原始凭证（而非旧的 凭证日报）", () => {
    expect(DEFAULT_PRINT_TITLE).toBe("原始凭证");
    expect(defaultConfig().print.title).toBe("原始凭证");
    expect(defaultConfig().print.title).not.toBe("凭证日报");
  });

  it("返回全新的可变副本（每次调用互不共享引用，避免编辑缓冲互相污染）", () => {
    const a = defaultConfig();
    const b = defaultConfig();
    expect(a).not.toBe(b);
    a.naming.template = "{seq}";
    expect(b.naming.template).toBe("{date-}-{n}");
  });
});
