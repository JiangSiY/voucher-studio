<script setup lang="ts">
/** 通用确认对话框（对应 v4.3 的 warning dialog：危险操作前二次确认） */
import { useModal } from "../composables/useModal";

const props = withDefaults(
  defineProps<{
    visible: boolean;
    title?: string;
    body: string;
    confirmText?: string;
    /** 取消按钮文案（默认「取消」；「未保存修改」这类场景用「留在此页」更顺口） */
    cancelText?: string;
    danger?: boolean;
  }>(),
  { title: "确认操作", confirmText: "确定", cancelText: "取消", danger: true },
);

const emit = defineEmits<{ confirm: []; cancel: [] }>();

/** 焦点契约：打开聚焦首个可操作元素（本模板里是「取消」，危险动作默认焦点不落在删除上）、
    Tab 循环留在浮层内、Esc 取消并把焦点归还给打开它的按钮 */
const { panelEl } = useModal(
  () => props.visible,
  () => emit("cancel"),
);
</script>

<template>
  <Teleport to="body">
    <div v-if="props.visible" class="dialog-mask" @click.self="emit('cancel')">
      <div ref="panelEl" class="dialog" role="alertdialog" aria-modal="true" :aria-label="props.title" tabindex="-1">
        <h3>{{ props.title }}</h3>
        <div class="dialog-body">{{ props.body }}</div>
        <div class="dialog-ops">
          <button class="btn" @click="emit('cancel')">{{ props.cancelText }}</button>
          <button class="btn" :class="danger ? 'btn-danger' : 'btn-primary'" @click="emit('confirm')">
            {{ props.confirmText }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
