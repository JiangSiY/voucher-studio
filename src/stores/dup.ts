/**
 * 疑似重复处置的**跨页共享状态**（侧栏徽标 / 仪表盘面板 / 编辑页提示共用一份）。
 *
 * 为什么不各自取：侧栏徽标、仪表盘面板、保存提示都需要"还剩多少要处理"，
 * 三处各查一次会出现互相不一致的瞬间（处置后这一处刷新了、那一处还挂着旧数字），
 * 正是用户抱怨的"一直挂着提示"的成因之一。统一由本 store 持有，
 * 处置命令（后端发 `dup:changed`）与页面动作都只调 `refresh()`。
 *
 * `notified` 是**本次会话内已提示过的对**（不持久化）：后端已经排除了已判定的对，
 * 这里再挡一层，避免同一张凭证反复保存时同一条提示反复弹。
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import { api, type PendingSummary } from "../api/client";

export const useDupStore = defineStore("dup", () => {
  const pending = ref<PendingSummary>({ groups: 0, pairs: 0 });

  /** 本次会话已提示过的对（key 与后端 normalize_pair 同口径：小的在前） */
  const notified = new Set<string>();

  /** 拉取待处理统计（辅助信息，失败静默——不影响其余界面） */
  async function refresh() {
    try {
      pending.value = await api.duplicatePendingSummary();
    } catch {
      // 徽标/面板是辅助能力：取不到就维持上次数字，不打扰主流程
    }
  }

  /** 该对本次会话是否已提示过 */
  function wasNotified(key: string): boolean {
    return notified.has(key);
  }

  /** 记下"这一对已提示过" */
  function markNotified(key: string) {
    notified.add(key);
  }

  /** 仅测试用：清空会话记忆 */
  function resetNotified() {
    notified.clear();
  }

  return { pending, refresh, wasNotified, markNotified, resetNotified };
});
