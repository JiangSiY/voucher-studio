<script setup lang="ts">
/**
 * 设置 → OCR：引擎选择（本地/云端）、本地模型状态、批量识别待处理附件。
 * draftCfg 为共享编辑缓冲（stores/settingsDraft，体检 #22 单向流改造）：
 * Tab 不再接收可变 cfg prop，而是读写共享 store 状态，保存由设置页整体提交。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { storeToRefs } from "pinia";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, errMsg, type LocalOcrStatus, type OcrHealth } from "../../api/client";
import FluentIcon from "../FluentIcon.vue";
import { toast, toastError } from "../../stores/toast";
import { useTaskStore } from "../../stores/tasks";
import { useSettingsDraft } from "../../stores/settingsDraft";

const { cfg: draftCfg } = storeToRefs(useSettingsDraft());

const localOcr = ref<LocalOcrStatus | null>(null);
const ocrHealth = ref<OcrHealth | null>(null);

// 批量识别待处理附件（列表页导入未跑 OCR 的、历史失败件等），成功后自动填单（只填空字段）
const taskStore = useTaskStore();
/** 正在跑的批量识别任务 id（进度事件据此上报；完成后置空） */
let ocrTaskId: number | null = null;
const ocrPendingBusy = ref(false);
const ocrProgress = ref<{ done: number; total: number } | null>(null);

const usesCloud = computed(() => draftCfg.value.ocr.engine !== "local");

/** 引擎标识 → 展示名 */
function engineLabel(engine: string | null | undefined): string {
  if (!engine) return "未知";
  if (engine === "local") return "本地 Apple Vision";
  if (engine === "deepseek") return "云端 DeepSeek";
  return "云端服务";
}

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
    ocrHealth.value = await api.ocrHealth();
  } catch (e) {
    toastError(e);
  }
  unlistenOcrProgress = await listen<{ done: number; total: number }>("ocr:progress", (e) => {
    if (ocrPendingBusy.value) ocrProgress.value = e.payload;
    if (ocrTaskId !== null)
      taskStore.progress(ocrTaskId, { done: e.payload.done, total: e.payload.total });
  });
});

onUnmounted(() => {
  unlistenOcrProgress?.();
});
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-body">
      <div class="settings-head">
        <div>
          <div class="settings-caption-strong">启用 OCR</div>
          <div class="settings-caption">关闭后导入图片不再自动识别文字</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="draftCfg.ocr.enabled" />
          <span class="slider"></span>
        </label>
      </div>
      <div class="field">
        <label>识别引擎</label>
        <select v-model="draftCfg.ocr.engine" class="input settings-narrow-lg">
          <option value="auto">云端优先，失败自动用本地（推荐）</option>
          <option value="cloud" :disabled="!localOcr?.available">仅云端（DeepSeek 视觉）</option>
          <option value="local" :disabled="!localOcr?.available">
            仅本地（离线 · Apple Vision）
          </option>
        </select>
        <div class="desc">
          云端能直接给出日期/金额/对方，识别最准；本地用 macOS 原生 Vision，完全离线、无需下载模型。
          <span v-if="localOcr">（本地：{{ localOcr.engineName }} · {{ localOcr.note }}）</span>
        </div>
      </div>
      <div v-if="ocrHealth?.lastEngine" class="callout settings-note">
        <span class="ci"><FluentIcon name="sparkle" :size="18" /></span>
        <div class="ct">
          <strong>最近一次识别：{{ engineLabel(ocrHealth.lastEngine) }}</strong>
          <template v-if="ocrHealth.lastCloudError">
            <br />但云端未用上：{{ ocrHealth.lastCloudError }}（已回退本地）
          </template>
        </div>
      </div>
      <template v-if="usesCloud">
        <div class="field">
          <label>云端服务</label>
          <select v-model="draftCfg.ocr.provider" class="input settings-narrow-lg">
            <option value="deepseek">DeepSeek 视觉（推荐）</option>
            <option value="custom">自建 HTTP 服务（兼容旧协议）</option>
          </select>
          <div class="desc">
            识别时图片会上传到云端服务（单张约 0.003 元），API Key 在「密钥管理 → 云端 OCR API
            Key」设置。
          </div>
        </div>
        <div class="field">
          <label>API 地址</label>
          <input
            v-model="draftCfg.ocr.apiUrl"
            class="input mono"
            :placeholder="
              draftCfg.ocr.provider === 'deepseek'
                ? 'https://api.deepseek.com/chat/completions'
                : 'https://api.example.com/ocr'
            "
          />
          <div class="desc">
            <template v-if="draftCfg.ocr.provider === 'deepseek'">
              OpenAI 兼容接口，需 https；字段留空即用默认地址。
            </template>
            <template v-else>
              POST JSON：{image_base64, model}，返回 {text}（须 https；回环地址允许 http）。
            </template>
          </div>
        </div>
        <div class="field">
          <label>模型名</label>
          <input
            v-model="draftCfg.ocr.model"
            class="input"
            :placeholder="
              draftCfg.ocr.provider === 'deepseek' ? 'deepseek-flash' : '如：voucher-ocr-v1'
            "
          />
        </div>
      </template>
      <div class="field settings-section">
        <label>批量识别</label>
        <div class="settings-inline">
          <button class="btn" :disabled="ocrPendingBusy" @click="runOcrPending">
            {{ ocrPendingBusy ? "识别中…" : "识别全部待处理" }}
          </button>
          <span class="settings-caption">
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
  transition: width var(--motion-normal) var(--motion-curve);
}

.ocr-progress-text {
  font-size: var(--fs-caption);
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
</style>
