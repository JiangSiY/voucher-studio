<script setup lang="ts">
/**
 * 授权失效提示条（S0-07）。
 *
 * 依据 `docs/验收基线/证据/2026-11-19-S0/签审决策表.md` §6：
 * 失效后界面必须标注「需重新授权」与最后同步时间，并保留待处理项（不自动重发）。
 *
 * 边界：前端只做展示与二次确认，不实现真实 OAuth；「重新授权」仅触发本地合成
 * `auth_recover`（`api.devW2MockSync`，零网络）。真实恢复仍走 A2 逐次授权。
 */
import { computed, ref } from "vue";
import type { DevW2Outcome } from "../../api/client";
import ConfirmDialog from "../ConfirmDialog.vue";
import { fmtUtcSecs } from "../../utils/wpsTime";

const props = defineProps<{
  authRequired: boolean;
  lastSyncedAt: string | null;
  pendingOutcomes: DevW2Outcome[];
  cooldownUntil?: string | null;
}>();

const emit = defineEmits<{ reauth: [] }>();

const confirmOpen = ref(false);
const PENDING_STATUS: ReadonlySet<DevW2Outcome["status"]> = new Set([
  "unknown",
  "pending_backfill",
] as const);
const pending = computed(() => props.pendingOutcomes.filter((o) => PENDING_STATUS.has(o.status)));
/** 最后同步时间当前没有只读命令暴露（`wps_sync_runs` 最近一条），未知就照实标待补充。 */
const lastSyncedText = computed(() =>
  props.lastSyncedAt ? fmtUtcSecs(props.lastSyncedAt) : "—（数据源待补充）",
);
const cooldownText = computed(() => fmtUtcSecs(props.cooldownUntil ?? null));

function codeOf(o: DevW2Outcome): string {
  if (o.error) return o.error.split(":")[0] ?? o.error;
  return o.status;
}

function onConfirm() {
  confirmOpen.value = false;
  emit("reauth");
}
</script>

<template>
  <div v-if="authRequired" class="reauth-banner" role="alert" data-testid="reauth-banner">
    <p class="reauth-line">
      <strong>需重新授权</strong>
      <span
        >WPS 授权已失效，当前仅可查看本机缓存。待处理
        {{ pending.length }} 项已保留，恢复后不会自动重发。</span
      >
    </p>
    <p class="reauth-line">最后同步时间：{{ lastSyncedText }}</p>
    <p class="reauth-line reauth-note">真实 OAuth 重授权：未实现（待 A2/真实凭据）。</p>
    <button class="btn btn-primary btn-sm" type="button" @click="confirmOpen = true">
      重新授权
    </button>
    <ul v-if="pending.length" class="reauth-queue" data-testid="reauth-queue">
      <li v-for="(o, i) in pending" :key="o.idempotencyKey">
        <span>{{ i + 1 }}.</span>
        <code>{{ o.idempotencyKey }}</code>
        <span class="reauth-code">{{ codeOf(o) }}</span>
      </li>
    </ul>
    <ConfirmDialog
      :visible="confirmOpen"
      title="重新授权（本地合成）"
      :body="`当前只能查看本机已缓存的数据。确认后将执行本地合成恢复（auth_recover）：不发起任何网络请求，也不代表真实 WPS 授权已完成；真实 OAuth 重授权未实现（待 A2/真实凭据）。待处理 ${pending.length} 项保留，恢复后不会自动重发。`"
      confirm-text="执行本地合成恢复"
      cancel-text="取消"
      :danger="false"
      @confirm="onConfirm"
      @cancel="confirmOpen = false"
    />
  </div>
  <p v-if="cooldownUntil" class="reauth-cooldown" role="status" data-testid="cooldown-banner">
    WPS 限流：冷却至 <code>{{ cooldownText }}</code
    >，冷却结束前不会发起新批次。冷却截止统一按 UTC 秒级显示。
  </p>
</template>

<style scoped>
.reauth-banner {
  display: flex;
  flex-direction: column;
  gap: var(--s-1);
  padding: var(--s-2);
  border: 1px solid var(--warning);
  border-radius: var(--radius-control);
  background: var(--warning-soft);
  color: var(--warning);
  font-size: var(--fs-caption);
}

.reauth-line {
  margin: 0;
}

.reauth-note {
  margin: 0;
  color: var(--text-3);
}

.reauth-banner .btn {
  align-self: flex-start;
}

.reauth-cooldown {
  margin: 0;
  padding: var(--s-2);
  border: 1px solid var(--warning);
  border-radius: var(--radius-control);
  background: var(--warning-soft);
  color: var(--warning);
  font-size: var(--fs-caption);
}

.reauth-queue {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.reauth-queue li {
  display: flex;
  gap: var(--s-2);
  align-items: baseline;
}

.reauth-code {
  font-family: var(--font-mono);
}
</style>
