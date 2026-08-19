<script setup lang="ts">
/** 通用确认对话框（对应 v4.3 的 warning dialog：危险操作前二次确认） */
const props = withDefaults(
  defineProps<{
    visible: boolean;
    title?: string;
    body: string;
    confirmText?: string;
    danger?: boolean;
  }>(),
  { title: "确认操作", confirmText: "确定", danger: true },
);

const emit = defineEmits<{ confirm: []; cancel: [] }>();
</script>

<template>
  <Teleport to="body">
    <div v-if="props.visible" class="dialog-mask" @click.self="emit('cancel')">
      <div class="dialog" role="alertdialog" :aria-label="props.title">
        <h3>{{ props.title }}</h3>
        <div class="dialog-body">{{ props.body }}</div>
        <div class="dialog-ops">
          <button class="btn" @click="emit('cancel')">取消</button>
          <button class="btn" :class="danger ? 'btn-danger' : 'btn-primary'" @click="emit('confirm')">
            {{ props.confirmText }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
