<script setup lang="ts">
/**
 * v0.10 摘要快速录入：输入框 + 联想下拉（模板置顶 pin + 历史高频带次数）。
 * 从 VoucherFormPage 提取；「联想」只改输入值，落库仍由父组件「保存」决定。
 */
import { computed, onBeforeUnmount, ref } from "vue";
import { useDescriptionSuggest } from "../../composables/descriptionSuggest";
import FluentIcon from "../FluentIcon.vue";

const props = defineProps<{
  modelValue: string;
  /** 用户自定义摘要模板（来自配置中心，置顶展示） */
  templates: string[];
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();

const templatesRef = computed(() => props.templates);
const { load: loadDescHistory, suggest: suggestDesc } = useDescriptionSuggest({
  templates: templatesRef,
});
const descFocused = ref(false);
const descHighlight = ref(-1);
/**
 * blur 延迟收起的定时器句柄（审计 A-10）：不接句柄的话，组件在 150ms 内被卸载
 * （表单路由切换）后回调仍会写已卸载组件的 ref；重复 blur 还会叠加多个定时器。
 */
let blurTimer: ReturnType<typeof setTimeout> | null = null;

/** 联想列表：聚焦时展示；键盘 ↑↓ 移动高亮（Enter 选中，Esc 关闭） */
const descSuggestions = computed(() => suggestDesc(props.modelValue));

function onDescFocus() {
  descFocused.value = true;
  loadDescHistory();
}

function onDescBlur() {
  // 延迟收起：点击下拉项时 mousedown 先于 blur 触发，直接关会吃掉点击
  clearBlurTimer();
  blurTimer = setTimeout(() => {
    blurTimer = null;
    descFocused.value = false;
    descHighlight.value = -1;
  }, 150);
}

function clearBlurTimer() {
  if (blurTimer !== null) {
    clearTimeout(blurTimer);
    blurTimer = null;
  }
}

function pickDescription(text: string) {
  emit("update:modelValue", text);
  clearBlurTimer(); // 选完即收起，不必等那 150ms
  descFocused.value = false;
  descHighlight.value = -1;
}

onBeforeUnmount(clearBlurTimer);

function onDescKeydown(e: KeyboardEvent) {
  if (!descFocused.value || !descSuggestions.value.length) return;
  if (e.key === "ArrowDown") {
    e.preventDefault();
    descHighlight.value = (descHighlight.value + 1) % descSuggestions.value.length;
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    descHighlight.value =
      descHighlight.value <= 0 ? descSuggestions.value.length - 1 : descHighlight.value - 1;
  } else if (e.key === "Enter" && descHighlight.value >= 0) {
    e.preventDefault();
    pickDescription(descSuggestions.value[descHighlight.value].description);
  } else if (e.key === "Escape") {
    descFocused.value = false;
    descHighlight.value = -1;
  }
}
</script>

<template>
  <div class="field desc-suggest">
    <label>摘要</label>
    <input :value="modelValue" class="input" placeholder="如：八月办公用品采购（聚焦可联想常用摘要）"
      @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
      @focus="onDescFocus" @blur="onDescBlur" @keydown="onDescKeydown" />
    <!-- v0.10 摘要联想：模板（置顶标 pin）+ 历史高频（带次数） -->
    <div v-if="descFocused && descSuggestions.length" class="desc-dropdown">
      <button v-for="(s, i) in descSuggestions" :key="s.description" type="button"
        class="desc-option" :class="{ active: i === descHighlight }"
        @mousedown.prevent="pickDescription(s.description)"
        @mousemove="descHighlight = i">
        <FluentIcon v-if="s.pinned" name="pin" :size="11" />
        <span class="desc-text">{{ s.description }}</span>
        <span v-if="s.count != null" class="desc-count">{{ s.count }} 次</span>
      </button>
    </div>
  </div>
</template>

<style scoped>
/* v0.10 摘要联想下拉（模板置顶 + 历史高频） */
.desc-suggest {
  position: relative;
}

.desc-dropdown {
  position: absolute;
  left: 0;
  right: 0;
  top: 100%;
  z-index: 30;
  margin-top: 4px;
  background: var(--bg-1, #fff);
  border: 1px solid var(--border);
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.12);
  max-height: 240px;
  overflow-y: auto;
  padding: 4px;
}

.desc-option {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 6px 8px;
  border: none;
  border-radius: 4px;
  background: none;
  cursor: pointer;
  font-size: 13px;
  text-align: left;
  color: var(--text-1);
}

.desc-option.active {
  background: var(--bg-3, rgba(0, 0, 0, 0.06));
}

.desc-text {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.desc-count {
  font-size: 11px;
  color: var(--text-3);
  flex-shrink: 0;
}
</style>
