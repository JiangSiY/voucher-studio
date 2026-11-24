<script setup lang="ts">
/**
 * 查看范围二次确认（S0-07 / 签审决策表 §8 待拍板 #4）。
 *
 * 授权失效时只允许查看本机已缓存的数据，且必须二次确认；确认态由调用方存
 * sessionStorage（同一会话只弹一次）。本组件只说明范围，不发任何请求。
 */
import { computed } from "vue";
import ConfirmDialog from "../ConfirmDialog.vue";
import { fmtUtcSecs } from "../../utils/wpsTime";

const props = defineProps<{ visible: boolean; lastSyncedAt: string | null }>();

const emit = defineEmits<{ confirm: []; cancel: [] }>();

/** 最后同步时间当前没有只读命令暴露（`wps_sync_runs` 最近一条），未知就照实标待补充。 */
const lastSyncedText = computed(() =>
  props.lastSyncedAt ? fmtUtcSecs(props.lastSyncedAt) : "—（数据源待补充）",
);

const body = computed(
  () =>
    `可见：本机已缓存的凭证列表、详情与待处理状态。` +
    `不可见：远端的任何新数据。` +
    `最后同步时间：${lastSyncedText.value}。` +
    `确认后仍为只读，不发起任何网络请求。`,
);
</script>

<template>
  <ConfirmDialog
    :visible="visible"
    title="查看范围确认（授权失效）"
    :body="body"
    confirm-text="确认，仅查看本机缓存"
    cancel-text="返回"
    :danger="false"
    @confirm="emit('confirm')"
    @cancel="emit('cancel')"
  />
</template>
