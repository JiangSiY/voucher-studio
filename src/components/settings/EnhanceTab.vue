<script setup lang="ts">
/**
 * 设置 → 扫描矫正：7 步流水线开关、存量重新增强、增强缓存管理
 * （D-2 摘要/命中率/释放策略/最近列表、D-8 来源分列、D-10 命中趋势）。
 * draftCfg 为共享编辑缓冲（stores/settingsDraft，体检 #22 单向流改造）：
 * Tab 不再接收可变 cfg prop，而是读写共享 store 状态，保存由设置页整体提交。
 */
import { onMounted, onUnmounted, ref } from "vue";
import { storeToRefs } from "pinia";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, errMsg, type AppConfig, type ClearStrategy, type ClearSummary, type EnhanceStatusV2, type HitTrend, type RecentEnhancedRow } from "../../api/client";
import FluentIcon from "../FluentIcon.vue";
import { toast, toastError } from "../../stores/toast";
import { useTaskStore } from "../../stores/tasks";
import { useSettingsDraft } from "../../stores/settingsDraft";

const { cfg: draftCfg } = storeToRefs(useSettingsDraft());

// 7 步流水线（v0.6.1 起去噪/锐化实装，全部可逐项开关）
const enhanceActiveSteps: { key: keyof AppConfig["enhance"]; label: string; desc: string }[] = [
  { key: "stepDeskew",    label: "1. 倾斜矫正",      desc: "Hough 直线 → 取最强水平线倾角 → 旋转校正" },
  { key: "stepDewarp",    label: "2. 透视矫正",      desc: "检测最大四边形 → 透视变换" },
  { key: "stepAutocrop",  label: "3. 自动裁边",      desc: "Otsu 自适应阈值 → 最大轮廓 bbox → 外扩 2% 安全边（v0.8 修复：旧版会裁掉票据边缘）" },
  { key: "stepDenoise",   label: "4. 中值去噪",      desc: "3×3 中值滤波，保边去噪点（默认关，建议搭配锐化开）" },
  { key: "stepSharpen",   label: "5. Unsharp 锐化",  desc: "高斯差分锐化文字边缘（默认关，过度锐化会放大噪声）" },
  { key: "stepGrayscale", label: "6. 灰度化",        desc: "BGR → GRAY，给 OCR 用" },
  { key: "stepBinarize",  label: "7. 自适应二值化",  desc: "高斯加权 → 31 窗口 / C=10" },
];

// D-2：增强缓存管理 v2
const enhanceStatV2 = ref<EnhanceStatusV2 | null>(null);
const recentEnhanced = ref<RecentEnhancedRow[]>([]);
const clearingCache = ref(false);

// D-10：命中趋势（近 N 天堆叠柱状图）
const hitTrend = ref<HitTrend | null>(null);
const trendDays = ref<7 | 14 | 30>(14);

// D-2 释放策略状态
const clearStrategy = ref<ClearStrategy>("all");
const clearThreshold = ref<number>(30); // older_than_days / smart=30 天 · larger_than_mb=10 MB
const pendingDeleteOne = ref<number | null>(null); // 单条释放：行 id（null = 不可见）
/** 「5 秒撤销」定时器句柄（审计 A-10）：句柄留存以便重置与卸载时清理 */
let undoOneTimer: ReturnType<typeof setTimeout> | null = null;

/** D-2 刷新：v2 状态 + 最近列表 + D-10 趋势 */
async function refreshEnhancePanel() {
  try {
    enhanceStatV2.value = await api.enhanceStatusV2();
    recentEnhanced.value = await api.recentEnhanced(20);
    hitTrend.value = await api.enhanceHitTrend(trendDays.value);
  } catch (e) {
    toastError(e);
  }
}

/** D-10 切换趋势区间（7 / 14 / 30 天） */
async function changeTrendDays(d: 7 | 14 | 30) {
  if (trendDays.value === d) return;
  trendDays.value = d;
  try {
    hitTrend.value = await api.enhanceHitTrend(d);
  } catch (e) {
    toastError(e);
  }
}

/** D-10 柱段高度百分比（相对区间峰值；全零区间返回 0） */
function barPct(n: number): number {
  const max = hitTrend.value?.maxTotal ?? 0;
  if (max === 0) return 0;
  return (n / max) * 100;
}

/** D-10 日标签只显示月-日 */
function shortDay(day: string): string {
  return day.slice(5);
}

/** D-2 释放（按策略） */
async function clearEnhanceV2() {
  if (clearingCache.value) return;
  const s = clearStrategy.value;
  const t = clearThreshold.value;
  const confirmMsg =
    s === "all"
      ? "确定要清除全部增强版缓存？下次导入会重新生成（首次识别变慢）。"
      : s === "older_than_days"
        ? `确定要清除「${t} 天前」的增强版缓存？其它不受影响。`
        : s === "smart"
          ? `智能清理：释放「零命中且超过 ${t} 天」的增强版（被用过的永不清理）。`
          : `确定要清除「大于 ${t} MB」的增强版缓存？其它不受影响。`;
  if (!confirm(confirmMsg)) return;
  clearingCache.value = true;
  try {
    const r: ClearSummary = await api.clearEnhanceCacheV2(s, t);
    toast.success(
      `已释放 ${r.deletedFiles} 个增强版（${(r.freedBytes / 1048576).toFixed(1)} MB）`,
    );
    await refreshEnhancePanel();
  } catch (e) {
    toastError(e);
  } finally {
    clearingCache.value = false;
  }
}

/** D-2 单条释放：第一次点变「再点确认」态，第二次真删 */
async function deleteOneEnhanced(row: RecentEnhancedRow) {
  if (pendingDeleteOne.value === row.attachmentId) {
    // 二次确认 → 真删
    try {
      await api.deleteEnhancedOne(row.attachmentId);
      toast.success(`已释放「${row.fileName}」的增强版`);
      pendingDeleteOne.value = null;
      await refreshEnhancePanel();
    } catch (e) {
      toastError(e);
    }
    return;
  }
  pendingDeleteOne.value = row.attachmentId;
  // 5 秒后自动撤销二次确认态（句柄留存：再次点击别的行要重置计时，卸载时要清掉——
  // 否则切走页面后回调仍写已卸载组件的 ref，审计 A-10）
  clearUndoOneTimer();
  undoOneTimer = setTimeout(() => {
    undoOneTimer = null;
    if (pendingDeleteOne.value === row.attachmentId) {
      pendingDeleteOne.value = null;
    }
  }, 5000);
}

function clearUndoOneTimer() {
  if (undoOneTimer !== null) {
    clearTimeout(undoOneTimer);
    undoOneTimer = null;
  }
}

// v0.8：存量重新增强（进度事件 enhance:progress 驱动按钮文案）
const taskStore = useTaskStore();
/** 正在跑的重建增强任务 id（进度事件据此上报） */
let enhanceTaskId: number | null = null;
const reEnhancing = ref(false);
const reEnhanceDone = ref(0);
const reEnhanceTotal = ref(0);
/** 增强版文件缺失数（来自启动健康报告；>0 时提示可一键重建） */
const missingEnhanced = ref(0);
let unlistenEnhanceProgress: UnlistenFn | null = null;

async function reEnhanceAll() {
  if (reEnhancing.value) return;
  if (!confirm("按新算法重新增强全部已有增强版图片？原图与凭证数据不变，仅重算增强版文件。")) return;
  reEnhancing.value = true;
  reEnhanceDone.value = 0;
  reEnhanceTotal.value = 0;
  enhanceTaskId = taskStore.start("enhance", "重建全部增强版");
  try {
    if (!unlistenEnhanceProgress) {
      unlistenEnhanceProgress = await listen<{ done: number; total: number }>(
        "enhance:progress",
        (e) => {
          reEnhanceDone.value = e.payload.done;
          reEnhanceTotal.value = e.payload.total;
          if (enhanceTaskId !== null)
            taskStore.progress(enhanceTaskId, { done: e.payload.done, total: e.payload.total });
        },
      );
    }
    const r = await api.reEnhanceAll();
    if (enhanceTaskId !== null) {
      taskStore.finish(enhanceTaskId, {
        detail: `${r.ok} 成功${r.failed ? ` · ${r.failed} 失败` : ""}`,
      });
      enhanceTaskId = null;
    }
    toast.success(`重新增强完成：${r.ok} 成功${r.failed ? ` · ${r.failed} 失败` : ""}`);
    refreshEnhancePanel();
  } catch (e) {
    if (enhanceTaskId !== null) {
      taskStore.fail(enhanceTaskId, errMsg(e));
      enhanceTaskId = null;
    }
    toastError(e);
  } finally {
    reEnhancing.value = false;
  }
}

onMounted(async () => {
  refreshEnhancePanel();
  // 启动健康报告里带"增强版缺失数"（备份不含增强版，恢复快照后必然出现）→ 给出重建入口提示
  void api
    .getRecoveryReport()
    .then((r) => (missingEnhanced.value = r.attachmentMissingEnhanced))
    .catch(() => (missingEnhanced.value = 0));
  unlistenEnhanceProgress = await listen<{ done: number; total: number }>(
    "enhance:progress",
    (e) => {
      reEnhanceDone.value = e.payload.done;
      reEnhanceTotal.value = e.payload.total;
    },
  );
});

onUnmounted(() => {
  clearUndoOneTimer();
  unlistenEnhanceProgress?.();
});
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-body">
      <div class="callout info" style="margin: 0 0 16px">
        <span class="ci"><FluentIcon name="sparkle" :size="18" /></span>
        <div class="ct">
          <strong>文档扫描矫正</strong>：对导入的图片自动跑 7 步算法（Hough 倾斜矫正 → 透视矫正 → 自动裁边 → 中值去噪 → Unsharp 锐化 → 灰度化 → 自适应二值化），
          矫正后的增强版用于 OCR（更准）和查看器（更易读）；任一步失败自动跳过、回退到上一步输出。
          去噪 / 锐化为 v0.6.1 新增步骤，默认关闭——可导入票据后开启对比效果。
        </div>
      </div>

      <!-- 总开关 + 仅对扫描件 -->
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 16px">
        <div>
          <div style="font-size: var(--fs-caption); font-weight: 600">启用扫描矫正</div>
          <div style="font-size: var(--fs-caption); color: var(--text-3)">关闭后导入图片保留原图，不再生成增强版</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="draftCfg.enhance.enabled" />
          <span class="slider"></span>
        </label>
      </div>
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 16px">
        <div>
          <div style="font-size: var(--fs-caption); font-weight: 600">仅对扫描件生效</div>
          <div style="font-size: var(--fs-caption); color: var(--text-3)">关闭 = 全部图片都跑；开启 = 截图跳过（已接近二值图，无收益）</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="draftCfg.enhance.onlyScans" :disabled="!draftCfg.enhance.enabled" />
          <span class="slider"></span>
        </label>
      </div>

      <div class="field" style="border-top: 1px solid var(--stroke); padding-top: 16px">
        <label>7 步流水线（按顺序执行，单步失败跳过）</label>
        <div class="step-list">
          <div v-for="s in enhanceActiveSteps" :key="s.key" class="step-row" :class="{ disabled: !draftCfg.enhance.enabled }">
            <label class="toggle">
              <input type="checkbox" v-model="draftCfg.enhance[s.key]" :disabled="!draftCfg.enhance.enabled" />
              <span class="slider"></span>
            </label>
            <div class="step-info">
              <div class="step-label">
                {{ s.label }}
                <span v-if="s.key === 'stepDenoise' || s.key === 'stepSharpen'" class="tag tag-muted">v0.6.1</span>
              </div>
              <div class="step-desc">{{ s.desc }}</div>
            </div>
          </div>
        </div>
      </div>

      <!-- 增强版缺失提示：备份把增强版当可重建缓存排除在外，恢复快照后必然出现。
           界面已自动回退原图（后端不再下发失效路径），这里给出重建入口。 -->
      <div v-if="missingEnhanced > 0" class="callout warning" style="margin-bottom: 12px">
        <span class="ci"><FluentIcon name="warning" :size="18" /></span>
        <div class="ct">
          有 <strong>{{ missingEnhanced }}</strong> 张图片的「增强版」文件不在磁盘上——增强版属可重建缓存、
          不随备份保存（恢复备份后常见）。界面已自动改用原图显示，点下方「重新增强全部」即可重建。
        </div>
      </div>

      <!-- v0.8：存量重新增强（autocrop 算法修复后按新算法重算已有增强版） -->
      <div class="field" style="border-top: 1px solid var(--stroke); padding-top: 16px">
        <label>重新增强存量图片（v0.8）</label>
        <div style="display: flex; align-items: center; gap: 12px; margin-top: 4px">
          <button class="btn" :disabled="reEnhancing" @click="reEnhanceAll">
            {{ reEnhancing ? `重新增强中 ${reEnhanceDone}/${reEnhanceTotal}…` : "重新增强全部" }}
          </button>
          <span style="font-size: var(--fs-caption); color: var(--text-3)">
            自动裁边算法已修复（旧版会把票据边缘裁掉）；点击后按新算法重算全部已有增强版，原图与凭证数据不变
          </span>
        </div>
      </div>

      <!-- D-2 增强缓存管理：摘要 + 命中率 + 释放策略 + 最近列表 -->
      <div class="field" style="border-top: 1px solid var(--stroke); padding-top: 16px">
        <label style="display: flex; align-items: center; gap: 8px">
          增强缓存
          <button class="btn-mini" @click="refreshEnhancePanel" title="刷新"><FluentIcon name="refresh" :size="12" /></button>
        </label>

        <!-- 摘要卡：总数 / 大小 / 截图 / 扫描件 -->
        <div v-if="enhanceStatV2" class="enhance-stat" style="margin-top: 4px">
          <div>
            <span class="stat-num">{{ enhanceStatV2.total }}</span>
            <span class="stat-label">个增强版</span>
          </div>
          <div>
            <span class="stat-num">{{ (enhanceStatV2.totalSize / 1048576).toFixed(1) }}</span>
            <span class="stat-label">MB</span>
          </div>
          <div class="stat-split">
            <span class="tag tag-muted">截图 {{ enhanceStatV2.screenshotCount }}</span>
            <span class="tag">扫描件 {{ enhanceStatV2.scanCount }}</span>
          </div>
        </div>

        <!-- 命中率卡：总命中 / 30 天命中 / 活跃占比 -->
        <div v-if="enhanceStatV2" class="enhance-hit-bar">
          <div class="hit-cell">
            <div class="hit-num">{{ enhanceStatV2.hitCount }}</div>
            <div class="hit-label">累计命中</div>
          </div>
          <div class="hit-cell">
            <div class="hit-num">{{ enhanceStatV2.hitCount30d }}</div>
            <div class="hit-label">近 30 天命中</div>
          </div>
          <div class="hit-cell">
            <div class="hit-num">
              {{ enhanceStatV2.total > 0 ? Math.round((recentEnhanced.filter(r => r.hitCount > 0).length / enhanceStatV2.total) * 100) : 0 }}%
            </div>
            <div class="hit-label">活跃增强版占比</div>
          </div>
        </div>
        <!-- D-8 命中来源分列：OCR / 导出 / 查看 -->
        <div v-if="enhanceStatV2" class="enhance-hit-breakdown">
          <span class="tag" title="OCR 识别时用了增强版">
            OCR {{ enhanceStatV2.hitCountOcr }}
          </span>
          <span class="tag tag-info" title="增强版进了 PDF 附录（仅扫描件）">
            导出 {{ enhanceStatV2.hitCountExport }}
          </span>
          <span class="tag tag-muted" title="大图查看器打开了增强版">
            查看 {{ enhanceStatV2.hitCountViewer }}
          </span>
        </div>

        <!-- D-10 命中趋势：近 N 天堆叠柱状图（手写，无第三方图表库） -->
        <div v-if="hitTrend" class="hit-trend">
          <div class="ht-head">
            <span class="ht-title">命中趋势</span>
            <div class="ht-days">
              <button v-for="d in [7, 14, 30] as const" :key="d" class="ht-day"
                :class="{ on: trendDays === d }" @click="changeTrendDays(d)">
                {{ d }}天
              </button>
            </div>
          </div>
          <div class="ht-chart">
            <div v-for="p in hitTrend.points" :key="p.day" class="ht-col"
              :title="`${p.day}：OCR ${p.ocr} / 导出 ${p.export} / 查看 ${p.viewer}（合计 ${p.total}）`">
              <div class="ht-bar">
                <div class="ht-seg seg-ocr" :style="{ height: barPct(p.ocr) + '%' }"></div>
                <div class="ht-seg seg-export" :style="{ height: barPct(p.export) + '%' }"></div>
                <div class="ht-seg seg-viewer" :style="{ height: barPct(p.viewer) + '%' }"></div>
              </div>
              <div class="ht-day-label">{{ shortDay(p.day) }}</div>
            </div>
          </div>
          <div class="ht-legend">
            <span><i class="ht-dot seg-ocr"></i>OCR</span>
            <span><i class="ht-dot seg-export"></i>导出</span>
            <span><i class="ht-dot seg-viewer"></i>查看</span>
            <span v-if="hitTrend.maxTotal === 0" class="ht-empty">区间内暂无命中</span>
          </div>
        </div>

        <!-- 释放策略 -->
        <div class="clear-strategy">
          <div class="cs-title">释放策略</div>
          <div class="cs-row">
            <label class="cs-opt" title="智能清理：只释放「零命中且超过宽限期」的增强版，被用过的永不清理">
              <input type="radio" v-model="clearStrategy" value="smart" />
              <span>智能·零命中超</span>
              <input v-if="clearStrategy === 'smart'" type="number" min="1" max="365"
                v-model.number="clearThreshold" class="cs-num" />
              <span v-else>30</span>
              <span>天</span>
            </label>
            <label class="cs-opt">
              <input type="radio" v-model="clearStrategy" value="older_than_days" />
              <span>早于</span>
              <input v-if="clearStrategy === 'older_than_days'" type="number" min="1" max="365"
                v-model.number="clearThreshold" class="cs-num" />
              <span v-else>30</span>
              <span>天</span>
            </label>
            <label class="cs-opt">
              <input type="radio" v-model="clearStrategy" value="larger_than_mb" />
              <span>大于</span>
              <input v-if="clearStrategy === 'larger_than_mb'" type="number" min="1" max="500"
                v-model.number="clearThreshold" class="cs-num" />
              <span v-else>10</span>
              <span>MB</span>
            </label>
            <label class="cs-opt">
              <input type="radio" v-model="clearStrategy" value="all" />
              <span>全部</span>
            </label>
            <button class="btn btn-danger" :disabled="clearingCache || !enhanceStatV2?.total" @click="clearEnhanceV2">
              {{ clearingCache ? "释放中…" : "释放空间" }}
            </button>
          </div>
          <div class="cs-hint">
            仅清缓存，不删原图；下次导入同 hash 图会重新生成。智能 = 零命中且超过宽限期的先淘汰（推荐）
          </div>
        </div>

        <!-- 最近增强 -->
        <div class="recent-enhanced">
          <div class="re-title">最近增强（按修改时间倒序）</div>
          <!-- 窄窗兜底：裸表在窄档会被 .fluent-panel 的 overflow:hidden 裁掉，套滚动容器 -->
          <div v-if="recentEnhanced.length" class="dtable-wrap">
          <table class="re-table">
            <thead>
              <tr>
                <th style="width: 38%">文件</th>
                <th style="width: 18%">来源</th>
                <th style="width: 14%; text-align: right">大小</th>
                <th style="width: 10%; text-align: right">命中</th>
                <th style="width: 20%; text-align: right">操作</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="r in recentEnhanced" :key="r.attachmentId">
                <td :title="r.voucherNo + ' / ' + r.fileName">
                  <div class="re-file">{{ r.fileName }}</div>
                  <div class="re-vno">{{ r.voucherNo }}</div>
                </td>
                <td>
                  <span v-if="r.enhancedSource === 'scan'" class="tag">扫描件</span>
                  <span v-else-if="r.enhancedSource === 'screenshot'" class="tag tag-muted">截图</span>
                  <span v-else class="tag tag-muted">来源未知</span>
                </td>
                <td style="text-align: right; font-variant-numeric: tabular-nums">
                  {{ (r.enhancedSize / 1024).toFixed(0) }} KB
                </td>
                <td style="text-align: right; font-variant-numeric: tabular-nums">
                  <span :class="['re-hit', r.hitCount > 0 ? 'on' : 'off']">{{ r.hitCount }}</span>
                </td>
                <td style="text-align: right">
                  <button v-if="pendingDeleteOne === r.attachmentId" class="btn-mini danger"
                    title="再次点击确认释放（5 秒内有效）" @click="deleteOneEnhanced(r)">
                    再点确认
                  </button>
                  <button v-else class="btn-mini" title="释放该条增强版" @click="deleteOneEnhanced(r)">
                    释放
                  </button>
                </td>
              </tr>
            </tbody>
          </table>
          </div>
          <div v-else class="re-empty">尚无增强记录；导入图片并启用扫描矫正后会出现在这里</div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 7 步流水线步骤列表 */
.step-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.step-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 0;
  transition: opacity 0.15s ease;
}
.step-row.disabled {
  opacity: 0.5;
}
.step-info {
  flex: 1;
  min-width: 0;
}
.step-label {
  font-size: var(--fs-caption);
  font-weight: 600;
}
.step-desc {
  font-size: var(--fs-caption);
  color: var(--text-3);
  margin-top: 2px;
}

/* 增强缓存统计 */
.enhance-stat {
  display: flex;
  gap: 24px;
  align-items: center;
  padding: 8px 0;
}
.stat-num {
  font-size: var(--fs-display);
  font-weight: 600;
  color: var(--text);
  font-variant-numeric: tabular-nums;
  margin-right: 4px;
}
.stat-label {
  font-size: var(--fs-caption);
  color: var(--text-3);
}
.stat-split {
  display: flex;
  gap: 6px;
}

/* D-2 增强缓存管理 v2 */
.btn-mini {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: var(--fs-caption);
  font-weight: 500;
  padding: 3px 10px;
  border-radius: 4px;
  border: 1px solid var(--stroke);
  background: var(--surface-2);
  color: var(--text-2);
  cursor: pointer;
  transition: background 0.12s ease, border-color 0.12s ease;
  line-height: 1.4;
}
.btn-mini:hover {
  background: var(--surface-3);
  border-color: var(--accent);
  color: var(--text);
}
.btn-mini.danger {
  background: rgba(220, 38, 38, 0.1);
  border-color: rgba(220, 38, 38, 0.4);
  color: var(--danger);
  font-weight: 600;
  animation: d2-pulse 1.2s ease-in-out infinite;
}
@keyframes d2-pulse {
  0%, 100% { background: rgba(220, 38, 38, 0.1); }
  50%      { background: rgba(220, 38, 38, 0.22); }
}
/* 释放空间按钮（危险加强版；与期间解锁按钮保持一致） */
.btn.btn-danger {
  background: var(--bg-control);
  border-color: var(--danger-soft);
  color: var(--danger);
}
.btn.btn-danger:hover:not(:disabled) {
  background: var(--danger-soft);
}

/* .enhance-hit-bar 已收敛到 src/styles/base.css《窗口缩放适配》一节（scoped 特异性高于
   base.css，会让全局断点失效，故移出组件）。 */
.hit-cell { text-align: center; }
.hit-num {
  font-size: var(--fs-display);
  font-weight: 600;
  color: var(--accent);
  font-variant-numeric: tabular-nums;
  line-height: 1.2;
}
.hit-label {
  font-size: var(--fs-caption);
  color: var(--text-3);
  margin-top: 2px;
}
/* D-8：命中来源分列（OCR / 导出 / 查看） */
.enhance-hit-breakdown {
  display: flex;
  gap: 8px;
  margin: -8px 0 16px;
  padding: 0 2px;
}

/* D-10：命中趋势堆叠柱状图 */
.hit-trend {
  border: 1px solid var(--stroke);
  border-radius: var(--radius-control);
  padding: 12px;
  margin-bottom: 16px;
  background: var(--surface);
}
.ht-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}
.ht-title {
  font-size: var(--fs-caption);
  font-weight: 600;
  color: var(--text-2);
}
.ht-days {
  display: flex;
  gap: 4px;
}
.ht-day {
  font-size: var(--fs-caption);
  padding: 2px 8px;
  border: 1px solid var(--stroke);
  border-radius: 4px;
  background: var(--surface-2);
  color: var(--text-3);
  cursor: pointer;
  transition: all 0.12s ease;
}
.ht-day:hover {
  color: var(--text);
  border-color: var(--accent);
}
.ht-day.on {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--label-on-accent);
}
.ht-chart {
  display: flex;
  align-items: flex-end;
  gap: 2px;
  height: 84px;
}
.ht-col {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: stretch;
}
.ht-bar {
  height: 64px;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  gap: 1px;
}
.ht-seg {
  width: 100%;
  border-radius: var(--radius-small);
  min-height: 0;
  transition: height 0.25s ease;
}
.ht-seg.seg-ocr    { background: var(--accent); }
.ht-seg.seg-export { background: var(--success); }
.ht-seg.seg-viewer { background: var(--label-tertiary); }
.ht-day-label {
  font-size: var(--fs-caption);
  color: var(--text-3);
  text-align: center;
  margin-top: 4px;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  overflow: hidden;
}
.ht-legend {
  display: flex;
  gap: 12px;
  align-items: center;
  margin-top: 8px;
  font-size: var(--fs-caption);
  color: var(--text-3);
}
.ht-dot {
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: var(--radius-small);
  margin-right: 4px;
  vertical-align: -1px;
}
.ht-empty {
  margin-left: auto;
  color: var(--text-3);
}

.clear-strategy {
  border: 1px solid var(--stroke);
  border-radius: var(--radius-control);
  padding: 12px;
  margin-bottom: 16px;
  background: var(--surface);
}
.cs-title {
  font-size: var(--fs-caption);
  font-weight: 600;
  color: var(--text-2);
  margin-bottom: 8px;
}
.cs-row {
  display: flex;
  align-items: center;
  gap: 16px;
  flex-wrap: wrap;
}
.cs-opt {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-caption);
  color: var(--text-2);
  cursor: pointer;
}
.cs-opt input[type="radio"] {
  margin: 0 4px 0 0;
  cursor: pointer;
}
.cs-num {
  width: 56px;
  font-size: var(--fs-caption);
  padding: 2px 6px;
  border: 1px solid var(--stroke);
  border-radius: var(--radius-small);
  background: var(--surface-2);
  color: var(--text);
  text-align: center;
  font-variant-numeric: tabular-nums;
}
.cs-hint {
  font-size: var(--fs-caption);
  color: var(--text-3);
  margin-top: 8px;
}

.recent-enhanced {
  border: 1px solid var(--stroke);
  border-radius: var(--radius-control);
  padding: 12px;
  background: var(--surface);
}
.re-title {
  font-size: var(--fs-caption);
  font-weight: 600;
  color: var(--text-2);
  margin-bottom: 8px;
}
.re-table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-caption);
}
.re-table th {
  text-align: left;
  font-weight: 500;
  color: var(--text-3);
  padding: 4px 8px;
  border-bottom: 1px solid var(--stroke);
  background: var(--surface-2);
}
.re-table td {
  padding: 6px 8px;
  border-bottom: 1px solid var(--stroke);
  color: var(--text);
  vertical-align: middle;
}
.re-table tr:last-child td { border-bottom: none; }
.re-file {
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 220px;
}
.re-vno {
  font-size: var(--fs-caption);
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
  margin-top: 1px;
}
.re-hit {
  display: inline-block;
  min-width: 22px;
  padding: 1px 6px;
  border-radius: var(--radius-small);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
.re-hit.on  { background: rgba(99, 102, 241, 0.12); color: var(--accent); }
.re-hit.off { background: var(--surface-2); color: var(--text-3); }
.re-empty {
  padding: 16px;
  text-align: center;
  color: var(--text-3);
  font-size: var(--fs-caption);
}
</style>
