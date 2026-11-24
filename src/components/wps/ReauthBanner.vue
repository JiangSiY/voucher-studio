<script setup lang="ts">
/**
 * 授权失效提示条（S0-07）。
 *
 * 依据 `docs/验收基线/证据/2026-11-19-S0/签审决策表.md` §6：
 * 失效后界面必须标注「需重新授权」与最后同步时间，并保留待处理项（不自动重发）。
 *
 * 数据来源是只读命令 `wps_sync_status`（本机 SQLite，零网络）：`lastSyncAt` 给最后同步
 * 时间，`pendingCount` 给待处理条数，`pendingItems` 给四列队列（表 / 幂等键 / 错误码 / 状态）。
 * `unknownCount`（结果不明）单独成句，**不并入**徽标口径。状态命令失败时只降级本组件，
 * 不触发任何同步、重试或 WPS 请求。
 */
import { computed, ref } from "vue";
import type { WpsPendingItem } from "../../api/client";
import ConfirmDialog from "../ConfirmDialog.vue";
import { fmtUtcSecs } from "../../utils/wpsTime";

const props = defineProps<{
  authRequired: boolean;
  /** 只读命令返回的最近成功同步时刻（已按 `…Z` 归一）；无记录为 null。 */
  lastSyncAt: string | null;
  /** `wps_links.state='pending_backfill'` 条数（徽标口径）。 */
  pendingCount: number;
  /** `wps_write_intents.state='unknown'` 条数：需人工核对。 */
  unknownCount: number;
  /** 两源合并的待处理项（最多 20 条），只含码与状态。 */
  pendingItems: WpsPendingItem[];
  cooldownUntil?: string | null;
  /** 状态查询失败/旧壳无此命令：时间与队列降级为「状态不可用」，不影响页面其余部分。 */
  statusUnavailable?: boolean;
}>();

const emit = defineEmits<{ reauth: [] }>();

const confirmOpen = ref(false);
const unavailable = computed(() => props.statusUnavailable === true);
const lastSyncedText = computed(() => {
  if (unavailable.value) return "—（状态不可用）";
  return props.lastSyncAt ? fmtUtcSecs(props.lastSyncAt) : "—（尚无同步记录）";
});
const lastSyncSentence = computed(() => `最后同步时间：${lastSyncedText.value}`);
/** 用整句 computed 而不是模板插值拼句子：避免 Prettier 折行在断言/展示里留下换行空白。 */
const pendingSentence = computed(() =>
  unavailable.value
    ? "WPS 授权已失效，当前仅可查看本机缓存；待处理项数不可用（状态查询失败），恢复后不会自动重发。"
    : `WPS 授权已失效，当前仅可查看本机缓存。待处理 ${props.pendingCount} 项已保留，恢复后不会自动重发。`,
);
const unknownSentence = computed(
  () => `有 ${props.unknownCount} 项结果不明，需人工核对，不会自动重发。`,
);
const cooldownText = computed(() => fmtUtcSecs(props.cooldownUntil ?? null));
const pendingItems = computed(() => (unavailable.value ? [] : props.pendingItems));

function onConfirm() {
  confirmOpen.value = false;
  emit("reauth");
}
</script>

<template>
  <div v-if="authRequired" class="reauth-banner" role="alert" data-testid="reauth-banner">
    <p class="reauth-line">
      <strong>需重新授权</strong>
      <span>{{ pendingSentence }}</span>
    </p>
    <p class="reauth-line" data-testid="last-sync-line">{{ lastSyncSentence }}</p>
    <p v-if="!unavailable && unknownCount > 0" class="reauth-line" data-testid="unknown-note">
      {{ unknownSentence }}
    </p>
    <p class="reauth-line reauth-note">真实 OAuth 重授权：未实现（待 A2/真实凭据）。</p>
    <button class="btn btn-primary btn-sm" type="button" @click="confirmOpen = true">
      重新授权
    </button>
    <table v-if="pendingItems.length" class="reauth-queue" data-testid="reauth-queue">
      <thead>
        <tr>
          <th>表</th>
          <th>幂等键</th>
          <th>错误码</th>
          <th>状态</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="it in pendingItems" :key="`${it.sheetId}:${it.idempotencyKey}`">
          <td>{{ it.sheetId }}</td>
          <td>
            <code>{{ it.idempotencyKey }}</code>
          </td>
          <td>
            <code>{{ it.code }}</code>
          </td>
          <td>{{ it.state }}</td>
        </tr>
      </tbody>
    </table>
    <ConfirmDialog
      :visible="confirmOpen"
      title="重新授权（本地合成）"
      :body="`当前只能查看本机已缓存的数据。确认后将执行本地合成恢复（auth_recover）：不发起任何网络请求，也不代表真实 WPS 授权已完成；真实 OAuth 重授权未实现（待 A2/真实凭据）。待处理 ${pendingCount} 项保留，恢复后不会自动重发。`"
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
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-caption);
}

.reauth-queue th,
.reauth-queue td {
  padding: 2px var(--s-1);
  text-align: left;
  vertical-align: baseline;
}

.reauth-queue th {
  color: var(--text-3);
  font-weight: 600;
  border-bottom: 1px solid var(--warning);
}

.reauth-queue code {
  font-family: var(--font-mono);
}
</style>
