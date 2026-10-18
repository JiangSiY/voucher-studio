<script setup lang="ts">
/**
 * 修改历史（快照）面板：按触发原因展示每次变更前的关键字段摘要。
 * 从 VoucherFormPage 提取；纯展示，数据由父组件加载。
 */
import { computed } from "vue";

import type { VoucherSnapshot } from "../../api/client";
import CollapsiblePanel from "../CollapsiblePanel.vue";
import { fmtDate } from "../../composables/format";

const props = defineProps<{
  snapshots: VoucherSnapshot[];
}>();

/** 徽标：条数 + 最近一次时间（折叠时也能判断要不要展开） */
const badge = computed(() => {
  const n = props.snapshots.length;
  if (!n) return "";
  const latest = props.snapshots[0]?.createdAt ?? "";
  return latest ? `${n} 条 · 最近 ${latest.slice(5, 16).replace("T", " ")}` : `${n} 条`;
});

/** 快照 payload 是任意 JSON；摘要取 description 字段（缺失时回退占位） */
function snapshotNote(payload: Record<string, unknown>): string {
  const d = payload["description"];
  return typeof d === "string" && d ? d : "（无摘要）";
}

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
  <CollapsiblePanel v-if="snapshots.length" title="修改历史（快照）" :badge="badge">
    <div>
      <div v-for="s in snapshots" :key="s.id" class="snapshot-item">
        <div class="sn-meta">
          <span>{{ SNAPSHOT_LABELS[s.reason] ?? s.reason }}</span>
          <span>{{ fmtDate(s.createdAt) }}</span>
        </div>
        <div style="color: var(--text-2)">{{ snapshotNote(s.payload) }}</div>
      </div>
    </div>
  </CollapsiblePanel>
</template>
