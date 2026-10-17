<script setup lang="ts">
/**
 * 多选下拉（v0.22，2026-10-17 用户要求「下拉菜单做成可多选，可以做更多筛选组合且省空间」）。
 *
 * 形态：一个按钮充当输入框（沿用 `.input` 样式，高度与其它筛选项一致），点开是复选框浮层。
 * - 按钮文案：未选时显示占位名；选 1–2 项直接列出（省一次点开）；超过 2 项显示「名称（N）」。
 * - 浮层：选项复选框 + 「清空」；点外部或 Esc 关闭。
 * - 受控组件：`modelValue: string[]` + `update:modelValue`。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";

export type MultiOption = { value: string; label: string };

const props = defineProps<{
  /** 未选时的占位名，如「状态」 */
  label: string;
  options: MultiOption[];
  modelValue: string[];
  title?: string;
  maxWidth?: string;
  /** 互斥项：选中它会清掉其它已选项；选其它项时会清掉它（如"未标"与"收入/支出"） */
  exclusive?: string[];
}>();
const emit = defineEmits<{ "update:modelValue": [value: string[]] }>();

const open = ref(false);
const root = ref<HTMLElement | null>(null);

const summary = computed(() => {
  const labels = props.modelValue.map(
    (v) => props.options.find((o) => o.value === v)?.label ?? v,
  );
  if (!labels.length) return props.label;
  if (labels.length <= 2) return `${props.label}：${labels.join("、")}`;
  return `${props.label}（${labels.length}）`;
});

function toggle(value: string) {
  const excl = props.exclusive ?? [];
  if (excl.includes(value)) {
    // 互斥项：选中它 = 只留它；再点一次 = 取消
    emit("update:modelValue", props.modelValue.includes(value) ? [] : [value]);
    return;
  }
  const set = new Set(props.modelValue.filter((v) => !excl.includes(v)));
  if (props.modelValue.includes(value)) set.delete(value);
  else set.add(value);
  // 保持选项顺序，避免选了又删导致文案顺序跳动
  emit(
    "update:modelValue",
    props.options.filter((o) => set.has(o.value)).map((o) => o.value),
  );
}
function clear() {
  emit("update:modelValue", []);
}

function onDocPointer(e: MouseEvent) {
  if (root.value && !root.value.contains(e.target as Node)) open.value = false;
}
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") open.value = false;
}
onMounted(() => {
  document.addEventListener("mousedown", onDocPointer);
  document.addEventListener("keydown", onKey);
});
onBeforeUnmount(() => {
  document.removeEventListener("mousedown", onDocPointer);
  document.removeEventListener("keydown", onKey);
});
</script>

<template>
  <div ref="root" class="msel" :style="maxWidth ? { maxWidth } : undefined">
    <button
      type="button"
      class="input msel-btn"
      :class="{ 'is-active': modelValue.length > 0 }"
      :title="title || label"
      :aria-expanded="open"
      aria-haspopup="listbox"
      @click="open = !open"
    >
      <span class="msel-label">{{ summary }}</span>
      <span class="msel-caret" aria-hidden="true">▾</span>
    </button>
    <div v-if="open" class="msel-pop" role="listbox" aria-multiselectable="true">
      <label v-for="o in options" :key="o.value" class="msel-item">
        <input type="checkbox" :checked="modelValue.includes(o.value)" @change="toggle(o.value)" />
        <span>{{ o.label }}</span>
      </label>
      <button v-if="modelValue.length" type="button" class="msel-clear" @click="clear()">清空</button>
    </div>
  </div>
</template>

<style scoped>
.msel {
  position: relative;
  min-width: 0;
}
.msel-btn {
  display: inline-flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--s-2);
  width: 100%;
  cursor: pointer;
  text-align: left;
}
.msel-btn.is-active {
  border-color: var(--accent);
  color: var(--accent);
}
.msel-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.msel-caret {
  flex: none;
  color: var(--label-tertiary);
  font-size: 10px;
}
.msel-pop {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  z-index: 30;
  min-width: 100%;
  max-height: 260px;
  overflow-y: auto;
  padding: var(--s-2);
  background: var(--bg-content);
  border: 1px solid var(--separator-strong);
  border-radius: var(--radius-control);
  box-shadow: var(--shadow-2);
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.msel-item {
  display: flex;
  align-items: center;
  gap: var(--s-2);
  padding: 4px 6px;
  border-radius: var(--radius-small);
  font: var(--type-body);
  white-space: nowrap;
  cursor: pointer;
}
.msel-item:hover {
  background: var(--bg-selection);
}
.msel-clear {
  margin-top: 4px;
  padding: 4px 6px;
  border: 0;
  border-top: 1px solid var(--separator);
  background: transparent;
  color: var(--label-secondary);
  font: var(--type-caption);
  text-align: left;
  cursor: pointer;
}
.msel-clear:hover {
  color: var(--accent);
}
</style>
