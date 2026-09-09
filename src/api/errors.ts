/**
 * 结构化错误归一（体检 #10 / A-4）
 *
 * 后端命令层自 v0.13.2 起返回 `{ code, message }`（见 src-tauri/src/commands/mod.rs
 * 的 ApiError）：invoke reject 时收到的是一对象而非旧版裸字符串。
 * 本模块提供两个纯函数，把所有历史/未来错误形态统一成可展示文本：
 * - asApiError(e)：仅当对象同时含 string 型 code + message 才判定为结构化
 *   ApiError（JS Error 实例只有 message 无 code，不会被误判）；
 * - errMsg(e)：结构化对象/仅含 message 对象 → message；字符串原样
 *   （兼容旧后端/事件字符串）；Error 实例 → message；其余回退 String(e)，
 *   永不产生 "[object Object]"。
 */
export interface ApiErrorPayload {
  code: string;
  message: string;
}

export function asApiError(e: unknown): ApiErrorPayload | null {
  if (typeof e === "object" && e !== null) {
    const obj = e as Record<string, unknown>;
    if (typeof obj.code === "string" && typeof obj.message === "string") {
      return { code: obj.code, message: obj.message };
    }
  }
  return null;
}

export function errMsg(e: unknown): string {
  if (typeof e === "string") return e;
  const api = asApiError(e);
  if (api) return api.message;
  if (typeof e === "object" && e !== null) {
    // 兼容只有 message 的普通对象（如非标准 reject），避免 [object Object]
    const m = (e as Record<string, unknown>).message;
    if (typeof m === "string") return m;
  }
  if (e instanceof Error) return e.message;
  return String(e);
}
