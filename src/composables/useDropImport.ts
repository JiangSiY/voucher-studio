/**
 * 全局拖放导入（对应 v4.3 的 drop 拖放导入——最高频录入入口）
 * 使用 Tauri WebView 的 drag-drop 事件获取文件绝对路径
 */
import { onUnmounted, ref } from "vue";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { api } from "../api/client";
import { toast, toastError } from "../stores/toast";

const IMAGE_FILTER = /\.(jpe?g|png|bmp|webp)$/i;

/**
 * 全局拖放导入。一图一凭证：每张图片各建一张凭证（voucherId 附加模式除外）。
 * @param runOcr 导入后是否立即 OCR 并自动填单（失败不影响入库）
 */
export function useDropImport(onImported: () => void, voucherId: number | null = null, runOcr = true) {
  const dragging = ref(false);
  let unlisten: (() => void) | null = null;

  async function bind() {
    unlisten = await getCurrentWebviewWindow().onDragDropEvent((event) => {
      const payload = event.payload;
      if (payload.type === "over") {
        dragging.value = true;
      } else if (payload.type === "leave") {
        dragging.value = false;
      } else if (payload.type === "drop") {
        dragging.value = false;
        const images = payload.paths.filter((p) => IMAGE_FILTER.test(p));
        if (images.length === 0) return;
        api
          .importImages(images, voucherId, runOcr)
          .then((summary) => {
            const parts: string[] = [];
            if (summary.vouchers.length) {
              parts.push(
                voucherId == null
                  ? `新建 ${summary.vouchers.length} 张凭证（一图一凭证）`
                  : `已附加图片到凭证 ${summary.vouchers[0].voucherNo}`,
              );
            }
            if (summary.skippedDuplicates.length) parts.push(`跳过重复 ${summary.skippedDuplicates.length} 张`);
            if (summary.failed.length) parts.push(`失败 ${summary.failed.length} 张`);
            if (parts.length) toast.success(parts.join(" · "));
            onImported();
          })
          .catch(toastError);
      }
    });
  }

  onUnmounted(() => unlisten?.());

  return { dragging, bind };
}

/** 打开系统文件选择器选图片 */
export async function pickImages(): Promise<string[] | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const result = await open({
    multiple: true,
    filters: [{ name: "图片", extensions: ["jpg", "jpeg", "png", "bmp", "webp"] }],
  });
  if (!result) return null;
  return Array.isArray(result) ? result : [result];
}
