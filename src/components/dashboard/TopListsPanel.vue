<script setup lang="ts">
/**
 * 仪表盘 → Top 对方账户排行 + 标签分类统计（双栏，按金额降序，全部历史）。
 * 自加载数据并监听 import:done 刷新；行点击下钻列表页关键词筛选。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { safeListen } from "../../composables/safeListen";
import { useRouter } from "vue-router";
import { api, type CounterpartyRow, type TagRow } from "../../api/client";
import { fmtAmount } from "../../composables/format";
import { toastError } from "../../stores/toast";

const router = useRouter();

// ---------- v0.7 R-1：Top 对方账户排行（按金额降序） ----------

const CP_TOP = 10;
const counterparties = ref<CounterpartyRow[]>([]);

const cpMaxAmount = computed(() =>
  Math.max(1, ...counterparties.value.map((c) => c.amountCents)),
);

/** 条形宽度：按最大金额归一化（最小 2% 保证可见） */
function cpBarWidth(c: CounterpartyRow): string {
  return `${Math.max(2, (c.amountCents / cpMaxAmount.value) * 100)}%`;
}

// ---------- 功能深化 3：标签分类统计（按金额降序，全部历史） ----------

const TAG_TOP = 10;
const tagRows = ref<TagRow[]>([]);

const tagMaxAmount = computed(() =>
  Math.max(1, ...tagRows.value.map((t) => t.amountCents)),
);

function tagBarWidth(t: TagRow): string {
  return `${Math.max(2, (t.amountCents / tagMaxAmount.value) * 100)}%`;
}

/** 标签显示名：空串 = 未分类 */
function tagLabel(t: TagRow): string {
  return t.tag || "未分类";
}

/** Top 对方账户行 → 列表页按对方账户关键词筛选 */
function gotoCounterparty(name: string) {
  router.push({ path: "/vouchers", query: { keyword: name } });
}

/** 标签分类行 → 列表页按标签关键词筛选（「未分类」无法用关键词表达，不可点） */
function gotoTag(tag: string) {
  if (!tag) return;
  router.push({ path: "/vouchers", query: { keyword: tag } });
}

async function load() {
  try {
    counterparties.value = await api.topCounterparties(0, CP_TOP);
    tagRows.value = await api.tagStats(0, TAG_TOP);
  } catch (e) {
    toastError(e);
  }
}

let unlistenImportDone: UnlistenFn | null = null;

onMounted(async () => {
  load();
  unlistenImportDone = await safeListen("import:done", () => load());
});

onUnmounted(() => {
  unlistenImportDone?.();
});
</script>

<template>
  <div class="report-grid" v-if="counterparties.length || tagRows.length">
    <div class="fluent-panel" v-if="counterparties.length">
      <div class="fluent-panel-header">
        <h3>Top 对方账户（按金额 · 全部历史）</h3>
        <span class="dist-sum">共 {{ counterparties.length }} 家 · 金额为凭证金额合计</span>
      </div>
      <div class="fluent-panel-body">
        <div v-for="(c, i) in counterparties" :key="c.name" class="cp-row cp-click" style="cursor: pointer"
          :title="`点击查看「${c.name}」的 ${c.count} 张凭证`" @click="gotoCounterparty(c.name)">
          <span class="cp-rank" :class="{ 'cp-rank-top': i < 3 }">{{ i + 1 }}</span>
          <span class="cp-name" :title="c.name">{{ c.name }}</span>
          <div class="cp-bar-wrap">
            <div class="cp-bar" :style="{ width: cpBarWidth(c) }"
              :title="`${c.name} · ¥${fmtAmount(c.amountCents)} · ${c.count} 张`" />
          </div>
          <span class="cp-amount num" :title="`¥${fmtAmount(c.amountCents)}`">¥{{ fmtAmount(c.amountCents) }}</span>
          <span class="cp-count">{{ c.count }} 张</span>
        </div>
      </div>
    </div>

    <!-- 功能深化 3：标签分类统计（未分类固定末尾；一张多标签在每个标签下各计一次） -->
    <div class="fluent-panel" v-if="tagRows.length">
      <div class="fluent-panel-header">
        <h3>标签分类（按金额 · 全部历史）</h3>
        <span class="dist-sum">共 {{ tagRows.length }} 类 · 含未分类</span>
      </div>
      <div class="fluent-panel-body">
        <div v-for="(t, i) in tagRows" :key="t.tag || '__untagged__'" class="cp-row"
          :class="{ 'cp-click': t.tag }"
          :style="t.tag ? 'cursor: pointer' : ''"
          :title="t.tag ? `点击查看「${tagLabel(t)}」的 ${t.count} 张凭证` : '没有任何标签的凭证'"
          @click="gotoTag(t.tag)">
          <span class="cp-rank" :class="{ 'cp-rank-top': i < 3 && t.tag }">{{ t.tag ? i + 1 : "—" }}</span>
          <span class="cp-name" :class="{ 'cp-name-muted': !t.tag }" :title="t.tag ? t.tag : '没有任何标签的凭证'">
            {{ t.tag ? `# ${tagLabel(t)}` : tagLabel(t) }}
          </span>
          <div class="cp-bar-wrap">
            <div class="cp-bar" :class="{ 'cp-bar-muted': !t.tag }" :style="{ width: tagBarWidth(t) }"
              :title="`${tagLabel(t)} · ¥${fmtAmount(t.amountCents)} · ${t.count} 张`" />
          </div>
          <span class="cp-amount num" :title="`¥${fmtAmount(t.amountCents)}`">¥{{ fmtAmount(t.amountCents) }}</span>
          <span class="cp-count">{{ t.count }} 张</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* .report-grid 已收敛到 src/styles/base.css《窗口缩放适配》一节（scoped 特异性高于
   base.css，会让全局断点失效，故移出组件）。 */

.dist-sum {
  font-size: 12px;
  color: var(--text-disabled);
}

/* v0.9 下钻：可点击行的 hover 反馈 */
.cp-click:hover {
  background: var(--accent-soft);
  border-radius: 4px;
}

/* v0.7 R-1：Top 对方账户排行行（排名 / 名称 / 条形 / 金额 / 张数） */
.cp-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 5px 0;
}

.cp-rank {
  flex: none;
  width: 22px;
  height: 22px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  border-radius: 50%;
  background: var(--stroke);
  color: var(--text-2);
  font-variant-numeric: tabular-nums;
}

.cp-rank-top {
  background: var(--accent-soft);
  color: var(--accent);
  font-weight: 600;
}

.cp-name {
  flex: none;
  width: 140px;
  font-size: 13px;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cp-bar-wrap {
  flex: 1;
  min-width: 0;
}

.cp-bar {
  height: 10px;
  border-radius: 5px;
  background: var(--accent);
  transition: opacity 0.15s ease;
}

.cp-bar:hover {
  opacity: 0.8;
}

.cp-amount {
  flex: none;
  width: 110px;
  font-size: 13px;
  color: var(--text);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.cp-count {
  flex: none;
  width: 52px;
  font-size: 12px;
  color: var(--text-4);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

/* 标签面板中「未分类」行的弱化样式 */
.cp-name-muted {
  color: var(--text-3);
}

.cp-bar-muted {
  background: var(--text-disabled);
}
</style>
