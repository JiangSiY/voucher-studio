// @vitest-environment node
/**
 * 附件图片回退阶梯（纯逻辑）。
 *
 * 场景来源：2026-09-17 缺陷——备份不含增强版（可重建缓存），恢复快照后
 * DB 的 `enhanced_path` 指向磁盘上不存在的文件，界面若直接照用就是一块灰白。
 * 后端已在下发时剥掉坏路径（`services::attachment_view`），这里的阶梯是第二道保险。
 */
import { describe, expect, it } from "vitest";
import {
  candidatePaths,
  nextFallbackStep,
  pickCandidate,
} from "../useAttachmentFallback";

describe("candidatePaths", () => {
  it("有增强版时：增强优先、原图兜底", () => {
    const a = { filePath: "2026/08/x.jpg", enhancedPath: "2026/08/x.enhanced.jpg" };
    expect(candidatePaths(a)).toEqual(["2026/08/x.enhanced.jpg", "2026/08/x.jpg"]);
  });

  it("增强版缺失（null/undefined/空串）时只用原图", () => {
    expect(candidatePaths({ filePath: "a.jpg", enhancedPath: null })).toEqual(["a.jpg"]);
    expect(candidatePaths({ filePath: "a.jpg" })).toEqual(["a.jpg"]);
    expect(candidatePaths({ filePath: "a.jpg", enhancedPath: "   " })).toEqual(["a.jpg"]);
  });

  it("forceOriginal（裁剪基底 / A-B 对比底层）时忽略增强版", () => {
    const a = { filePath: "a.jpg", enhancedPath: "a.enhanced.jpg" };
    expect(candidatePaths(a, true)).toEqual(["a.jpg"]);
  });

  it("增强与原图相同时不重复", () => {
    expect(candidatePaths({ filePath: "a.jpg", enhancedPath: "a.jpg" })).toEqual(["a.jpg"]);
  });
});

describe("pickCandidate / nextFallbackStep", () => {
  const a = { filePath: "a.jpg", enhancedPath: "a.enhanced.jpg" };

  it("按步取候选，用尽返回 null（调用方显示占位）", () => {
    expect(pickCandidate(a, 0)).toBe("a.enhanced.jpg");
    expect(pickCandidate(a, 1)).toBe("a.jpg");
    expect(pickCandidate(a, 2)).toBeNull();
    expect(pickCandidate(a, 99)).toBeNull();
  });

  it("只有原图时第 0 步就是原图、第 1 步即用尽", () => {
    const b = { filePath: "a.jpg" };
    expect(pickCandidate(b, 0)).toBe("a.jpg");
    expect(pickCandidate(b, 1)).toBeNull();
  });

  it("失败步进不超过候选数（用尽后停在末值，避免越界）", () => {
    expect(nextFallbackStep(0, 2)).toBe(1);
    expect(nextFallbackStep(1, 2)).toBe(2);
    expect(nextFallbackStep(2, 2)).toBe(2);
  });
});
