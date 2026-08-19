<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { api, type BackupInfo, type Voucher, type VoucherStats } from "../api/client";
import StatCard from "../components/StatCard.vue";
import { STATUS_META, fmtAmount, fmtDate } from "../composables/format";
import { useConfigStore } from "../stores/config";
import { toast, toastError } from "../stores/toast";

const router = useRouter();
const configStore = useConfigStore();

const stats = ref<VoucherStats | null>(null);
const recent = ref<Voucher[]>([]);
const backups = ref<BackupInfo[]>([]);
const busy = ref(false);

async function load() {
  try {
    stats.value = await api.voucherStats();
    const list = await api.listVouchers({ page: 1, pageSize: 6 });
    recent.value = list.items;
    backups.value = await api.listBackups();
  } catch (e) {
    toastError(e);
  }
}

async function backupNow() {
  busy.value = true;
  try {
    const info = await api.runBackup();
    toast.success(`备份完成：${info.fileName}`);
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

onMounted(load);
</script>

<template>
  <div class="stat-grid" v-if="stats">
    <StatCard label="凭证总数" :value="stats.total" :delta="`本月新增 ${stats.thisMonth}`" />
    <StatCard label="草稿" :value="stats.draft" delta="待整理" />
    <StatCard label="已复核" :value="stats.reviewed" delta="可归档打印" />
    <StatCard label="已导出" :value="stats.exported" delta="已生成 PDF" />
    <StatCard label="附件图片" :value="stats.attachments" delta="含 OCR 识别" />
    <StatCard label="本地备份" :value="backups.length" :delta="backups[0] ? `最近 ${fmtDate(backups[0].createdAt)}` : '尚无备份'" />
  </div>

  <div class="callout info">
    <span class="ci">💡</span>
    <div class="ct">
      <strong>快捷入口：</strong>把凭证图片直接拖进「凭证列表」页面即可自动建卡入库；
      数据全部保存在本机（{{ configStore.appInfo?.dataDir }}），
      <span v-if="backups.length === 0" style="color: var(--warning)">建议先做一次备份。</span>
      <span v-else>最近备份 {{ fmtDate(backups[0].createdAt) }}。</span>
    </div>
  </div>

  <div class="fluent-panel">
    <div class="fluent-panel-header">
      <h3>最近凭证</h3>
      <div style="display: flex; gap: 8px">
        <button class="btn btn-sm" :disabled="busy" @click="backupNow()">💾 立即备份</button>
        <button class="btn btn-sm" @click="router.push('/vouchers/new')">＋ 新建凭证</button>
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
          <div class="es-icon">🗂</div>
          <div class="es-title">还没有凭证</div>
          <div class="es-desc">拖入图片、或点「新建凭证」开始整理</div>
          <button class="btn btn-primary" @click="router.push('/vouchers/new')">＋ 新建凭证</button>
        </div>
      </div>
    </div>
  </div>
</template>
