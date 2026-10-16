import { computed, ref } from "vue";
import { defineStore } from "pinia";

/**
 * 全局任务中心（规范 §1.5：「任务中心（全局浮层，可最小化）」）。
 *
 * 为什么要有：导入 / 批量识别 / 重建增强 / 生成 PDF / 日报 / 备份这些**长任务**原先各自把进度
 * 写在页面里（设置页的两个进度条、打印中心的一个忙按钮），用户一换页就再也看不到它跑到哪、
 * 是不是失败了。这里把「正在进行的活儿」提成全局状态，浮层在任何页面都可见，并且可最小化。
 *
 * 与 toast 的分工：toast 是**一次性结果**（成功/失败，几秒后消失），任务中心是**过程**
 * （还在跑、跑到几成、能不能取消/跳转），两者不互相替代。
 */

export type TaskState = "running" | "done" | "failed";

/** 完成后的去向（如「打开导出目录」「开始核对」）——比 toast 活得久，用户随时可点 */
export interface TaskAction {
  label: string;
  onClick: () => void;
}

export interface TaskItem {
  id: number;
  /** 分类（import / export / ocr / enhance / backup），仅用于图标与文案分组 */
  kind: string;
  title: string;
  detail: string;
  /** 已完成量；-1 表示后端不报进度（进度条走不确定态） */
  done: number;
  /** 总量；-1 表示未知 */
  total: number;
  state: TaskState;
  error: string;
  startedAt: number;
  finishedAt: number | null;
  action?: TaskAction;
}

let nextId = 1;

/** 已完成条目最多留 3 条：留着能看到"刚做完什么"，但不会把浮层堆满 */
const MAX_FINISHED = 3;

export const useTaskStore = defineStore("tasks", () => {
  const items = ref<TaskItem[]>([]);
  /** 最小化：只留一行胶囊（用户要求"可最小化"；有新任务时自动展开） */
  const collapsed = ref(false);

  const running = computed(() => items.value.filter((t) => t.state === "running"));
  const finished = computed(() => items.value.filter((t) => t.state !== "running"));
  const runningCount = computed(() => running.value.length);
  /** 折叠态的胶囊文案：优先报"还剩几件在跑"，全做完时报"已完成" */
  const summary = computed(() => {
    if (runningCount.value > 0) return `${runningCount.value} 个任务进行中`;
    if (finished.value.some((t) => t.state === "failed")) return "有任务失败";
    return "任务已完成";
  });

  /** 只保留最近的若干条已完成条目（运行中的永不丢弃） */
  function trim() {
    const done = items.value.filter((t) => t.state !== "running");
    if (done.length <= MAX_FINISHED) return;
    const drop = new Set(done.slice(0, done.length - MAX_FINISHED).map((t) => t.id));
    items.value = items.value.filter((t) => !drop.has(t.id));
  }

  /** 开一个任务；`total = -1` 表示进度未知（不确定态进度条） */
  function start(kind: string, title: string, total = -1): number {
    const id = nextId++;
    items.value.push({
      id,
      kind,
      title,
      detail: "",
      done: 0,
      total,
      state: "running",
      error: "",
      startedAt: Date.now(),
      finishedAt: null,
    });
    // 有新活儿就把浮层展开——最小化时悄悄开跑等于没有任务中心
    collapsed.value = false;
    trim();
    return id;
  }

  /** 进度上报（后端不报就只更新 detail） */
  function progress(id: number, patch: { done?: number; total?: number; detail?: string }) {
    const t = items.value.find((x) => x.id === id);
    if (!t || t.state !== "running") return;
    if (typeof patch.done === "number") t.done = patch.done;
    if (typeof patch.total === "number") t.total = patch.total;
    if (typeof patch.detail === "string") t.detail = patch.detail;
  }

  function settle(id: number, state: "done" | "failed", patch: { detail?: string; error?: string; action?: TaskAction } = {}) {
    const t = items.value.find((x) => x.id === id);
    if (!t) return;
    t.state = state;
    t.finishedAt = Date.now();
    if (typeof patch.detail === "string") t.detail = patch.detail;
    if (typeof patch.error === "string") t.error = patch.error;
    if (patch.action) t.action = patch.action;
    trim();
  }

  const finish = (id: number, patch: { detail?: string; action?: TaskAction } = {}) => settle(id, "done", patch);
  const fail = (id: number, error: string) => settle(id, "failed", { error });

  function dismiss(id: number) {
    items.value = items.value.filter((t) => t.id !== id);
  }

  function clearFinished() {
    items.value = items.value.filter((t) => t.state === "running");
  }

  function toggleCollapsed() {
    collapsed.value = !collapsed.value;
  }

  return {
    items,
    collapsed,
    running,
    finished,
    runningCount,
    summary,
    start,
    progress,
    finish,
    fail,
    dismiss,
    clearFinished,
    toggleCollapsed,
  };
});
