<script setup lang="ts">
/**
 * 修改历史（快照）面板：按触发原因展示每次变更前的关键字段摘要。
 * 从 VoucherFormPage 提取；纯展示，数据由父组件加载。
 */
import type { VoucherSnapshot } from "../../api/client";
import { fmtDate } from "../../composables/format";

defineProps<{
  snapshots: VoucherSnapshot[];
}>();

const SNAPSHOT_LABELS: Record<string, string> = {
  update: "修改前快照",
  status: "状态变更前快照",
  export: "导出前快照",
  delete: "删除前快照",
  ocr_fill: "OCR 填单前快照",
  replace_image: "换图前快照",
  renumber: "档号重排前快照",
};
</script>

<template>
  <div class="fluent-panel" v-if="snapshots.length">
    <div class="fluent-panel-header"><h3>修改历史（快照）</h3></div>
    <div class="fluent-panel-body">
      <div v-for="s in snapshots" :key="s.id" class="snapshot-item">
        <div class="sn-meta">
          <span>{{ SNAPSHOT_LABELS[s.reason] ?? s.reason }}</span>
          <span>{{ fmtDate(s.createdAt) }}</span>
        </div>
        <div style="color: var(--text-2)">
          {{ (s.payload as Record<string, unknown>).description || "（无摘要）" }}
        </div>
      </div>
    </div>
  </div>
</template>
