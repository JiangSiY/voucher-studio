// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { effectScope } from "vue";
import { COMPACT_MAX, MEDIUM_MAX, useBreakpoint } from "../useBreakpoint";

/**
 * 断点 composable 的守卫（2026-11-18 UI 重设计 P0）。
 *
 * 三档必须和 `base.css` 的媒体查询同源：这里把 COMPACT_MAX / MEDIUM_MAX 当常量断言，
 * 改数值时若忘了同步样式表，至少测试会先红一次、提醒去改 CSS。
 * 环境说明：vitest 默认 jsdom 有 `matchMedia` 但只会返回 matches=false；
 * 需要模拟窄窗时按下面的 stub 覆写（每次用例结束还原，避免串味）。
 */
function stubMatchMedia(matchesFor: { compact: boolean; medium: boolean }) {
  const listeners: Record<string, Array<() => void>> = { compact: [], medium: [] };
  const factory = (query: string) => ({
    matches: query.includes(`${COMPACT_MAX}px`)
      ? matchesFor.compact
      : query.includes(`${MEDIUM_MAX}px`)
        ? matchesFor.medium
        : false,
    media: query,
    onchange: null,
    addEventListener: (_type: string, cb: () => void) => {
      const key = query.includes(`${COMPACT_MAX}px`) ? "compact" : "medium";
      listeners[key].push(cb);
    },
    removeEventListener: (_type: string, cb: () => void) => {
      const key = query.includes(`${COMPACT_MAX}px`) ? "compact" : "medium";
      listeners[key] = listeners[key].filter((fn) => fn !== cb);
    },
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  });
  vi.stubGlobal("matchMedia", vi.fn(factory));
  return listeners;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useBreakpoint", () => {
  it("断点常量与样式表约定一致（<768 手机 / 768–1099 平板 / ≥1100 桌面）", () => {
    expect(COMPACT_MAX).toBe(767);
    expect(MEDIUM_MAX).toBe(1099);
  });

  it("窄窗（390px）判定为 compact", () => {
    stubMatchMedia({ compact: true, medium: true });
    const scope = effectScope();
    const api = scope.run(() => useBreakpoint());
    expect(api?.isCompact.value).toBe(true);
    expect(api?.isWide.value).toBe(false);
    scope.stop();
  });

  it("平板（800px）判定为 medium", () => {
    stubMatchMedia({ compact: false, medium: true });
    const scope = effectScope();
    const api = scope.run(() => useBreakpoint());
    expect(api?.isMedium.value).toBe(true);
    expect(api?.isCompact.value).toBe(false);
    scope.stop();
  });

  it("宽窗（1440px）判定为 wide", () => {
    stubMatchMedia({ compact: false, medium: false });
    const scope = effectScope();
    const api = scope.run(() => useBreakpoint());
    expect(api?.isWide.value).toBe(true);
    scope.stop();
  });

  it("退出作用域时解绑媒体查询监听（HMR / 路由切页不泄漏）", () => {
    const listeners = stubMatchMedia({ compact: false, medium: false });
    const scope = effectScope();
    const api = scope.run(() => useBreakpoint());
    expect(listeners.compact.length).toBe(1);
    scope.stop();
    expect(listeners.compact.length).toBe(0);
    expect(api?.breakpoint.value).toBe("wide");
  });
});
