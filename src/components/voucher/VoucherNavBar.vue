<script setup lang="ts">
/**
 * v0.9 上下文导航条（面板头右侧：上一张 / 位置 / 下一张）。
 * 从 VoucherFormPage 提取；仅展示相邻张可用态，跳转由父组件编排（脏数据走确认）。
 */
import FluentIcon from "../FluentIcon.vue";

defineProps<{
  prevId: number | null;
  nextId: number | null;
  positionLabel: string;
  /** 列表页带来的筛选关键词（tooltip 展示来源，无则不显示） */
  ctxKeyword: string;
}>();

const emit = defineEmits<{
  prev: [];
  next: [];
}>();
</script>

<template>
  <div class="vnav">
    <button class="btn btn-sm" :disabled="!prevId" title="上一张（Alt+←）" @click="emit('prev')">
      <FluentIcon name="chevronLeft" :size="13" />上一张
    </button>
    <span class="vnav-pos" :title="ctxKeyword ? `筛选：${ctxKeyword}` : '当前列表筛选'">{{ positionLabel }}</span>
    <button class="btn btn-sm" :disabled="!nextId" title="下一张（Alt+→）" @click="emit('next')">
      下一张<FluentIcon name="chevronRight" :size="13" />
    </button>
  </div>
</template>

<style scoped>
/* v0.9 上下文导航条（面板头右侧：上一张 / 位置 / 下一张） */
.vnav {
  display: flex;
  align-items: center;
  gap: 8px;
}

.vnav-pos {
  font-size: 12px;
  color: var(--text-3);
  min-width: 64px;
  text-align: center;
  white-space: nowrap;
}
</style>
