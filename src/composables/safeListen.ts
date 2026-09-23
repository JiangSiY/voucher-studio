import { listen, type Event, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * 安全的事件监听：注册失败只告警，不打断后续注册。
 *
 * 背景（2026-09-23 审计 B15）：各页面此前把 `await listen(...)` 串行裸奔在 `try` 之外——
 * 任一注册 reject 会让**后续监听全部不注册**（功能静默缺失），同时产生 unhandled rejection。
 * `App.vue` 早先为自身 8 个监听加过局部 `safeListen`（Q-22），但修复没有下推到页面，
 * 本模块把它提为共用实现。
 *
 * 返回值可能是 `null`（注册失败）——调用方清理时用 `unlisten?.()` 即可。
 */
export async function safeListen<T>(
  event: string,
  handler: (e: Event<T>) => void,
): Promise<UnlistenFn | null> {
  try {
    return await listen<T>(event, handler);
  } catch (e) {
    console.warn(`事件监听注册失败（${event}）`, e);
    return null;
  }
}
