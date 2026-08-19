<script setup lang="ts">
/** 导出历史：批次列表 + 打开 PDF */
import { onMounted, ref } from "vue";
import { api, type ExportBatch } from "../api/client";
import EmptyState from "../components/EmptyState.vue";
import { fmtDate } from "../composables/format";
import { toastError } from "../stores/toast";

const batches = ref<ExportBatch[]>([]);

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
              <th>状态</th>
              <th>导出时间</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="b in batches" :key="b.id">
              <td><span class="mono">{{ b.batchNo }}</span></td>
              <td>{{ b.voucherCount }}</td>
              <td>
                <span class="tag" :class="b.status === 'done' ? 'tag-success' : 'tag-danger'">
                  {{ b.status === "done" ? "成功" : "失败" }}
                </span>
              </td>
              <td>{{ fmtDate(b.createdAt) }}</td>
              <td>
                <div class="action-row">
                  <button class="icon-btn" title="打开 PDF" @click="open(b)">📄</button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <EmptyState v-else icon="📊" title="还没有导出记录"
        desc="到「打印中心」勾选凭证并生成 PDF 后，这里会显示每一次导出批次" />
    </div>
  </div>
</template>
