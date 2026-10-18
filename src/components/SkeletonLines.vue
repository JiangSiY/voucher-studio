<script setup lang="ts">
/**
 * 加载骨架（U4）：面板首次加载时的占位行。
 * 只表达「内容在路上」，不模拟具体数据；宽度逐行递减，避免看成一张表格。
 * reduced-motion 下由 base.css 关掉脉冲动画（静态灰条）。
 */
withDefaults(defineProps<{ lines?: number }>(), { lines: 4 });
</script>

<template>
  <div class="sk-stack" aria-busy="true" aria-label="加载中">
    <span
      v-for="i in lines"
      :key="i"
      class="skeleton sk-line"
      :style="{ width: `${70 - i * 7}%` }"
    ></span>
  </div>
</template>
