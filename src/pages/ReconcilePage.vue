<script setup lang="ts">
/**
 * 对账页（v0.27）：台账 CSV 与条目库比对。
 *
 * B1 为静态原型；B4 接 `ledger_check` / `ledger_check_export` 与网页上传。
 * 本页是对账的唯一入口；工作台不再放「台账比对」行。
 */
import { ref } from "vue";
import FluentIcon from "../components/FluentIcon.vue";
import { toast } from "../stores/toast";

interface Diff {
  kind: string;
  ref: string;
  detail: string;
  amount: string;
}

const fileName = ref("");
const compared = ref(true);

const diffs: Diff[] = [
  {
    kind: "金额不一致",
    ref: "L-2026-0912",
    detail: "台账 ¥12,860.00 / 条目 ¥12,680.00",
    amount: "−¥180.00",
  },
  {
    kind: "条目缺失",
    ref: "L-2026-0931",
    detail: "台账有行，条目库没有对应记录",
    amount: "¥8,400.00",
  },
  {
    kind: "台账多余",
    ref: "2026-11-17-03",
    detail: "条目有金额，台账 CSV 没有这一行",
    amount: "¥3,200.00",
  },
];

function pick() {
  fileName.value = "项目台账_2026-11.csv";
  toast.info("CSV 选择将在 B4 接入");
}

function run() {
  compared.value = true;
  toast.info("重新比对将在 B4 接入");
}

function exportDiff() {
  toast.info("导出差异将在 B4 接入");
}
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <h1 class="page-title">台账比对</h1>
        <p class="page-sub">把项目台账 CSV 与条目库对齐，只列差异，不改任何一边的数据。</p>
      </div>
      <div class="page-actions">
        <button class="btn" type="button" @click="pick">
          <FluentIcon name="folder" :size="14" />
          <span>{{ fileName || "选择台账 CSV" }}</span>
        </button>
        <button class="btn btn-primary" type="button" @click="run">
          <FluentIcon name="compare" :size="14" />
          <span>重新比对</span>
        </button>
      </div>
    </div>

    <section v-if="compared" class="wb-kpis" aria-label="比对摘要">
      <div class="wb-kpi">
        <span class="wb-kpi-label">台账行</span>
        <span class="wb-kpi-value">128</span>
        <span class="wb-kpi-delta">2026-11 月</span>
      </div>
      <div class="wb-kpi">
        <span class="wb-kpi-label">条目库</span>
        <span class="wb-kpi-value">126</span>
        <span class="wb-kpi-delta">含 2 条未挂台账</span>
      </div>
      <div class="wb-kpi">
        <span class="wb-kpi-label">差异</span>
        <span class="wb-kpi-value">3</span>
        <span class="wb-kpi-delta">金额 / 缺失 / 多余</span>
      </div>
      <div class="wb-kpi">
        <span class="wb-kpi-label">上次比对</span>
        <span class="wb-kpi-value wb-kpi-value--text">今天 09:41</span>
        <span class="wb-kpi-delta">{{ fileName || "项目台账_2026-11.csv" }}</span>
      </div>
    </section>

    <section v-if="compared" class="data-panel">
      <div class="data-toolbar">
        <div class="data-chips">
          <span class="chip is-active">全部差异 3</span>
          <span class="chip">金额 1</span>
          <span class="chip">缺失 1</span>
          <span class="chip">多余 1</span>
        </div>
        <div class="data-actions">
          <button class="btn" type="button" @click="exportDiff">
            <FluentIcon name="download" :size="14" />
            <span>导出差异</span>
          </button>
        </div>
      </div>
      <table class="data-table">
        <thead>
          <tr>
            <th scope="col">差异类型</th>
            <th scope="col">台账行 / 条目</th>
            <th scope="col">说明</th>
            <th class="data-amount" scope="col">差额</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="d in diffs" :key="d.ref" class="data-row">
            <td>
              <span class="data-status" data-status="draft">{{ d.kind }}</span>
            </td>
            <td class="data-no">{{ d.ref }}</td>
            <td>{{ d.detail }}</td>
            <td class="data-amount">{{ d.amount }}</td>
          </tr>
        </tbody>
      </table>
    </section>

    <section v-else class="wb-card">
      <p class="wb-empty">还没有比对。先选择一份台账 CSV，再点「重新比对」。</p>
    </section>
  </div>
</template>

<style scoped>
.wb-kpi-value--text {
  font-size: var(--fs-title);
  font-weight: 600;
}
</style>
