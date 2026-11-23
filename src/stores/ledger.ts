/**
 * 台账流水线的**跨页共享计数**（v13，2026-11-14）。
 *
 * 为什么要有 store：侧栏「台账」徽标与待处理页的待办卡都要显示"还有几张等补附件"，
 * 与 `inboxStore` / `dupStore` 同构——各页各查一次会出现互相矛盾的瞬间
 * （这里刚挂完附件、那里还挂着旧数字）。
 *
 * 口径只有一条：`unattached`（有 `ledger_ref` 但还没有附件的凭证张数），
 * 与数据页「待补凭证」视图同源（都走 `ledger_counts`）。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { api, type LedgerCounts } from "../api/client";

export const useLedgerStore = defineStore("ledger", () => {
  const counts = ref<LedgerCounts | null>(null);

  /** 拉取计数（辅助信息，失败静默——不打扰主流程，徽标维持上次数字） */
  async function refresh() {
    try {
      counts.value = await api.ledgerCounts();
    } catch {
      // 见上：辅助能力取不到就保持原值
    }
  }

  /** 侧栏徽标用：待补附件张数 */
  const unattached = computed(() => counts.value?.unattached ?? 0);

  return { counts, unattached, refresh };
});
