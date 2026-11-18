/**
 * 运行环境判定（唯一真源）。
 *
 * 单独成文件的原因：`api/client.ts` 被大量单测 `vi.mock` 整体替换（只给 `api` 对象），
 * 若资源地址模块也从 client 取 `isTauri`，这些测试会因"mock 上没有这个导出"直接报错。
 * 判定本身只依赖 `window.__TAURI_INTERNALS__`（Tauri 2 注入），与业务无关。
 */
export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
