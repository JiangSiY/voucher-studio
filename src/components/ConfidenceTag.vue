<script setup lang="ts">
/** v5：OCR 字段置信度徽标（版面几何配对 high / 标签命中 medium / 启发式兜底 low） */
defineProps<{ level: "high" | "medium" | "low" | null }>();
</script>

<template>
  <span v-if="level === 'high'" class="tag tag-success" title="版面几何配对或标签命中，可信度高">可信</span>
  <span v-else-if="level === 'low'" class="tag tag-warning" title="启发式兜底推测（如首行店名），请人工核对">低置信</span>
  <span v-else-if="level === 'medium'" class="tag" title="标签命中但无版面佐证，建议核对" style="color: var(--text-3)">中等</span>
</template>
