<script setup lang="ts">
/**
 * 洞察（资料库 → 洞察）—— v0.18：统计视图从首页移到资料库。
 *
 * 为什么单独成页：重构后首页（收件箱）只回答"现在该做什么"；"这个月花了多少、
 * 钱花在谁身上"属于**查数据**的场景，放在资料库这一组里、与凭证列表并列，
 * 才符合"流程在收件箱、数据在资料库"的分工。
 *
 * 口径说明（设计正本 §3）：本页各块**各自标注口径**；统一时间窗控制器在与面板
 * 的内部窗口合并后再上线（面板当前自带窗口：月度 6 个月 / 排行全历史前 10）。
 */
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import TrendPanel from "../components/dashboard/TrendPanel.vue";
import TopListsPanel from "../components/dashboard/TopListsPanel.vue";
import ReconcilePanel from "../components/dashboard/ReconcilePanel.vue";
import FluentIcon from "../components/FluentIcon.vue";
import { api, type ExportCoverage, type ReconcileReport, type StorageUsage } from "../api/client";
import { toast, toastError } from "../stores/toast";
import { fmtSize } from "../composables/format";

const router = useRouter();
const reconcileReport = ref<ReconcileReport | null>(null);
const reconcileBusy = ref(false);

// 导出覆盖度与存储占用（批次 5 新增的台账口径；界面必须标注口径）
const coverage = ref<ExportCoverage | null>(null);
const storage = ref<StorageUsage | null>(null);
const storageBytesBusy = ref(false);

async function loadLedgers() {
  try {
    [coverage.value, storage.value] = await Promise.all([api.exportCoverage(), api.storageUsage(false)]);
  } catch (e) {
    toastError(e);
  }
}

/** 增强版体积需逐文件 stat（DB 里刻意不落该列）→ 用户主动点开才算 */
async function loadEnhancedBytes() {
  storageBytesBusy.value = true;
  try {
    storage.value = await api.storageUsage(true);
  } catch (e) {
    toastError(e);
  } finally {
    storageBytesBusy.value = false;
  }
}

onMounted(loadLedgers);

async function runReconcile() {
  if (reconcileBusy.value) return;
  reconcileBusy.value = true;
  try {
    const report = await api.pickAndReconcile();
    if (!report) return; // 用户取消
    reconcileReport.value = report;
    toast.info(
      `对账完成：✅ 匹配 ${report.matched.length} · 🔵 账单无凭证 ${report.billOnly.length} · 🔴 凭证无账单 ${report.voucherOnly.length}`,
    );
  } catch (e) {
    toastError(e);
  } finally {
    reconcileBusy.value = false;
  }
}
</script>

<template>
  <div class="insights">
    <header class="head">
      <h2>洞察</h2>
      <span class="scope">趋势与排行各块自带窗口（统一口径控制器随后端透视接口一并接入）</span>
      <span class="flex-1"></span>
      <button class="btn" :disabled="reconcileBusy" title="选账单 CSV 与库内凭证对账（只读，不改数据）" @click="runReconcile">
        <FluentIcon name="chart" :size="14" /> {{ reconcileBusy ? "对账中…" : "账单对账" }}
      </button>
      <button class="btn" @click="router.push('/vouchers')">
        <FluentIcon name="list" :size="14" /> 去看凭证
      </button>
    </header>

    <!-- 台账：导出覆盖度 + 存储占用（批次 5 的只读聚合；每块标注口径） -->
    <div class="ledgers">
      <section class="panel">
        <header class="sh">
          <h3>导出覆盖度</h3>
          <span class="scope">全库 · 含归档</span>
        </header>
        <template v-if="coverage">
          <div class="kv">
            <div><dt>全部凭证</dt><dd>{{ coverage.total }}</dd></div>
            <div><dt>已导出过</dt><dd>{{ coverage.exported }}</dd></div>
            <div><dt>从未导出</dt><dd :class="{ warn: coverage.neverExported > 0 }">{{ coverage.neverExported }}</dd></div>
          </div>
          <ul v-if="coverage.byFormat.length" class="formats">
            <li v-for="f in coverage.byFormat" :key="f.format">
              <span class="tag tag-info">{{ f.format.toUpperCase() }}</span>
              <span class="muted">{{ f.batches }} 批 · {{ f.vouchers }} 张</span>
            </li>
          </ul>
          <button v-if="coverage.neverExported > 0" class="btn" @click="router.push({ path: '/vouchers', query: { exported: 'never' } })">
            查看从未导出的 {{ coverage.neverExported }} 张
          </button>
        </template>
        <p v-else class="muted">加载中…</p>
      </section>

      <section class="panel">
        <header class="sh">
          <h3>存储占用</h3>
          <span class="scope">原图体积来自数据库；增强版体积需按需计算</span>
        </header>
        <template v-if="storage">
          <div class="kv">
            <div><dt>原图</dt><dd>{{ storage.originalCount }} 张 · {{ fmtSize(storage.originalBytes) }}</dd></div>
            <div>
              <dt>增强版</dt>
              <dd>
                {{ storage.enhancedCount }} 张
                <template v-if="storage.enhancedBytes != null"> · {{ fmtSize(storage.enhancedBytes) }}</template>
                <button v-else class="link" :disabled="storageBytesBusy" @click="loadEnhancedBytes()">
                  {{ storageBytesBusy ? "计算中…" : "（计算体积）" }}
                </button>
              </dd>
            </div>
            <div><dt>增强来源</dt><dd>扫描件 {{ storage.enhancedScan }} · 截图 {{ storage.enhancedScreenshot }} · 未识别 {{ storage.enhancedUnknown }}</dd></div>
            <div><dt>OCR 缓存</dt><dd>{{ storage.ocrCacheCount }} 条</dd></div>
          </div>
        </template>
        <p v-else class="muted">加载中…</p>
      </section>
    </div>

    <TrendPanel />
    <TopListsPanel />
    <ReconcilePanel v-if="reconcileReport" :report="reconcileReport" @close="reconcileReport = null" />
  </div>
</template>

<style scoped>
.insights { display: flex; flex-direction: column; gap: var(--s-4); }
.head { display: flex; align-items: center; gap: var(--s-3); flex-wrap: wrap; }
.head h2 { font: var(--type-title); color: var(--label-primary); margin: 0; font-size: calc(20px * var(--font-scale)); }
.scope { font: var(--type-caption); color: var(--label-tertiary); }
.flex-1 { flex: 1; }
.ledgers { display: grid; grid-template-columns: repeat(auto-fit, minmax(340px, 1fr)); gap: var(--s-3); }
.panel { padding: var(--s-3); border: 1px solid var(--separator); border-radius: var(--radius-panel); background: var(--bg-content); display: flex; flex-direction: column; gap: var(--s-2); }
.sh { display: flex; align-items: baseline; gap: var(--s-2); }
.sh h3 { font: var(--type-title); color: var(--label-primary); margin: 0; }
.kv { display: flex; flex-direction: column; gap: 6px; }
.kv > div { display: grid; grid-template-columns: 84px 1fr; gap: var(--s-2); align-items: baseline; }
.kv dt { color: var(--label-tertiary); font: var(--type-caption); }
.kv dd { margin: 0; font: var(--type-body); color: var(--label-primary); font-variant-numeric: tabular-nums; }
.kv dd.warn { color: var(--warning); font-weight: 600; }
.formats { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.formats li { display: flex; align-items: center; gap: var(--s-2); }
.muted { color: var(--label-tertiary); font: var(--type-caption); }
.link { border: none; background: none; color: var(--accent); font: var(--type-caption); cursor: default; }
.link:hover { text-decoration: underline; }
</style>
