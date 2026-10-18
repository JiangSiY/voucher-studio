<script setup lang="ts">
/**
 * 待处理（收件箱）—— v0.18 收件箱驱动重构的首页。
 *
 * 与旧「仪表盘」的根本区别（用户明确要求"重构，不是在旧布局上增删改"）：
 * - **问题不是"这个月花了多少"，而是"现在该做什么"**：页面就是一条流水线——
 *   待识别 → 待核对 → 待打印/导出 → 疑似重复处置 —— 每个阶段一张卡，能就地推进，
 *   不必先想"这功能在哪个页面"，也不用来回跳页再重新筛选。
 * - 统计（趋势/排行）**降级为资料库的视图**，不再占据首屏；首屏留给待办。
 * - 阶段卡的数字与侧栏徽标同源（`inboxStore`），处理完即刷新。
 *
 * 数据来源：`dashboard_todos`（待办/健康度，轻量计数）+ `voucher_stats`（全库口径的
 * 阶段计数）+ `duplicate_pending_count`（疑似重复轻量计数）。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { safeListen } from "../composables/safeListen";
import {
  api,
  type ExportBatch,
  type ExportCoverage,
  type Voucher,
  type VoucherStats,
} from "../api/client";
import FluentIcon from "../components/FluentIcon.vue";
import { STATUS_META, fmtDate } from "../composables/format";
import { useInboxStore } from "../stores/inbox";
import { toast, toastError } from "../stores/toast";

const router = useRouter();
const inbox = useInboxStore();

const stats = ref<VoucherStats | null>(null);
const recentDrafts = ref<Voucher[]>([]);
const batches = ref<ExportBatch[]>([]);
/** 导出覆盖度（「从未导出」口径）——「待打印/导出」待办用它，而不是「全库已复核张数」 */
const coverage = ref<ExportCoverage | null>(null);
const busy = ref<"ocr" | null>(null);

/**
 * **待办清单（单一来源）**：把流水线阶段与数据质量项合成一份列表，
 * **count 为 0 的一律不渲染**（用户要求："以待办显示，完成之后就隐藏，提高空间利用率"）。
 * 顺序固定（先流程、后质量），便于形成肌肉记忆。
 */
const todoItems = computed(() => {
  const t = inbox.todos;
  const s = stats.value;
  type Item = {
    key: string;
    icon: string;
    title: string;
    count: number;
    unit: string;
    hint: string;
    action: { label: string; run: () => void; disabled?: boolean };
    secondary?: { label: string; run: () => void };
  };
  const all: Item[] = [
    {
      key: "ocr",
      icon: "sparkle",
      title: "待识别",
      count: t?.ocrPending ?? 0,
      unit: "张",
      hint: "识别后可自动填单（日期 / 金额 / 对方 / 摘要）",
      action: { label: busy.value === "ocr" ? "识别中…" : "开始识别", run: runOcr, disabled: busy.value === "ocr" },
      // 精确下钻：与上面计数**同口径**（计数含 pending + failed，下钻必须带两者，
      // 否则"待识别 3 张"点进去只看到 1 张——2026-09-27 审计 P1-1）
      secondary: {
        label: "查看待识别",
        run: () => router.push({ path: "/vouchers", query: { ocrStatus: "pending,failed" } }),
      },
    },
    {
      key: "review",
      icon: "checkCircle",
      title: "待核对",
      count: s?.draft ?? 0,
      unit: "张草稿",
      hint: recentDrafts.value.length
        ? `最近一张：${recentDrafts.value[0].voucherNo}（点击右侧"继续核对"进入核对序列）`
        : "草稿尚未复核",
      action: { label: "继续核对", run: () => router.push({ path: "/vouchers", query: { status: "draft" } }) },
    },
    {
      key: "print",
      icon: "print",
      title: "待打印 / 导出",
      // 口径：**从未导出**（export_items 无记录，含已归档）——与洞察「导出覆盖度」同口径。
      // 此前用 stats.reviewed（全库已复核张数）冒充「待导出」，两者不是一回事（审计 P1-3）。
      count: coverage.value?.neverExported ?? 0,
      unit: "张",
      hint: "从未导出过 PDF / 表格（点进去可批量补出）",
      action: {
        label: "去看清单",
        run: () => router.push({ path: "/vouchers", query: { exported: "never" } }),
      },
    },
    {
      key: "dup",
      icon: "duplicate",
      title: "疑似重复待处置",
      count: t?.dupPairs ?? 0,
      unit: "对",
      hint: t?.dupHighPairs ? `其中 ${t.dupHighPairs} 对证据较强（同图或同对方同摘要）` : "同日期同金额，需人工判定",
      action: { label: "去核查", run: () => router.push("/duplicates") },
    },
    {
      key: "amount",
      icon: "edit",
      title: "未填金额",
      count: t?.amountMissing ?? 0,
      unit: "张",
      hint: "金额为空（不参与统计与重复判定）",
      // 精确下钻：只列"金额为空"的凭证（批次 5 的 amountEmpty 维度）
      action: { label: "去补填", run: () => router.push({ path: "/vouchers", query: { amountEmpty: "1" } }) },
    },
    {
      key: "stale",
      icon: "info",
      title: `超过 ${t?.staleDraftDays ?? 7} 天没动的草稿`,
      count: t?.staleDrafts ?? 0,
      unit: "张",
      hint: "长期挂着的草稿，建议复核或删除",
      // 精确下钻：与计数同口径（"超过 N 天没动"必须带 staleDays，否则列出全部草稿——审计 P1-2）
      action: {
        label: "去看看",
        run: () =>
          router.push({
            path: "/vouchers",
            query: { status: "draft", staleDays: String(t?.staleDraftDays ?? 7) },
          }),
      },
    },
    {
      key: "unlock",
      icon: "lock",
      title: "未封账的往月",
      count: t?.unlockedMonths ?? 0,
      unit: "个月",
      hint: "有凭证但未锁定会计期间",
      action: { label: "去封账", run: () => router.push("/settings?tab=naming") },
    },
    {
      key: "backup",
      icon: "save",
      title: "该备份了",
      count: t?.backupStale ? 1 : 0,
      unit: "项",
      hint: t?.latestBackup ? `最近备份 ${fmtDate(t.latestBackup.modifiedAt)}` : "尚无任何备份",
      action: { label: "去备份", run: () => router.push("/restore") },
    },
  ];
  return all.filter((i) => i.count > 0);
});

async function runOcr() {
  busy.value = "ocr";
  try {
    const done = await api.runOcrPending(50);
    toast.success(done ? `已识别 ${done} 张` : "没有可识别的附件");
    await load();
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = null;
  }
}

function openVoucher(id: number) {
  // v0.19：编辑在资料库右栏（不再有独立编辑页）——带 focus 深链过去并聚焦该张
  router.push({ path: "/vouchers", query: { focus: String(id) } });
}

async function load() {
  await Promise.all([
    inbox.refresh(),
    (async () => {
      try {
        const [s, drafts, bs, cov] = await Promise.all([
          api.voucherStats(),
          api.listVouchers({ status: "draft", page: 1, pageSize: 3 }),
          api.listExportBatches(3),
          api.exportCoverage(), // 全窗口：与「从未导出」待办同口径
        ]);
        stats.value = s;
        recentDrafts.value = drafts.items;
        batches.value = bs.slice(0, 3);
        coverage.value = cov;
      } catch (e) {
        toastError(e);
      }
    })(),
  ]);
}

let unlistenImport: UnlistenFn | null = null;
let unlistenDup: UnlistenFn | null = null;
let unlistenBackup: UnlistenFn | null = null;

onMounted(async () => {
  await load();
  unlistenImport = await safeListen("import:done", () => load());
  unlistenDup = await safeListen("dup:changed", () => load());
  unlistenBackup = await safeListen("backup:done", () => load());
});

onUnmounted(() => {
  unlistenImport?.();
  unlistenDup?.();
  unlistenBackup?.();
});
</script>

<template>
  <div class="inbox">
    <!-- 一句话现状：今天有几件事 -->
    <header class="inbox-head">
      <h2>待处理</h2>
      <span class="scope">
        <template v-if="inbox.pendingCount">有 {{ inbox.pendingCount }} 件事等着办</template>
        <template v-else>暂时没有待办</template>
      </span>
      <span class="flex-1"></span>
      <button class="btn" title="立即备份" @click="router.push('/restore')">
        <FluentIcon name="save" :size="14" /> 备份
      </button>
      <button class="btn btn-primary" @click="router.push('/vouchers/new')">
        <FluentIcon name="plus" :size="14" /> 新建凭证
      </button>
    </header>

    <!-- 待办清单：count 为 0 的项不渲染；全部清零时整块消失（空间让给资料） -->
    <section v-if="todoItems.length" class="todos">
      <ul>
        <li v-for="i in todoItems" :key="i.key" class="todo-row">
          <button class="todo" :disabled="i.action.disabled" @click="i.action.run()">
            <span class="todo-icon"><FluentIcon :name="i.icon" :size="17" /></span>
            <span class="todo-title">{{ i.title }}</span>
            <span class="todo-count">{{ i.count }}<small>{{ i.unit }}</small></span>
            <span class="todo-hint">{{ i.hint }}</span>
            <span class="flex-1"></span>
            <span class="todo-action">{{ i.action.label }}</span>
            <FluentIcon name="chevronRight" :size="13" />
          </button>
          <!-- 次要动作独立成按钮：原先写在主按钮内部（span @click），Tab 落不到、
               screen reader 也只当它是主按钮的文字；而且主按钮 disabled（如「识别中…」）
               时浏览器会吞掉内部点击 → 唯一的下钻入口在识别期间完全点不动 -->
          <button v-if="i.secondary" class="todo-secondary" @click="i.secondary.run()">
            {{ i.secondary.label }}
          </button>
        </li>
      </ul>
    </section>

    <!-- 全部处理完：只留一条细横幅，不占版面 -->
    <div v-else-if="inbox.todos" class="all-clear">
      <FluentIcon name="checkCircle" :size="17" />
      <span>待办都清空了 —— 没有待识别的图片、待核对的草稿、待打印的凭证，也没有未处置的疑似重复。</span>
      <span class="flex-1"></span>
      <button class="link" @click="router.push('/vouchers')">打开「全部凭证」</button>
    </div>

    <!-- 最近活动（次要信息，放最后） -->
    <div class="activity">
      <section class="panel">
        <header class="section-head">
          <h3>最近凭证</h3>
          <span class="flex-1"></span>
          <button class="link" @click="router.push('/vouchers')">全部</button>
        </header>
        <ul v-if="recentDrafts.length" class="mini-list">
          <li v-for="v in recentDrafts" :key="v.id" @click="openVoucher(v.id)">
            <span class="mono">{{ v.voucherNo }}</span>
            <span class="muted">{{ v.description || "（无摘要）" }}</span>
            <span class="flex-1"></span>
            <span class="tag" :class="STATUS_META[v.status].tag">{{ STATUS_META[v.status].label }}</span>
          </li>
        </ul>
        <p v-else class="empty-hint">没有草稿。</p>
      </section>

      <section class="panel">
        <header class="section-head">
          <h3>最近导出</h3>
          <span class="flex-1"></span>
          <button class="link" @click="router.push('/exports')">导出历史</button>
        </header>
        <ul v-if="batches.length" class="mini-list">
          <li v-for="b in batches" :key="b.id">
            <span class="mono">{{ b.batchNo }}</span>
            <span class="tag tag-info">{{ b.format?.toUpperCase() ?? "PDF" }}</span>
            <span class="muted">{{ b.voucherCount }} 张</span>
            <span class="flex-1"></span>
            <span class="muted">{{ fmtDate(b.createdAt) }}</span>
          </li>
        </ul>
        <p v-else class="empty-hint">还没有导出批次。</p>
      </section>
    </div>
  </div>
</template>

<style scoped>
/* 收件箱：首屏只讲"该做什么"；样式全部使用设计正本 §1 的语义令牌 */
.inbox { display: flex; flex-direction: column; gap: var(--s-4); }

.inbox-head { display: flex; align-items: center; gap: var(--s-3); flex-wrap: wrap; }
.inbox-head h2 { font: var(--type-title); color: var(--label-primary); margin: 0; font-size: var(--fs-display); }
.flex-1 { flex: 1; }

/* 待办行：一行一件，高 44px 左右——比卡片网格省下大量纵向空间。
   行 = 主按钮（整行动作）+ 可选次要按钮；背景与描边挂在行上，视觉上仍是一整行。 */
.todos ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
.todo-row {
  display: flex; align-items: center;
  border: 1px solid transparent;
  border-radius: var(--radius-control);
  background: var(--bg-content);
}
.todo-row:hover { background: var(--bg-selection); border-color: var(--separator); }
.todo {
  display: flex; align-items: center; gap: var(--s-3); flex: 1; min-width: 0;
  padding: 10px var(--s-3);
  border: 0;
  background: transparent;
  text-align: left; cursor: default;
}
.todo:disabled { opacity: 0.6; }
.todo-icon { color: var(--label-secondary); display: inline-flex; }
.todo-title { font: var(--type-body); font-weight: 600; color: var(--label-primary); white-space: nowrap; }
.todo-count { font: var(--type-body); font-weight: 600; color: var(--label-primary); font-variant-numeric: tabular-nums; white-space: nowrap; }
.todo-count small { margin-left: 3px; font: var(--type-caption); font-weight: 400; color: var(--label-tertiary); }
.todo-hint { font: var(--type-caption); color: var(--label-tertiary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
.todo-action { font: var(--type-caption); color: var(--accent); white-space: nowrap; }
.todo-secondary {
  flex-shrink: 0; margin-right: var(--s-3); padding: 2px 6px;
  border: 0; border-radius: var(--radius-small); background: transparent;
  font: var(--type-caption); color: var(--label-tertiary); white-space: nowrap; cursor: default;
}
.todo-secondary:hover { color: var(--accent); text-decoration: underline; }

.all-clear {
  display: flex; align-items: center; gap: var(--s-2);
  padding: var(--s-2) var(--s-3);
  border: 1px solid var(--separator);
  border-radius: var(--radius-control);
  background: var(--bg-content);
  color: var(--label-secondary);
  font: var(--type-body);
}

.section-head h3 { font: var(--type-title); color: var(--label-primary); margin: 0; }

.activity { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: var(--s-3); }
.panel { padding: var(--s-3); border: 1px solid var(--separator); border-radius: var(--radius-panel); background: var(--bg-content); }
.section-head { display: flex; align-items: baseline; gap: var(--s-2); }
.mini-list { list-style: none; margin: var(--s-2) 0 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
.mini-list li { display: flex; align-items: center; gap: var(--s-2); padding: 6px var(--s-2); border-radius: var(--radius-small); font: var(--type-body); cursor: default; }
.mini-list li:hover { background: var(--bg-selection); }
.muted { color: var(--label-tertiary); font: var(--type-caption); }
.link { border: none; background: none; color: var(--accent); font: var(--type-caption); cursor: default; }
.link:hover { text-decoration: underline; }
.empty-hint { color: var(--label-tertiary); font: var(--type-body); margin: var(--s-2) 0; }
</style>
