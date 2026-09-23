<script setup lang="ts">
/**
 * 仪表盘骨架：统计卡 / 快捷入口 / 疑似重复提示 / 最近凭证（含备份·对账·新建操作）。
 * 趋势与状态分布在 TrendPanel、账户与标签 TOP 在 TopListsPanel、
 * 对账结果在 ReconcilePanel（均自加载/按事件刷新）。
 */
import { onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { safeListen } from "../composables/safeListen";
import { api, type BackupInfo, type DuplicateGroup, type ReconcileReport, type Voucher, type VoucherStats } from "../api/client";
import StatCard from "../components/StatCard.vue";
import FluentIcon from "../components/FluentIcon.vue";
import TrendPanel from "../components/dashboard/TrendPanel.vue";
import TopListsPanel from "../components/dashboard/TopListsPanel.vue";
import ReconcilePanel from "../components/dashboard/ReconcilePanel.vue";
import { STATUS_META, fmtAmount, fmtDate } from "../composables/format";
import { useConfigStore } from "../stores/config";
import { toast, toastError } from "../stores/toast";

const router = useRouter();
const configStore = useConfigStore();

const stats = ref<VoucherStats | null>(null);
const recent = ref<Voucher[]>([]);
const backups = ref<BackupInfo[]>([]);
const busy = ref(false);

// ---------- v0.13：账单对账（微信/支付宝 CSV ↔ 凭证；只读匹配 + 报告） ----------

const reconcileReport = ref<ReconcileReport | null>(null);
const reconcileBusy = ref(false);

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

function closeReconcile() {
  reconcileReport.value = null;
}

// ---------- v0.10：疑似重复扫描（同日期 + 同金额；仅提示，不阻断任何操作） ----------

const duplicates = ref<DuplicateGroup[]>([]);

async function load() {
  try {
    stats.value = await api.voucherStats();
    const list = await api.listVouchers({ page: 1, pageSize: 6 });
    recent.value = list.items;
    backups.value = await api.listBackups();
    // 疑似重复是辅助检查，失败静默（不影响仪表盘其他数据展示）
    try {
      duplicates.value = await api.scanDuplicates();
    } catch {
      duplicates.value = [];
    }
  } catch (e) {
    toastError(e);
  }
}

async function backupNow() {
  busy.value = true;
  try {
    const info = await api.runBackup();
    // 副本告警（WebDAV/异盘失败）：本地主备份成功但 3-2-1 不完整，必须显式提示
    if (info.warnings?.length) {
      toast.warning(`备份完成：${info.fileName}，但副本存在问题：${info.warnings.join("；")}`);
    } else {
      toast.success(`备份完成：${info.fileName}`);
    }
    backups.value = await api.listBackups();
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = false;
  }
}

function statusOf(v: Voucher) {
  return STATUS_META[v.status];
}

// 拖放导入在任意页面全局生效，概览停留期间入库也需实时刷新；
// 自动备份在后台完成 → 刷新备份卡片与最近备份时间
let unlistenImportDone: UnlistenFn | null = null;
let unlistenBackupDone: UnlistenFn | null = null;

onMounted(async () => {
  load();
  unlistenImportDone = await safeListen("import:done", () => load());
  unlistenBackupDone = await safeListen("backup:done", () => load());
});

onUnmounted(() => {
  unlistenImportDone?.();
  unlistenBackupDone?.();
});
</script>

<template>
  <div class="stat-grid" v-if="stats">
    <!-- v0.7 R-2：状态卡片点击联动——跳转凭证列表并按状态预筛选 -->
    <div class="stat-click" title="查看全部凭证" @click="router.push('/vouchers')">
      <StatCard label="凭证总数" :value="stats.total" :delta="`本月新增 ${stats.thisMonth}`" />
    </div>
    <div class="stat-click" title="查看草稿凭证" @click="router.push('/vouchers?status=draft')">
      <StatCard label="草稿" :value="stats.draft" delta="待整理" />
    </div>
    <div class="stat-click" title="查看已复核凭证" @click="router.push('/vouchers?status=reviewed')">
      <StatCard label="已复核" :value="stats.reviewed" delta="可归档打印" />
    </div>
    <div class="stat-click" title="查看已导出凭证" @click="router.push('/vouchers?status=exported')">
      <StatCard label="已导出" :value="stats.exported" delta="已生成 PDF" />
    </div>
    <StatCard label="附件图片" :value="stats.attachments" delta="含 OCR 识别" />
    <StatCard label="本地备份" :value="backups.length" :delta="backups[0] ? `最近 ${fmtDate(backups[0].createdAt)}` : '尚无备份'" />
  </div>

  <div class="callout info">
    <span class="ci"><FluentIcon name="bulb" :size="18" /></span>
    <div class="ct">
      <strong>快捷入口：</strong>把凭证图片直接拖进「凭证列表」页面即可自动建卡入库；
      数据全部保存在本机（{{ configStore.appInfo?.dataDir }}），
      <span v-if="backups.length === 0" style="color: var(--warning)">建议先做一次备份。</span>
      <span v-else>最近备份 {{ fmtDate(backups[0].createdAt) }}。</span>
    </div>
  </div>

  <!-- v0.10：疑似重复面板（同日期 + 同金额；点击凭证号跳编辑页人工比对） -->
  <div class="callout warning" v-if="duplicates.length">
    <span class="ci"><FluentIcon name="warning" :size="18" /></span>
    <div class="ct">
      <strong>发现 {{ duplicates.length }} 组疑似重复凭证</strong>
      （同日期同金额；可能是同一票据多次拍照/扫描，也可能是同日两笔相同金额的正常业务——请人工核对）
      <div v-for="(g, gi) in duplicates.slice(0, 4)" :key="gi" class="dup-group">
        <span class="dup-key">{{ g.voucherDate }} · ¥{{ fmtAmount(g.amountCents) }} × {{ g.vouchers.length }} 张：</span>
        <template v-for="(v, vi) in g.vouchers" :key="v.id">
          <a class="dup-link" @click.prevent="router.push(`/vouchers/${v.id}`)" href="#"
            :title="`${v.description || '（无摘要）'} · ${v.counterparty || '（无对方）'}`">{{ v.voucherNo }}</a><span v-if="vi < g.vouchers.length - 1">、</span>
        </template>
      </div>
      <div v-if="duplicates.length > 4" style="font-size: 12px; margin-top: 4px">
        其余 {{ duplicates.length - 4 }} 组在保存凭证时也会逐张提示
      </div>
    </div>
  </div>

  <!-- v0.11 趋势/状态分布 + v0.7 账户/标签 TOP（自加载，import:done 刷新） -->
  <TrendPanel />
  <TopListsPanel />

  <div class="fluent-panel">
    <div class="fluent-panel-header">
      <h3>最近凭证</h3>
      <div style="display: flex; gap: 8px">
        <button class="btn btn-sm" :disabled="busy" @click="backupNow()"><FluentIcon v-if="!busy" name="save" :size="14" />立即备份</button>
        <!-- v0.13：账单对账（微信/支付宝 CSV ↔ 凭证差异盘点） -->
        <button class="btn btn-sm" :disabled="reconcileBusy" title="选择微信/支付宝账单 CSV，与库内凭证按日期+金额匹配"
          @click="runReconcile()"><FluentIcon v-if="!reconcileBusy" name="compare" :size="14" />{{ reconcileBusy ? "对账中…" : "账单对账" }}</button>
        <button class="btn btn-sm" @click="router.push('/vouchers/new')"><FluentIcon name="plus" :size="14" />新建凭证</button>
      </div>
    </div>
    <div class="fluent-panel-body" style="padding: 0">
      <div class="dtable-wrap">
        <table class="dtable" v-if="recent.length">
          <thead>
            <tr>
              <th>凭证编号</th>
              <th>日期</th>
              <th>摘要</th>
              <th style="text-align: right">金额</th>
              <th>状态</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="v in recent" :key="v.id" style="cursor: pointer"
              @click="router.push(`/vouchers/${v.id}`)">
              <td><span class="mono">{{ v.voucherNo }}</span></td>
              <td>{{ v.voucherDate }}</td>
              <td>{{ v.description || "—" }}</td>
              <td class="num">{{ fmtAmount(v.amountCents) }}</td>
              <td><span class="tag" :class="statusOf(v)?.tag">{{ statusOf(v)?.label }}</span></td>
            </tr>
          </tbody>
        </table>
        <div v-else class="empty-state" style="padding: 40px">
          <div class="es-icon"><FluentIcon name="folder" :size="42" /></div>
          <div class="es-title">还没有凭证</div>
          <div class="es-desc">拖入图片、或点「新建凭证」开始整理</div>
          <button class="btn btn-primary" @click="router.push('/vouchers/new')"><FluentIcon name="plus" :size="14" />新建凭证</button>
        </div>
      </div>
    </div>
  </div>

  <!-- v0.13：对账结果（报告由父页触发对账后传入） -->
  <ReconcilePanel v-if="reconcileReport" :report="reconcileReport" @close="closeReconcile" />
</template>

<style scoped>
/* v0.7 R-2：状态卡片可点击（hover 提示可跳转） */
.stat-click {
  cursor: pointer;
  border-radius: 8px;
  transition: transform 0.12s ease;
}

.stat-click:hover {
  transform: translateY(-1px);
}

/* v0.10 疑似重复面板：组行 + 可点凭证号 */
.dup-group {
  font-size: 13px;
  margin-top: 6px;
  line-height: 1.9;
}

.dup-key {
  color: var(--text-2);
}

.dup-link {
  color: var(--accent);
  cursor: pointer;
  text-decoration: underline;
  text-underline-offset: 2px;
}
</style>
