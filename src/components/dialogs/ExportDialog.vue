<script setup lang="ts">
/**
 * 打印与导出弹层（v0.27 B4）：导出与导出历史两片。
 * URL `?modal=export&tab=history` 直达历史；关闭回原页。
 */
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import Dialog from "../ui/Dialog.vue";
import PrintCenterPage from "../../pages/PrintCenterPage.vue";
import ExportHistoryPage from "../../pages/ExportHistoryPage.vue";
import { useModalRoute } from "../../composables/useModalRoute";

const route = useRoute();
const router = useRouter();
const { modal, close } = useModalRoute();

const tab = computed(() => (route.query.tab === "history" ? "history" : "export"));

function setTab(next: "export" | "history") {
  void router.replace({ query: { ...route.query, modal: modal.value, tab: next } });
}
</script>

<template>
  <Dialog :open="true" title="打印与导出" wide @close="close">
    <div class="export-tabs segmented" role="tablist" aria-label="导出视图">
      <button
        class="seg-item"
        :class="{ active: tab === 'export' }"
        type="button"
        role="tab"
        :aria-selected="tab === 'export'"
        @click="setTab('export')"
      >
        导出
      </button>
      <button
        class="seg-item"
        :class="{ active: tab === 'history' }"
        type="button"
        role="tab"
        :aria-selected="tab === 'history'"
        @click="setTab('history')"
      >
        导出历史
      </button>
    </div>
    <PrintCenterPage v-if="tab === 'export'" />
    <ExportHistoryPage v-else />
  </Dialog>
</template>

<style scoped>
.export-tabs {
  margin-bottom: var(--s-3);
}
</style>
