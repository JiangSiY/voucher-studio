/**
 * 收件箱状态（v0.18 收件箱驱动重构）：全局共享"今天要处理多少事"。
 *
 * 为什么需要 store：侧栏徽标（`WorkbenchLayout`）与收件箱页共用同一个数字；
 * 任何处理动作（批量识别、核对、打印、处置重复）完成后都要刷新它，
 * 与 `dupStore`（疑似重复）同构——避免各页各查一次导致互相矛盾。
 *
 * 口径：只统计**待办项数**（`dashboard_todos` 的各阶段计数），不是"凭证张数"。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { api, type DashboardTodos } from "../api/client";

export const useInboxStore = defineStore("inbox", () => {
  const todos = ref<DashboardTodos | null>(null);

  async function refresh() {
    try {
      todos.value = await api.dashboardTodos();
    } catch {
      // 待办是辅助信息：失败保持上一次的值，不打扰主流程
    }
  }

  /** 待处理项数（侧栏徽标）：各阶段有几件事等着做 */
  const pendingCount = computed(() => {
    const t = todos.value;
    if (!t) return 0;
    return (
      (t.ocrPending > 0 ? 1 : 0) +
      (t.staleDrafts > 0 ? 1 : 0) +
      (t.amountMissing > 0 ? 1 : 0) +
      (t.dupPairs > 0 ? 1 : 0) +
      (t.unlockedMonths > 0 ? 1 : 0) +
      (t.backupStale ? 1 : 0)
    );
  });

  return { todos, pendingCount, refresh };
});
