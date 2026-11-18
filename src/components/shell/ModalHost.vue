<script setup lang="ts">
/**
 * 弹层宿主（v0.27）：由 URL `?modal=` 驱动，B1 先给占位内容。
 * B4 在此处替换为真正的设置/备份/导出/重复核查/帮助组件。
 */
import { computed } from "vue";
import { useRoute } from "vue-router";
import Dialog from "../ui/Dialog.vue";
import HelpDialog from "../dialogs/HelpDialog.vue";
import { useModalRoute } from "../../composables/useModalRoute";

const route = useRoute();
const { modal, close } = useModalRoute();

const title = computed(() => {
  switch (modal.value) {
    case "settings":
      return "设置";
    case "backup":
      return "备份与恢复";
    case "export":
      return "打印与导出";
    case "duplicates":
      return "重复核查";
    case "help":
      return "帮助";
    default:
      return "";
  }
});

const hint = computed(() => {
  switch (modal.value) {
    case "settings":
      return "常规、命名、识别、增强、密钥与关于（B4 接入）。";
    case "backup":
      return "立即备份、最近备份、恢复与自动策略（B4 接入）。";
    case "export":
      return "范围、格式、预览与历史导出一体（B2/B4 接入）。";
    case "duplicates": {
      const pair = typeof route.query.pair === "string" ? route.query.pair : "";
      return pair ? `直接打开指定重复对：${pair}（B4 接入）。` : "按证据逐组处理（B4 接入）。";
    }
    case "help":
      return "快捷键、条目/凭证规则与版本信息（B4 接入）。";
    default:
      return "";
  }
});
</script>

<template>
  <HelpDialog v-if="modal === 'help'" />
  <Dialog
    v-else
    :open="modal !== ''"
    :title="title"
    :wide="modal === 'export' || modal === 'duplicates'"
    :narrow="modal === 'backup'"
    @close="close"
  >
    <p class="modal-placeholder">{{ hint }}</p>
  </Dialog>
</template>
