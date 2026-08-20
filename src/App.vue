<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useConfigStore } from "./stores/config";
import { toast } from "./stores/toast";
import type { BackupInfo, ImportSummary } from "./api/client";
import ToastHost from "./components/ToastHost.vue";

const configStore = useConfigStore();

// 全局事件告警（任何页面都可见）
let unlistenDone: UnlistenFn | null = null;
let unlistenFailed: UnlistenFn | null = null;
let unlistenWxImported: UnlistenFn | null = null;
let unlistenWxError: UnlistenFn | null = null;

onMounted(async () => {
  configStore.init();

  // 自动备份：失败必报；副本告警提示 3-2-1 不完整
  unlistenDone = await listen<BackupInfo>("backup:done", (e) => {
    if (e.payload.warnings?.length) {
      toast.warning(`自动备份完成，但副本存在问题：${e.payload.warnings.join("；")}`);
    }
  });
  unlistenFailed = await listen<string>("backup:failed", (e) => {
    toast.danger(`自动备份失败：${e.payload}`);
  });

  // 微信监听：导入结果与错误实时提示（事件内容哈希去重，重复图片静默跳过）
  unlistenWxImported = await listen<ImportSummary>("wechat:imported", (e) => {
    const s = e.payload;
    if (s.imported.length) {
      toast.success(`微信图片已入库：${s.voucherNo}（${s.imported.length} 张）`);
    }
  });
  unlistenWxError = await listen<string>("wechat:error", (e) => {
    toast.danger(`微信监听异常：${e.payload}`);
  });
});

onUnmounted(() => {
  unlistenDone?.();
  unlistenFailed?.();
  unlistenWxImported?.();
  unlistenWxError?.();
});
</script>

<template>
  <router-view />
  <ToastHost />
</template>
