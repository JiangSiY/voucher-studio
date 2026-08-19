/**
 * 全局拖放导入（对应 v4.3 的 drop 拖放导入——最高频录入入口）
 * 使用 Tauri WebView 的 drag-drop 事件获取文件绝对路径
 */
import { onUnmounted, ref } from "vue";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { api } from "../api/client";
import { toast, toastError } from "../stores/toast";

const IMAGE_FILTER = /\.(jpe?g|png|bmp|webp)$/i;

export function useDropImport(onImported: () => void, voucherId: number | null = null) {
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
          .importImages(images, voucherId, false)
          .then((summary) => {
            const parts = [`新凭证 ${summary.voucherNo}`];
            if (summary.imported.length) parts.push(`导入 ${summary.imported.length} 张`);
            if (summary.skippedDuplicates.length) parts.push(`跳过重复 ${summary.skippedDuplicates.length} 张`);
            if (summary.failed.length) parts.push(`失败 ${summary.failed.length} 张`);
            toast.success(parts.join(" · "));
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
