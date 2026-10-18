<script setup lang="ts">
/**
 * 设置 → OCR：引擎选择（本地/云端）、本地模型状态、批量识别待处理附件。
 * draftCfg 为共享编辑缓冲（stores/settingsDraft，体检 #22 单向流改造）：
 * Tab 不再接收可变 cfg prop，而是读写共享 store 状态，保存由设置页整体提交。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { storeToRefs } from "pinia";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, errMsg, type LocalOcrStatus } from "../../api/client";
import FluentIcon from "../FluentIcon.vue";
import { toast, toastError } from "../../stores/toast";
import { useTaskStore } from "../../stores/tasks";
import { useSettingsDraft } from "../../stores/settingsDraft";

const { cfg: draftCfg } = storeToRefs(useSettingsDraft());

const localOcr = ref<LocalOcrStatus | null>(null);

// 批量识别待处理附件（列表页导入未跑 OCR 的、历史失败件等），成功后自动填单（只填空字段）
const taskStore = useTaskStore();
/** 正在跑的批量识别任务 id（进度事件据此上报；完成后置空） */
let ocrTaskId: number | null = null;
const ocrPendingBusy = ref(false);
const ocrProgress = ref<{ done: number; total: number } | null>(null);

const ocrProgressPct = computed(() => {
  const p = ocrProgress.value;
  if (!p || p.total === 0) return 0;
  return Math.round((p.done / p.total) * 100);
});

async function runOcrPending() {
  if (ocrPendingBusy.value) return;
  ocrPendingBusy.value = true;
  ocrProgress.value = null;
  // 批量识别可能跑几分钟：任务中心兜住"换页也看得见"，页内进度条照旧（在本页时不离开视线）
  const taskId = taskStore.start("ocr", "批量识别待识别附件");
  ocrTaskId = taskId;
  try {
    const n = await api.runOcrPending(500);
    taskStore.finish(taskId, { detail: n > 0 ? `已识别 ${n} 张并自动填单` : "没有待识别的附件" });
    if (n > 0) {
      toast.success(`批量识别完成：${n} 张附件已识别并自动填单`);
    } else {
      toast.info?.("没有待识别的附件");
    }
  } catch (e) {
    taskStore.fail(taskId, errMsg(e));
    toastError(e);
  } finally {
    ocrPendingBusy.value = false;
    ocrProgress.value = null;
    ocrTaskId = null;
  }
}

// 批量识别进度（后端逐张推送 ocr:progress 事件）
let unlistenOcrProgress: UnlistenFn | null = null;

onMounted(async () => {
  try {
    localOcr.value = await api.getLocalOcrStatus();
  } catch (e) {
    toastError(e);
  }
  unlistenOcrProgress = await listen<{ done: number; total: number }>("ocr:progress", (e) => {
    if (ocrPendingBusy.value) ocrProgress.value = e.payload;
    if (ocrTaskId !== null) taskStore.progress(ocrTaskId, { done: e.payload.done, total: e.payload.total });
  });
});

onUnmounted(() => {
  unlistenOcrProgress?.();
});
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-body">
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 16px">
        <div>
          <div style="font-size: var(--fs-caption); font-weight: 600">启用 OCR</div>
          <div style="font-size: var(--fs-caption); color: var(--text-3)">关闭后导入图片不再自动识别文字</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="draftCfg.ocr.enabled" />
          <span class="slider"></span>
        </label>
      </div>
      <div class="field">
        <label>识别引擎</label>
        <select v-model="draftCfg.ocr.engine" class="input" style="max-width: 280px">
          <option value="local" :disabled="!localOcr?.installed">本地（离线 · 推荐）</option>
          <option value="cloud">云端（HTTP API）</option>
        </select>
        <div class="desc">本地引擎完全离线运行，数据不出本机（PP-OCRv5）。</div>
      </div>
      <div v-if="localOcr?.installed" class="callout success" style="margin: 0 0 16px">
        <span class="ci"><FluentIcon name="checkCircle" :size="18" /></span>
        <div class="ct">
          <strong>本地模型已安装</strong>
          <span v-if="localOcr.files.length">（{{ localOcr.files.length }} 个文件，约 {{ (localOcr.totalSize / 1048576).toFixed(1) }} MB）</span>
          ：RapidOCR PP-OCRv5 检测 + 方向分类 + 识别，首次识别时加载约 0.2 秒。
        </div>
      </div>
      <div v-else class="callout warning" style="margin: 0 0 16px">
        <span class="ci"><FluentIcon name="warning" :size="18" /></span>
        <div class="ct">
          <strong>本地模型未安装</strong>——将模型文件（ch_PP-OCRv5_mobile_det.onnx 等 3 个）放入
          <code>{{ localOcr?.modelsDir || "数据目录/models" }}</code> 后重启应用即可启用。
        </div>
      </div>
      <template v-if="draftCfg.ocr.engine === 'cloud'">
        <div class="field">
          <label>API 地址</label>
          <input v-model="draftCfg.ocr.apiUrl" class="input mono" placeholder="https://api.example.com/ocr" />
          <div class="desc">POST JSON：{image_base64, model}，返回 {text}；API Key 在「密钥管理」页设置。</div>
        </div>
        <div class="field">
          <label>模型名（可选）</label>
          <input v-model="draftCfg.ocr.model" class="input" placeholder="如：voucher-ocr-v1" />
        </div>
      </template>
      <div class="field" style="border-top: 1px solid var(--stroke); padding-top: 16px">
        <label>批量识别</label>
        <div style="display: flex; gap: 8px; align-items: center">
          <button class="btn" :disabled="ocrPendingBusy" @click="runOcrPending">
            {{ ocrPendingBusy ? "识别中…" : "识别全部待处理" }}
          </button>
          <span style="font-size: var(--fs-caption); color: var(--text-3)">
            处理所有待识别附件（列表页导入、识别失败件），识别后自动填单（只填空字段）
          </span>
        </div>
        <div v-if="ocrProgress" class="ocr-progress">
          <div class="ocr-progress-track">
            <div class="ocr-progress-fill" :style="{ width: `${ocrProgressPct}%` }"></div>
          </div>
          <span class="ocr-progress-text">
            {{ ocrProgress.done }} / {{ ocrProgress.total }}（{{ ocrProgressPct }}%）
          </span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 批量识别进度条（Fluent 细条样式） */
.ocr-progress {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
}

.ocr-progress-track {
  flex: 1;
  height: 4px;
  border-radius: var(--radius-small);
  background: var(--stroke);
  overflow: hidden;
}

.ocr-progress-fill {
  height: 100%;
  border-radius: var(--radius-small);
  background: var(--accent);
  transition: width 0.2s ease;
}

.ocr-progress-text {
  font-size: var(--fs-caption);
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
</style>
