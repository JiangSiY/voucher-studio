<script setup lang="ts">
/** 导出历史：批次列表 + 打开 PDF + 批次反查凭证（v0.9 动线：批次 → 凭证明细不断链） */
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { api, type ExportBatch, type Voucher } from "../api/client";
import EmptyState from "../components/EmptyState.vue";
import FluentIcon from "../components/FluentIcon.vue";
import { STATUS_META, fmtAmount, fmtDate } from "../composables/format";
import { toastError } from "../stores/toast";

const router = useRouter();
const batches = ref<ExportBatch[]>([]);

/** 批次格式（后端 EXP- 前缀：EXP-X=Excel / EXP-C=CSV，无字母或 EXP-D=PDF）。
    此前所有批次的操作按钮都写"打开 PDF"，Excel/CSV 批次点开才发现不是 PDF。 */
const FORMAT_LABEL: Record<string, string> = { pdf: "PDF", xlsx: "Excel", csv: "CSV" };
function formatLabel(b: ExportBatch): string {
  return FORMAT_LABEL[b.format ?? "pdf"] ?? "PDF";
}

async function load() {
  try {
    batches.value = await api.listExportBatches(100);
  } catch (e) {
    toastError(e);
  }
}

async function open(batch: ExportBatch) {
  try {
    await api.openPath(batch.pdfPath);
  } catch (e) {
    toastError(e);
  }
}

// ---------- v0.9 批次反查：查看该批次导出了哪些凭证 ----------

const detailVisible = ref(false);
const detailBatch = ref<ExportBatch | null>(null);
const detailVouchers = ref<Voucher[]>([]);
const detailBusy = ref(false);

async function showVouchers(batch: ExportBatch) {
  detailBatch.value = batch;
  detailVisible.value = true;
  detailBusy.value = true;
  try {
    detailVouchers.value = await api.exportBatchVouchers(batch.id);
  } catch (e) {
    toastError(e);
    detailVouchers.value = [];
  } finally {
    detailBusy.value = false;
  }
}

/** 凭证行 → 资料库并聚焦该张（v0.19：编辑在右栏；批次详情是临时上下文，关掉再过去） */
function openVoucher(id: number) {
  detailVisible.value = false;
  router.push({ path: "/vouchers", query: { focus: String(id) } });
}

onMounted(load);
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-header"><h3>导出批次</h3></div>
    <div class="fluent-panel-body" style="padding: 0">
      <div class="dtable-wrap" v-if="batches.length">
        <table class="dtable">
          <thead>
            <tr>
              <th>批次号</th>
              <th>凭证数</th>
              <th>格式</th>
              <th>状态</th>
              <th>导出时间</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="b in batches" :key="b.id">
              <td><span class="mono">{{ b.batchNo }}</span></td>
              <td>{{ b.voucherCount }}</td>
              <td><span class="tag">{{ formatLabel(b) }}</span></td>
              <td>
                <!-- 批次状态与凭证状态是两套体系：这里用「已完成 / 失败」，避免与凭证的「已导出」混淆 -->
                <span class="tag" :class="b.status === 'done' ? 'tag-success' : 'tag-danger'">
                  {{ b.status === "done" ? "已完成" : "失败" }}
                </span>
              </td>
              <td>{{ fmtDate(b.createdAt) }}</td>
              <td>
                <div class="action-row">
                  <button class="icon-btn" title="查看批次包含的凭证" @click="showVouchers(b)"><FluentIcon name="list" :size="15" /></button>
                  <button class="icon-btn" :title="`打开 ${formatLabel(b)} 文件`" @click="open(b)"><FluentIcon name="file" :size="15" /></button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <EmptyState v-else icon="chart" title="还没有导出记录"
        desc="到「打印与导出」勾选凭证并生成导出文件（PDF / Excel / CSV）后，这里会显示每一次导出批次" />
    </div>
  </div>

  <!-- v0.9：批次凭证明细（点击行进编辑页核对） -->
  <div v-if="detailVisible" class="dialog-mask" @click.self="detailVisible = false">
    <div class="dialog" role="dialog" aria-modal="true" :aria-label="`批次 ${detailBatch?.batchNo} 的凭证`">
      <h3>批次 {{ detailBatch?.batchNo }}（{{ detailBatch?.voucherCount }} 张）</h3>
      <div class="dialog-body">
        <div v-if="detailBusy" style="text-align: center; padding: 24px; color: var(--text-3)">加载中…</div>
        <div v-else-if="detailVouchers.length" class="dtable-wrap" style="max-height: 50vh; overflow-y: auto">
          <table class="dtable">
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
              <tr v-for="v in detailVouchers" :key="v.id" style="cursor: pointer" title="在「全部凭证」中打开"
                @click="openVoucher(v.id)">
                <td><span class="mono">{{ v.voucherNo }}</span></td>
                <td>{{ v.voucherDate }}</td>
                <td style="max-width: 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">{{ v.description || "—" }}</td>
                <td class="num">{{ fmtAmount(v.amountCents) }}</td>
                <td><span class="tag" :class="STATUS_META[v.status].tag">{{ STATUS_META[v.status].label }}</span></td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else style="text-align: center; padding: 24px; color: var(--text-3)">
          批次内的凭证已被删除（导出文件不受影响）
        </div>
      </div>
      <div class="dialog-ops">
        <button class="btn btn-primary" @click="detailVisible = false">关闭</button>
      </div>
    </div>
  </div>
</template>
