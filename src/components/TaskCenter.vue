<script setup lang="ts">
/**
 * 任务中心浮层（规范 §1.5：全局浮层、可最小化）。
 *
 * 与 toast 分工：toast = 一次性结果（几秒后消失）；这里 = **过程**（还在跑 / 跑到几成 / 去哪看结果）。
 * 位置放内容区左下（`base.css` 的 `.task-center`）：右下角已经归 toast，两个浮层挤一个角会互相盖。
 * 有新任务时 store 会自动展开——最小化状态下悄悄开跑等于没有任务中心。
 */
import { useTaskStore, type TaskItem } from "../stores/tasks";
import FluentIcon from "./FluentIcon.vue";

const store = useTaskStore();

const icons: Record<string, string> = {
  import: "image",
  export: "print",
  ocr: "sparkle",
  enhance: "sparkle",
  backup: "save",
};

function percent(t: TaskItem): number {
  if (t.total <= 0) return 0;
  return Math.min(100, Math.round((t.done / t.total) * 100));
}

function stateIcon(t: TaskItem): string {
  if (t.state === "done") return "checkCircle";
  if (t.state === "failed") return "warning";
  return icons[t.kind] ?? "info";
}
</script>

<template>
  <div v-if="store.items.length" class="task-center" :class="{ 'is-collapsed': store.collapsed }">
    <!-- 头部即最小化开关：收起态只剩这一行胶囊 -->
    <button
      class="tc-header"
      :aria-expanded="!store.collapsed"
      aria-controls="task-center-list"
      :title="store.collapsed ? '展开任务中心' : '最小化任务中心'"
      @click="store.toggleCollapsed()"
    >
      <FluentIcon name="chevronRight" :size="12" class="tc-caret" />
      <span class="tc-title">任务中心</span>
      <span class="tc-summary" role="status" aria-live="polite">{{ store.summary }}</span>
    </button>

    <div v-show="!store.collapsed" id="task-center-list" class="tc-list">
      <div v-for="t in store.items" :key="t.id" class="tc-item" :class="t.state">
        <span class="tc-icon" :title="t.state === 'failed' ? t.error : t.title">
          <FluentIcon :name="stateIcon(t)" :size="14" />
        </span>
        <div class="tc-body">
          <div class="tc-line">
            <span class="tc-name">{{ t.title }}</span>
            <span v-if="t.state === 'failed'" class="tc-error">{{ t.error }}</span>
            <span v-else-if="t.total > 0" class="tc-count">{{ t.done }} / {{ t.total }}</span>
            <span v-else-if="t.state === 'done'" class="tc-count">已完成</span>
          </div>
          <div v-if="t.detail" class="tc-detail">{{ t.detail }}</div>
          <!-- 有总量才画确定态进度条；不知道进度就画不确定条纹（不是"卡在 0%"） -->
          <div
            v-if="t.state === 'running'"
            class="tc-bar"
            :class="{ 'is-indeterminate': t.total <= 0 }"
            role="progressbar"
            :aria-valuenow="t.total > 0 ? percent(t) : undefined"
            :aria-valuemin="t.total > 0 ? 0 : undefined"
            :aria-valuemax="t.total > 0 ? 100 : undefined"
            :aria-label="`${t.title} 进度`"
          >
            <span
              class="tc-fill"
              :style="t.total > 0 ? { width: percent(t) + '%' } : undefined"
            ></span>
          </div>
        </div>
        <button
          v-if="t.action && t.state !== 'running'"
          class="btn btn-sm"
          @click="
            t.action.onClick();
            store.dismiss(t.id);
          "
        >
          {{ t.action.label }}
        </button>
        <button
          class="btn-icon tc-close"
          :style="{ width: 'var(--control-h-xs)', height: 'var(--control-h-xs)' }"
          aria-label="关闭这条任务"
          @click="store.dismiss(t.id)"
        >
          <FluentIcon name="close" :size="12" />
        </button>
      </div>
      <div v-if="store.finished.length > 1" class="tc-foot">
        <button class="link" @click="store.clearFinished()">清除已完成</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.task-center {
  position: fixed;
  left: calc(var(--sidebar-w) + var(--s-5));
  bottom: var(--s-5);
  z-index: 900; /* 低于 toast（1000）：结果提示永远压在上面 */
  width: min(420px, calc(100vw - var(--sidebar-w) - 2 * var(--s-5)));
  background: var(--bg-popover);
  border: 1px solid var(--separator);
  border-radius: var(--radius-panel);
  box-shadow: var(--shadow-popover);
  overflow: hidden;
}
/* 收起态与窄窗图标栏都是 56px —— 固定定位靠 `--sidebar-w` 跟不动，得单独纠一次 */
:root[data-sidebar-collapsed="true"] .task-center {
  left: calc(56px + var(--s-5));
  width: min(420px, calc(100vw - 56px - 2 * var(--s-5)));
}
@media (max-width: 760px) {
  .task-center {
    left: calc(56px + var(--s-4));
    width: calc(100vw - 56px - 2 * var(--s-4));
  }
}

.tc-header {
  display: flex;
  align-items: center;
  gap: var(--s-2);
  width: 100%;
  padding: var(--s-2) var(--s-3);
  border: none;
  background: transparent;
  color: var(--label-primary);
  font: var(--type-caption-strong, var(--type-caption));
  text-align: left;
  cursor: default;
}
.tc-header:hover {
  background: var(--bg-selection);
}
.tc-caret {
  transition: transform var(--motion-fast) var(--motion-curve);
  transform: rotate(90deg);
}
.task-center.is-collapsed .tc-caret {
  transform: none;
}
.tc-title {
  font-weight: 600;
}
.tc-summary {
  margin-left: auto;
  color: var(--label-secondary);
}

.tc-list {
  border-top: 1px solid var(--separator);
  max-height: 40vh;
  overflow-y: auto;
}
.tc-item {
  display: flex;
  align-items: flex-start;
  gap: var(--s-2);
  padding: var(--s-2) var(--s-3);
}
.tc-item + .tc-item {
  border-top: 1px solid var(--separator);
}
.tc-icon {
  color: var(--label-secondary);
  padding-top: 2px;
}
.tc-item.done .tc-icon {
  color: var(--success);
}
.tc-item.failed .tc-icon {
  color: var(--danger);
}
.tc-body {
  flex: 1;
  min-width: 0;
}
.tc-line {
  display: flex;
  align-items: baseline;
  gap: var(--s-2);
}
.tc-name {
  font: var(--type-caption);
  color: var(--label-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tc-count {
  margin-left: auto;
  font: var(--type-caption);
  color: var(--label-tertiary);
  font-variant-numeric: tabular-nums;
}
.tc-error {
  margin-left: auto;
  font: var(--type-caption);
  color: var(--danger);
}
.tc-detail {
  font: var(--type-caption);
  color: var(--label-tertiary);
  margin-top: 2px;
}

.tc-bar {
  position: relative;
  height: 3px;
  margin-top: 6px;
  border-radius: var(--radius-small);
  background: var(--separator);
  overflow: hidden;
}
.tc-fill {
  display: block;
  height: 100%;
  background: var(--accent);
  transition: width var(--motion-normal) var(--motion-curve);
}
.tc-bar.is-indeterminate .tc-fill {
  width: 40%;
  animation: tc-slide var(--motion-slow) ease-in-out infinite;
}
@keyframes tc-slide {
  0% {
    margin-left: -40%;
  }
  100% {
    margin-left: 100%;
  }
}
@media (prefers-reduced-motion: reduce) {
  .tc-bar.is-indeterminate .tc-fill {
    animation: none;
    width: 100%;
    opacity: 0.4;
    margin-left: 0;
  }
}

.tc-close {
  color: var(--label-tertiary);
  flex-shrink: 0;
}
.tc-foot {
  padding: var(--s-2) var(--s-3);
  border-top: 1px solid var(--separator);
  text-align: right;
}
</style>
