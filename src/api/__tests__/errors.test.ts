// @vitest-environment node
/**
 * 结构化错误归一测试（体检 #10 / A-4）
 *
 * 后端命令层返回 { code, message } 后，invoke reject 值变为对象；
 * 这些纯函数保证所有错误形态在 UI 归一为可展示文本。
 */
import { describe, expect, it } from "vitest";
import { asApiError, errMsg } from "../errors";

describe("asApiError", () => {
  it("仅当对象同时含 string code + message 时判定为结构化 ApiError", () => {
    expect(asApiError({ code: "lock_held", message: "数据目录已被其他实例锁定" })).toEqual({
      code: "lock_held",
      message: "数据目录已被其他实例锁定",
    });
  });

  it("只有 message 无 code（如 JS Error）不算 ApiError，返回 null", () => {
    expect(asApiError({ message: "只有文案" })).toBeNull();
    expect(asApiError(new Error("boom"))).toBeNull();
  });

  it("字符串 / null / undefined / code 非 string 均返回 null", () => {
    expect(asApiError("plain string")).toBeNull();
    expect(asApiError(null)).toBeNull();
    expect(asApiError(undefined)).toBeNull();
    expect(asApiError({ code: 5, message: "x" })).toBeNull();
  });
});

describe("errMsg", () => {
  it("结构化对象取 message（ApiError 主路径）", () => {
    expect(errMsg({ code: "db", message: "数据库错误: disk I/O" })).toBe("数据库错误: disk I/O");
  });

  it("只有 message 的普通对象也取 message（兜底不出现 [object Object]）", () => {
    expect(errMsg({ message: "只有文案" })).toBe("只有文案");
  });

  it("字符串原样（兼容旧版裸字符串 reject / 事件字符串）", () => {
    expect(errMsg("连接失败")).toBe("连接失败");
  });

  it("Error 实例取 message", () => {
    expect(errMsg(new Error("桌面功能不可用"))).toBe("桌面功能不可用");
  });

  it("未知形态回退 String 兜底，不产生 [object Object]", () => {
    expect(errMsg(42)).toBe("42");
    expect(errMsg(undefined)).toBe("undefined");
  });
});
