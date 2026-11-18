<script setup lang="ts">
/**
 * 数据页（v0.27）：数据库视图 + 行内展开。
 *
 * B1 为静态原型：列、视图、展开形态先定稿；B2 接真实 `list_vouchers`、
 * `VoucherEditor`、创建/导入/导出。本页是全站唯一的新建/导入/打印导出入口。
 */
import { computed, ref } from "vue";
import FluentIcon from "../components/FluentIcon.vue";
import { toast } from "../stores/toast";
import { useModalRoute } from "../composables/useModalRoute";

type View = "all" | "missing" | "legacy" | "booked" | "draft" | "month" | "batches";

interface Row {
  id: number;
  no: string;
  date: string;
  customer: string;
  description: string;
  category: string;
  amount: number | null;
  status: "draft" | "reviewed" | "exported" | "archived";
  booked: boolean;
  thumb: boolean;
}

const views: { key: View; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "missing", label: "待补凭证" },
  { key: "legacy", label: "历史未补" },
  { key: "booked", label: "待入账" },
  { key: "draft", label: "待核对" },
  { key: "month", label: "本月" },
  { key: "batches", label: "导入批次" },
];

const activeView = ref<View>("all");
const expanded = ref<number | null>(1);
const { open } = useModalRoute();

const rows: Row[] = [
  {
    id: 1,
    no: "2026-11-18-01",
    date: "2026-11-18",
    customer: "凯诗蓝德",
    description: "9 月并网款",
    category: "组件",
    amount: 128_600_00,
    status: "draft",
    booked: false,
    thumb: true,
  },
  {
    id: 2,
    no: "2026-11-18-02",
    date: "2026-11-18",
    customer: "中天光伏",
    description: "逆变器尾款",
    category: "逆变器",
    amount: 86_400_00,
    status: "reviewed",
    booked: true,
    thumb: true,
  },
  {
    id: 3,
    no: "2026-11-17-03",
    date: "2026-11-17",
    customer: "正泰安能",
    description: "支架补差",
    category: "支架",
    amount: null,
    status: "draft",
    booked: false,
    thumb: false,
  },
  {
    id: 4,
    no: "2026-11-15-04",
    date: "2026-11-15",
    customer: "晴天科技",
    description: "运维服务费",
    category: "服务",
    amount: 41_200_00,
    status: "exported",
    booked: true,
    thumb: true,
  },
];

const filtered = computed(() => {
  switch (activeView.value) {
    case "missing":
      return rows.filter((r) => !r.thumb);
    case "legacy":
      return [];
    case "booked":
      return rows.filter((r) => !r.booked);
    case "draft":
      return rows.filter((r) => r.status === "draft");
    case "batches":
      return [];
    default:
      return rows;
  }
});

const statusLabel: Record<Row["status"], string> = {
  draft: "草稿",
  reviewed: "已复核",
  exported: "已导出",
  archived: "已归档",
};

function money(cents: number | null): string {
  if (cents === null) return "—";
  return `¥${(cents / 100).toLocaleString("zh-CN", { maximumFractionDigits: 0 })}`;
}

function toggle(id: number) {
  expanded.value = expanded.value === id ? null : id;
}

function later(what: string) {
  toast.info(`${what}将在 B2 接入`);
}
</script>

<template>
  <div class="page page--wide">
    <div class="data-toolbar">
      <div class="data-chips" role="group" aria-label="视图">
        <button
          v-for="v in views"
          :key="v.key"
          class="chip"
          :class="{ 'is-active': activeView === v.key }"
          type="button"
          :aria-pressed="activeView === v.key"
          @click="activeView = v.key"
        >
          {{ v.label }}
        </button>
      </div>

      <div class="data-actions">
        <button class="btn btn-primary" type="button" @click="later('新建条目')">
          <FluentIcon name="plus" :size="14" />
          <span>新建条目</span>
        </button>
        <button class="btn" type="button" @click="later('导入凭证')">
          <FluentIcon name="upload" :size="14" />
          <span>导入</span>
        </button>
        <button class="btn" type="button" @click="open('export')">
          <FluentIcon name="print" :size="14" />
          <span>打印导出</span>
        </button>
        <button
          class="btn btn-icon"
          type="button"
          aria-label="筛选"
          title="筛选"
          @click="later('筛选')"
        >
          <FluentIcon name="compare" :size="16" />
        </button>
        <button
          class="btn btn-icon"
          type="button"
          aria-label="列设置"
          title="列设置"
          @click="later('列设置')"
        >
          <FluentIcon name="list" :size="16" />
        </button>
      </div>
    </div>

    <div class="data-panel">
      <table class="data-table">
        <thead>
          <tr>
            <th class="data-check" scope="col"><span class="sr-only">选择</span></th>
            <th class="col-hide-sm" scope="col">编号</th>
            <th scope="col">日期</th>
            <th scope="col">客户</th>
            <th class="col-hide-sm" scope="col">摘要</th>
            <th class="col-hide-sm" scope="col">类别</th>
            <th class="data-amount" scope="col">金额</th>
            <th class="col-hide-sm" scope="col">凭证</th>
            <th scope="col">状态</th>
            <th class="col-hide-sm" scope="col">入账</th>
          </tr>
        </thead>
        <tbody>
          <template v-for="r in filtered" :key="r.id">
            <tr
              class="data-row"
              :class="{ 'is-open': expanded === r.id }"
              tabindex="0"
              @click="toggle(r.id)"
              @keydown.enter="toggle(r.id)"
            >
              <td class="data-check" @click.stop>
                <input type="checkbox" :aria-label="`选择 ${r.no}`" />
              </td>
              <td class="data-no col-hide-sm">{{ r.no }}</td>
              <td>{{ r.date }}</td>
              <td>{{ r.customer }}</td>
              <td class="data-desc col-hide-sm">{{ r.description }}</td>
              <td class="col-hide-sm">{{ r.category }}</td>
              <td class="data-amount">{{ money(r.amount) }}</td>
              <td class="col-hide-sm">
                <span v-if="r.thumb" class="data-thumb" aria-label="有凭证">图</span>
                <span v-else class="data-thumb data-thumb--missing" aria-label="缺凭证">缺</span>
              </td>
              <td>
                <span class="data-status" :data-status="r.status">{{ statusLabel[r.status] }}</span>
              </td>
              <td class="col-hide-sm">
                <span
                  class="data-booked"
                  :class="{ 'is-on': r.booked }"
                  :aria-label="r.booked ? '已入账' : '未入账'"
                >
                  {{ r.booked ? "已入" : "未入" }}
                </span>
              </td>
            </tr>
            <tr v-if="expanded === r.id" class="data-expand">
              <td colspan="10">
                <div class="data-expand-inner">
                  <div class="data-expand-fields">
                    <label class="data-field">
                      <span>客户</span>
                      <input class="input" :value="r.customer" />
                    </label>
                    <label class="data-field">
                      <span>摘要</span>
                      <input class="input" :value="r.description" />
                    </label>
                    <label class="data-field">
                      <span>金额</span>
                      <input class="input" :value="money(r.amount)" />
                    </label>
                    <label class="data-field">
                      <span>类别</span>
                      <input class="input" :value="r.category" />
                    </label>
                    <p class="data-expand-hint">行内编辑接 `VoucherEditor`（B2 接入）。</p>
                  </div>
                  <div class="data-expand-side">
                    <div class="data-expand-thumb">
                      {{ r.thumb ? "凭证预览" : "凭证必填：补图后才能入账/导出" }}
                    </div>
                  </div>
                </div>
              </td>
            </tr>
          </template>
          <tr v-if="!filtered.length">
            <td colspan="10">
              <p class="wb-empty">
                {{ activeView === "legacy" ? "没有历史未补凭证。" : "当前视图没有条目。" }}
              </p>
            </td>
          </tr>
        </tbody>
      </table>
      <div class="data-pager">
        <span>共 {{ filtered.length }} 条 · 每页 50</span>
        <span class="data-pager-hint">点击行展开编辑</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.data-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.data-no {
  font-variant-numeric: tabular-nums;
  color: var(--text-2);
  white-space: nowrap;
}

.data-desc {
  max-width: 260px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.data-booked {
  color: var(--text-3);
  font-size: var(--fs-caption);
}

.data-booked.is-on {
  color: var(--success);
}

.data-expand-thumb {
  display: grid;
  place-items: center;
  min-height: 168px;
  border: 1px dashed var(--separator-strong);
  border-radius: var(--radius-control);
  color: var(--text-3);
  font-size: var(--fs-caption);
  text-align: center;
  padding: 16px;
}

.data-expand-hint {
  margin: 0;
  color: var(--text-3);
  font-size: var(--fs-caption);
}
</style>
