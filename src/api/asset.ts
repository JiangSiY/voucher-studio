/**
 * 资源地址：桌面版走 Tauri asset 协议，网页版走服务端 `/api/asset`。
 *
 * 为什么不直接用 `convertFileSrc`：`@tauri-apps/api` 的实现在浏览器里拿不到
 * `window.__TAURI_INTERNALS__`，调用会抛错；而网页版所有图片都在服务端数据目录里，
 * 由 `/api/asset?path=` 校验路径后返回（见 center-server/src/files.rs）。
 */
import { convertFileSrc as tauriConvertFileSrc } from "@tauri-apps/api/core";
import { isTauri } from "./env";

/** 图片/PDF 预览地址；空值返回空串（调用方据此回退占位图） */
export function assetUrl(abs?: string | null): string {
  if (!abs) return "";
  return isTauri ? tauriConvertFileSrc(abs) : `/api/asset?path=${encodeURIComponent(abs)}`;
}

/** 下载地址（导出批次、备份包）：网页版让浏览器接管下载 */
export function downloadUrl(abs: string): string {
  return isTauri ? abs : `/api/download?path=${encodeURIComponent(abs)}`;
}
