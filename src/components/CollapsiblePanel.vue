<script setup lang="ts">
/**
 * 可折叠面板（v0.22 B 批：右栏编辑器重设计）。
 *
 * 统一「标题行（标题 + 徽标 + 右侧动作）+ 可展开内容」的外壳，给「识别结果」「修改历史」用。
 * - **折叠状态不持久化**（用户 2026-10-16 明确选择「不记忆」）：组件内 `ref`，切凭证或重新挂载即回到默认；
 * - `aria-expanded` / `aria-controls` 齐全，`Esc` 在标题或展开区按下即收起；
 * - 徽标在**折叠时也要能看出有没有内容**（如「识别到 3 项」「12 条 · 最近 10-13」）。
 */
import { ref } from "vue";

import FluentIcon from "./FluentIcon.vue";

const props = withDefaults(
  defineProps<{
    title: string;
    /** 折叠时也要显示的关键信息（条数 / 可填项） */
    badge?: string;
    /** 默认是否展开（B 批：识别结果与修改历史默认折叠） */
    defaultOpen?: boolean;
  }>(),
  { badge: "", defaultOpen: false },
);

const open = ref(props.defaultOpen);
const bodyId = `collapsible-panel-${Math.random().toString(36).slice(2, 9)}`;

function toggle() {
  open.value = !open.value;
}
function collapse() {
  open.value = false;
}

defineExpose({ collapse, isOpen: () => open.value });
</script>

<template>
  <section class="fluent-panel collapsible">
    <header class="fluent-panel-header cp-head">
      <button
        class="cp-toggle"
        type="button"
        :aria-expanded="open"
        :aria-controls="bodyId"
        @click="toggle"
        @keydown.esc="collapse"
      >
        <FluentIcon :name="open ? 'chevronDown' : 'chevronRight'" :size="14" />
        <h3>{{ title }}</h3>
        <span v-if="badge" class="cp-badge">{{ badge }}</span>
      </button>
      <slot name="actions" />
    </header>
    <!-- 折叠用 v-show（内容保留在 DOM 里，展开不重新请求/不丢输入） -->
    <div v-show="open" :id="bodyId" class="fluent-panel-body" @keydown.esc="collapse">
      <slot />
    </div>
  </section>
</template>

<style scoped>
.cp-head {
  /* 标题整行可点；右侧留出动作位 */
  padding: var(--s-2) var(--s-3);
}
.cp-toggle {
  display: flex;
  align-items: center;
  gap: var(--s-2);
  flex: 1;
  min-width: 0;
  padding: 2px 0;
  background: none;
  border: 0;
  color: var(--label-primary);
  cursor: pointer;
  text-align: left;
}
.cp-toggle h3 {
  font-size: 15px;
  font-weight: 600;
  margin: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.cp-badge {
  font: var(--type-caption);
  color: var(--label-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.cp-toggle:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
  border-radius: 4px;
}
</style>
