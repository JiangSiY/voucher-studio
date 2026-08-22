<script setup lang="ts">
/**
 * 设置（Tab 分区，对齐 UI 方案的配置页模式）：
 * 通用 / 凭证参数 / OCR / 密钥管理 / 备份与同步 / 微信监听
 */
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, type AppConfig, type ClearStrategy, type ClearSummary, type EnhanceStatus, type EnhanceStatusV2, type LocalOcrStatus, type RecentEnhancedRow, type SecretStatus, type WechatStatus } from "../api/client";
import { toast, toastError } from "../stores/toast";
import { useConfigStore } from "../stores/config";

const configStore = useConfigStore();

const tabs = [
  { key: "general", label: "通用" },
  { key: "naming", label: "凭证参数" },
  { key: "ocr", label: "OCR" },
  { key: "enhance", label: "扫描矫正" },
  { key: "secrets", label: "密钥管理" },
  { key: "backup", label: "备份与同步" },
  { key: "wechat", label: "微信监听" },
] as const;
type TabKey = (typeof tabs)[number]["key"];
const tab = ref<TabKey>("general");

// 编辑中的配置副本（保存时整体提交）
const cfg = reactive<AppConfig>(JSON.parse(JSON.stringify(defaultConfig())));

function defaultConfig(): AppConfig {
  return {
    appearance: { theme: "system" },
    naming: { template: "{date}-{seq}" },
    ocr: { engine: "local", enabled: true, apiUrl: "", model: "" },
    backup: {
      autoEnabled: true,
      intervalHours: 24,
      keepDaily: 7,
      keepWeekly: 4,
      keepMonthly: 6,
      secondaryDir: "",
      webdav: { enabled: false, url: "", username: "", passwordSet: false, remoteRetention: 10 },
    },
    wechat: { enabled: false, watchDir: "", mode: "events" },
    print: { title: "凭证日报", cardStyle: "standard", columns: 3, includeImages: true, groupByDay: true },
    enhance: {
      enabled: true,
      onlyScans: false,
      stepDeskew: true,
      stepDewarp: true,
      stepAutocrop: true,
      stepGrayscale: true,
      stepBinarize: true,
      stepSharpen: false,
      stepDenoise: false,
    },
  };
}

const secrets = ref<SecretStatus[]>([]);
const localOcr = ref<LocalOcrStatus | null>(null);
const wechatStatus = ref<WechatStatus | null>(null);
const enhanceStat = ref<EnhanceStatus | null>(null);
// D-2：增强缓存管理 v2
const enhanceStatV2 = ref<EnhanceStatusV2 | null>(null);
const recentEnhanced = ref<RecentEnhancedRow[]>([]);
const clearingCache = ref(false);
const saving = ref(false);
const templatePreview = ref("");
const templateError = ref("");

// D-2 释放策略状态
type ClearStrategyOpt = ClearStrategy;
const clearStrategy = ref<ClearStrategyOpt>("all");
const clearThreshold = ref<number>(30); // older_than_days=30 天 / larger_than_mb=10 MB
const pendingDeleteOne = ref<number | null>(null); // 单条释放：行 id（null = 不可见）

async function load() {
  try {
    const c = await api.getConfig();
    Object.assign(cfg, c);
    secrets.value = await api.listSecrets();
    localOcr.value = await api.getLocalOcrStatus();
    wechatStatus.value = await api.getWechatStatus();
    enhanceStat.value = await api.enhanceStatus();
    await refreshEnhancePanel();
    validateTemplate();
  } catch (e) {
    toastError(e);
  }
}

async function refreshEnhanceStatus() {
  try {
    enhanceStat.value = await api.enhanceStatus();
  } catch (e) {
    toastError(e);
  }
}

/** D-2 刷新：v2 状态 + 最近列表 */
async function refreshEnhancePanel() {
  try {
    enhanceStatV2.value = await api.enhanceStatusV2();
    recentEnhanced.value = await api.recentEnhanced(20);
  } catch (e) {
    toastError(e);
  }
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
        : `确定要清除「大于 ${t} MB」的增强版缓存？其它不受影响。`;
  if (!confirm(confirmMsg)) return;
  clearingCache.value = true;
  try {
    const r: ClearSummary = await api.clearEnhanceCacheV2(s, t);
    toast.success(
      `已释放 ${r.deletedFiles} 个增强版（${(r.freedBytes / 1048576).toFixed(1)} MB）`,
    );
    await Promise.all([refreshEnhanceStatus(), refreshEnhancePanel()]);
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
      await Promise.all([refreshEnhanceStatus(), refreshEnhancePanel()]);
    } catch (e) {
      toastError(e);
    }
    return;
  }
  pendingDeleteOne.value = row.attachmentId;
  // 5 秒后自动撤销二次确认态
  setTimeout(() => {
    if (pendingDeleteOne.value === row.attachmentId) {
      pendingDeleteOne.value = null;
    }
  }, 5000);
}

/** D-2 兼容：旧「清除增强缓存」按钮（保留以防外部链接 / 旧调用） */
async function clearEnhance() {
  clearStrategy.value = "all";
  clearThreshold.value = 0;
  await clearEnhanceV2();
}

// 5 步流水线（v0.6 真生效）
const enhanceActiveSteps: { key: keyof AppConfig["enhance"]; label: string; desc: string }[] = [
  { key: "stepDeskew",    label: "1. 倾斜矫正",      desc: "Hough 直线 → 取最强水平线倾角 → 旋转校正" },
  { key: "stepDewarp",    label: "2. 透视矫正",      desc: "检测最大四边形 → 透视变换" },
  { key: "stepAutocrop",  label: "3. 自动裁边",      desc: "去白边，按轮廓裁剪 + 5% 边距" },
  { key: "stepGrayscale", label: "4. 灰度化",        desc: "BGR → GRAY，给 OCR 用" },
  { key: "stepBinarize",  label: "5. 自适应二值化",  desc: "高斯加权 → 31 窗口 / C=10" },
];

// 2 个占位（v0.6.1 评估，UI 先行）
const enhancePlaceholderSteps: { key: keyof AppConfig["enhance"]; label: string; desc: string }[] = [
  { key: "stepSharpen", label: "锐化", desc: "v0.6.1 评估：拉普拉斯 / Unsharp Mask" },
  { key: "stepDenoise", label: "去噪", desc: "v0.6.1 评估：中值 / 高斯 / 双边" },
];

async function validateTemplate() {
  templateError.value = "";
  try {
    // 后端返回归一化模板；预览用今日日期本地渲染全部令牌
    // （preview_voucher_no 基于已保存配置，编辑中的模板尚未保存）
    const normalized = await api.validateNamingTemplate(cfg.naming.template);
    const now = new Date();
    const y = now.getFullYear();
    const m = String(now.getMonth() + 1).padStart(2, "0");
    const d = String(now.getDate()).padStart(2, "0");
    templatePreview.value = normalized
      .replace(/\{date-\}/g, `${y}-${m}-${d}`)
      .replace(/\{date\}/g, `${y}${m}${d}`)
      .replace(/\{seq\}/g, "001")
      .replace(/\{n\}/g, "01");
  } catch (e) {
    templateError.value = typeof e === "string" ? e : String(e);
    templatePreview.value = "";
  }
}

async function save() {
  saving.value = true;
  try {
    await configStore.save(JSON.parse(JSON.stringify(cfg)));
    secrets.value = await api.listSecrets();
    wechatStatus.value = await api.getWechatStatus();
    toast.success("设置已保存");
    // 微信监听保存后即时生效；失败在此明确提示
    const ws = wechatStatus.value;
    if (ws?.enabled && !ws.running) {
      toast.warning(`微信监听未能启动：${ws.error ?? "未知原因"}`);
    }
  } catch (e) {
    toastError(e);
  } finally {
    saving.value = false;
  }
}

// 密钥管理
const secretDrafts = reactive<Record<string, string>>({});

async function pickSecondaryDir() {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const dir = await open({ directory: true });
  if (typeof dir === "string") {
    cfg.backup.secondaryDir = dir;
  }
}

async function saveSecret(alias: string) {
  const value = secretDrafts[alias]?.trim();
  if (!value) {
    toast.warning("请输入内容");
    return;
  }
  try {
    await api.setSecret(alias, value);
    secretDrafts[alias] = "";
    secrets.value = await api.listSecrets();
    toast.success("已写入系统凭据库");
  } catch (e) {
    toastError(e);
  }
}

async function removeSecret(alias: string) {
  try {
    await api.deleteSecret(alias);
    secrets.value = await api.listSecrets();
    toast.success("已删除");
  } catch (e) {
    toastError(e);
  }
}

// WebDAV 密码写入凭据库后同步 passwordSet 标记
async function saveWebdavPassword() {
  await saveSecret("webdav_password");
  cfg.backup.webdav.passwordSet = true;
}

async function pickWatchDir() {
  const dir = await api.pickDirectory();
  if (dir) cfg.wechat.watchDir = dir;
}

// 批量识别待处理附件（列表页导入未跑 OCR 的、历史失败件等），成功后自动填单（只填空字段）
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
  try {
    const n = await api.runOcrPending(500);
    if (n > 0) {
      toast.success(`批量识别完成：${n} 张附件已识别并自动填单`);
    } else {
      toast.info?.("没有待识别的附件");
    }
  } catch (e) {
    toastError(e);
  } finally {
    ocrPendingBusy.value = false;
    ocrProgress.value = null;
  }
}

const dataDir = computed(() => configStore.appInfo?.dataDir ?? "");

async function openDataDir() {
  try {
    await api.openPath(dataDir.value);
  } catch (e) {
    toastError(e);
  }
}

// 批量识别进度（后端逐张推送 ocr:progress 事件）
let unlistenOcrProgress: UnlistenFn | null = null;

onMounted(async () => {
  load();
  unlistenOcrProgress = await listen<{ done: number; total: number }>("ocr:progress", (e) => {
    if (ocrPendingBusy.value) ocrProgress.value = e.payload;
  });
});

onUnmounted(() => unlistenOcrProgress?.());
</script>

<template>
  <div class="tab-nav">
    <button v-for="t in tabs" :key="t.key" class="tab-item" :class="{ active: tab === t.key }" @click="tab = t.key">
      {{ t.label }}
    </button>
  </div>

  <!-- 通用 -->
  <div v-if="tab === 'general'" class="fluent-panel">
    <div class="fluent-panel-body">
      <div class="field">
        <label>界面主题</label>
        <select v-model="cfg.appearance.theme" class="input" style="max-width: 240px">
          <option value="system">跟随系统</option>
          <option value="light">浅色</option>
          <option value="dark">深色</option>
        </select>
      </div>
      <div class="field">
        <label>数据目录</label>
        <div style="display: flex; gap: 8px; align-items: center">
          <input :value="dataDir" class="input mono" disabled style="flex: 1" />
          <button class="btn" @click="openDataDir">打开目录</button>
        </div>
        <div class="desc">所有数据（数据库、附件图片、备份）都保存在本机此目录下。</div>
      </div>

      <div class="field" style="border-top: 1px solid var(--stroke); padding-top: 16px">
        <label>打印默认值</label>
        <div class="form-grid">
          <div class="field">
            <label>{{ cfg.print.groupByDay ? "日报标题" : "整理单标题" }}</label>
            <input v-model="cfg.print.title" class="input" placeholder="如：凭证日报" />
          </div>
          <div class="field">
            <label>每行列数（合并单份）</label>
            <select v-model.number="cfg.print.columns" class="input">
              <option :value="3">3 列 · 21 张/页</option>
              <option :value="2">2 列 · 14 张/页</option>
              <option :value="1">1 列 · 7 张/页</option>
            </select>
          </div>
        </div>
        <div class="form-grid" style="margin-top: 8px">
          <div class="field">
            <label>卡片风格（合并单份）</label>
            <select v-model="cfg.print.cardStyle" class="input">
              <option value="standard">标准式</option>
              <option value="clean">简洁式</option>
              <option value="ledger">账簿式</option>
            </select>
          </div>
          <div class="field">
            <label>导出方式</label>
            <div style="display: flex; gap: 8px; align-items: center; padding-top: 6px">
              <label class="toggle">
                <input type="checkbox" v-model="cfg.print.groupByDay" />
                <span class="slider"></span>
              </label>
              <span style="font-size: 12px; color: var(--text-3)">
                {{ cfg.print.groupByDay ? "按天分组，每天一份「凭证日报」" : "全部合并为一份整理单" }}
              </span>
            </div>
          </div>
        </div>
        <div style="display: flex; gap: 8px; align-items: center; margin-top: 8px">
          <label class="toggle">
            <input type="checkbox" v-model="cfg.print.includeImages" />
            <span class="slider"></span>
          </label>
          <span style="font-size: 12px; color: var(--text-3)">附带附件图片页（每页 2×2）</span>
        </div>
        <div class="desc">打印中心进入时按此默认值初始化；在打印中心内的临时调整不影响这里。</div>
      </div>
    </div>
  </div>

  <!-- 凭证参数 -->
  <div v-else-if="tab === 'naming'" class="fluent-panel">
    <div class="fluent-panel-body">
      <div class="field">
        <label>凭证编号模板</label>
        <input v-model="cfg.naming.template" class="input mono" @input="validateTemplate" />
        <div class="desc">
          可用令牌：<code>{date}</code> 当天日期（如 20260819）、<code>{seq}</code> 当日序号（001 起）。
          默认 <code>{date}-{seq}</code> 生成形如 20260819-001 的编号。
        </div>
        <div v-if="templateError" class="desc" style="color: var(--danger)">⚠ {{ templateError }}</div>
        <div v-else-if="templatePreview" class="desc" style="color: var(--success)">
          ✓ 保存后新凭证编号形如：{{ templatePreview }}
        </div>
      </div>
      <div class="callout info" style="margin: 0">
        <span class="ci">💡</span>
        <div class="ct">修改模板只影响之后新建的凭证，已有编号不会变。</div>
      </div>
    </div>
  </div>

  <!-- OCR -->
  <div v-else-if="tab === 'ocr'" class="fluent-panel">
    <div class="fluent-panel-body">
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 16px">
        <div>
          <div style="font-size: 13px; font-weight: 600">启用 OCR</div>
          <div style="font-size: 12px; color: var(--text-3)">关闭后导入图片不再自动识别文字</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="cfg.ocr.enabled" />
          <span class="slider"></span>
        </label>
      </div>
      <div class="field">
        <label>识别引擎</label>
        <select v-model="cfg.ocr.engine" class="input" style="max-width: 280px">
          <option value="local" :disabled="!localOcr?.installed">本地（离线 · 推荐）</option>
          <option value="cloud">云端（HTTP API）</option>
        </select>
        <div class="desc">本地引擎完全离线运行，数据不出本机（PP-OCRv4，Spike B 实测字符准确率 99.9%）。</div>
      </div>
      <div v-if="localOcr?.installed" class="callout success" style="margin: 0 0 16px">
        <span class="ci">✓</span>
        <div class="ct">
          <strong>本地模型已安装</strong>
          <span v-if="localOcr.files.length">（{{ localOcr.files.length }} 个文件，约 {{ (localOcr.totalSize / 1048576).toFixed(1) }} MB）</span>
          ：RapidOCR PP-OCRv4 检测 + 方向分类 + 识别，首次识别时加载约 0.2 秒。
        </div>
      </div>
      <div v-else class="callout warning" style="margin: 0 0 16px">
        <span class="ci">⚠</span>
        <div class="ct">
          <strong>本地模型未安装</strong>——将模型文件（ch_PP-OCRv4_det_infer.onnx 等 3 个）放入
          <code>{{ localOcr?.modelsDir || "数据目录/models" }}</code> 后重启应用即可启用。
        </div>
      </div>
      <template v-if="cfg.ocr.engine === 'cloud'">
        <div class="field">
          <label>API 地址</label>
          <input v-model="cfg.ocr.apiUrl" class="input mono" placeholder="https://api.example.com/ocr" />
          <div class="desc">POST JSON：{image_base64, model}，返回 {text}；API Key 在「密钥管理」页设置。</div>
        </div>
        <div class="field">
          <label>模型名（可选）</label>
          <input v-model="cfg.ocr.model" class="input" placeholder="如：voucher-ocr-v1" />
        </div>
      </template>
      <div class="field" style="border-top: 1px solid var(--stroke); padding-top: 16px">
        <label>批量识别</label>
        <div style="display: flex; gap: 8px; align-items: center">
          <button class="btn" :disabled="ocrPendingBusy" @click="runOcrPending">
            {{ ocrPendingBusy ? "识别中…" : "识别全部待处理" }}
          </button>
          <span style="font-size: 12px; color: var(--text-3)">
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

  <!-- 扫描矫正（v0.6：5 步算法 + 2 占位） -->
  <div v-else-if="tab === 'enhance'" class="fluent-panel">
    <div class="fluent-panel-body">
      <div class="callout info" style="margin: 0 0 16px">
        <span class="ci">✨</span>
        <div class="ct">
          <strong>文档扫描矫正</strong>：对导入的图片自动跑 5 步算法（Hough 倾斜矫正 → 透视矫正 → 自动裁边 → 灰度化 → 自适应二值化），
          矫正后的增强版用于 OCR（更准）和查看器（更易读）；任一步失败自动跳过、回退到上一步输出。
        </div>
      </div>

      <!-- 总开关 + 仅对扫描件 -->
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 16px">
        <div>
          <div style="font-size: 13px; font-weight: 600">启用扫描矫正</div>
          <div style="font-size: 12px; color: var(--text-3)">关闭后导入图片保留原图，不再生成增强版</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="cfg.enhance.enabled" />
          <span class="slider"></span>
        </label>
      </div>
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 16px">
        <div>
          <div style="font-size: 13px; font-weight: 600">仅对扫描件生效</div>
          <div style="font-size: 12px; color: var(--text-3)">关闭 = 全部图片都跑；开启 = 截图跳过（已接近二值图，无收益）</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="cfg.enhance.onlyScans" :disabled="!cfg.enhance.enabled" />
          <span class="slider"></span>
        </label>
      </div>

      <div class="field" style="border-top: 1px solid var(--stroke); padding-top: 16px">
        <label>5 步流水线（按顺序执行，单步失败跳过）</label>
        <div class="step-list">
          <div v-for="s in enhanceActiveSteps" :key="s.key" class="step-row" :class="{ disabled: !cfg.enhance.enabled }">
            <label class="toggle">
              <input type="checkbox" v-model="cfg.enhance[s.key]" :disabled="!cfg.enhance.enabled" />
              <span class="slider"></span>
            </label>
            <div class="step-info">
              <div class="step-label">{{ s.label }}</div>
              <div class="step-desc">{{ s.desc }}</div>
            </div>
          </div>
        </div>
      </div>

      <div class="field" style="border-top: 1px solid var(--stroke); padding-top: 16px">
        <label>v0.6.1 评估（占位 · v0.6 不生效）</label>
        <div class="step-list">
          <div v-for="s in enhancePlaceholderSteps" :key="s.key" class="step-row disabled" title="v0.6.1 评估，v0.6 不生效">
            <label class="toggle">
              <input type="checkbox" v-model="cfg.enhance[s.key]" disabled />
              <span class="slider"></span>
            </label>
            <div class="step-info">
              <div class="step-label">{{ s.label }} <span class="tag tag-muted">占位</span></div>
              <div class="step-desc">{{ s.desc }}</div>
            </div>
          </div>
        </div>
      </div>

      <!-- D-2 增强缓存管理：摘要 + 命中率 + 释放策略 + 最近列表 -->
      <div class="field" style="border-top: 1px solid var(--stroke); padding-top: 16px">
        <label style="display: flex; align-items: center; gap: 8px">
          增强缓存
          <button class="btn-mini" @click="refreshEnhancePanel" title="刷新">↻</button>
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

        <!-- 释放策略 -->
        <div class="clear-strategy">
          <div class="cs-title">释放策略</div>
          <div class="cs-row">
            <label class="cs-opt">
              <input type="radio" v-model="clearStrategy" value="all" />
              <span>全部</span>
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
            <button class="btn btn-danger" :disabled="clearingCache || !enhanceStatV2?.total" @click="clearEnhanceV2">
              {{ clearingCache ? "释放中…" : "释放空间" }}
            </button>
          </div>
          <div class="cs-hint">
            仅清缓存，不删原图；下次导入同 hash 图会重新生成
          </div>
        </div>

        <!-- 最近增强 -->
        <div class="recent-enhanced">
          <div class="re-title">最近增强（按修改时间倒序）</div>
          <table v-if="recentEnhanced.length" class="re-table">
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
                  <span v-else class="tag tag-muted">未识别</span>
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
          <div v-else class="re-empty">尚无增强记录；导入图片并启用扫描矫正后会出现在这里</div>
        </div>
      </div>
    </div>
  </div>

  <!-- 密钥管理 -->
  <div v-else-if="tab === 'secrets'" class="fluent-panel">
    <div class="fluent-panel-header"><h3>密钥管理</h3></div>
    <div class="fluent-panel-body">
      <div class="callout info">
        <span class="ci">🔐</span>
        <div class="ct">
          <strong>密钥保存在系统凭据库</strong>（Windows 凭据管理器 / macOS 钥匙串），不写进任何文件——
          换电脑需要重新录入。
        </div>
      </div>
      <div v-for="s in secrets" :key="s.alias" class="field"
        style="border: 1px solid var(--stroke); border-radius: 6px; padding: 12px">
        <label>{{ s.label }} <span class="tag" :class="s.exists ? 'tag-success' : 'tag-muted'">{{ s.exists ? "已设置" : "未设置" }}</span></label>
        <div style="display: flex; gap: 8px">
          <input v-model="secretDrafts[s.alias]" class="input" type="password" autocomplete="new-password"
            :placeholder="s.exists ? '输入新值覆盖' : '输入密钥'" style="flex: 1" />
          <button class="btn btn-primary" @click="saveSecret(s.alias)">保存</button>
          <button class="btn" :disabled="!s.exists" @click="removeSecret(s.alias)">删除</button>
        </div>
      </div>
    </div>
  </div>

  <!-- 备份与同步 -->
  <div v-else-if="tab === 'backup'" class="fluent-panel">
    <div class="fluent-panel-body">
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 16px">
        <div>
          <div style="font-size: 13px; font-weight: 600">自动备份</div>
          <div style="font-size: 12px; color: var(--text-3)">应用运行期间按间隔自动打包数据库、配置与全部附件原图</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="cfg.backup.autoEnabled" />
          <span class="slider"></span>
        </label>
      </div>
      <div class="form-grid">
        <div class="field">
          <label>备份间隔（小时）</label>
          <input v-model.number="cfg.backup.intervalHours" type="number" min="1" class="input" />
          <div class="desc">每份备份含 sha256 完整性清单，恢复前自动校验。</div>
        </div>
        <div class="field">
          <label>备用份目录（异盘双写）</label>
          <div style="display: flex; gap: 8px">
            <input v-model="cfg.backup.secondaryDir" class="input mono" placeholder="如 D:\VoucherBackups（留空不启用）" />
            <button class="btn" @click="pickSecondaryDir">选择…</button>
          </div>
          <div class="desc">建议指向另一块物理盘或移动硬盘：本盘故障时备份仍在。该目录同样应用下方保留策略。</div>
        </div>
      </div>
      <div class="field">
        <label>分层保留（日 / 周 / 月）</label>
        <div style="display: flex; gap: 8px">
          <input v-model.number="cfg.backup.keepDaily" type="number" min="0" class="input" style="max-width: 110px" />
          <span style="align-self: center; color: var(--text-3)">天每日备</span>
          <input v-model.number="cfg.backup.keepWeekly" type="number" min="0" class="input" style="max-width: 110px" />
          <span style="align-self: center; color: var(--text-3)">周每周备</span>
          <input v-model.number="cfg.backup.keepMonthly" type="number" min="0" class="input" style="max-width: 110px" />
          <span style="align-self: center; color: var(--text-3)">月每月备</span>
        </div>
        <div class="desc">祖父-父-子（GFS）策略：近 {{ cfg.backup.keepDaily }} 天全保留，再留 {{ cfg.backup.keepWeekly }} 周的每周最新一份、{{ cfg.backup.keepMonthly }} 月的每月最新一份。</div>
      </div>

      <div style="border-top: 1px solid var(--stroke); margin: 8px 0 16px"></div>

      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 16px">
        <div>
          <div style="font-size: 13px; font-weight: 600">WebDAV 云备份</div>
          <div style="font-size: 12px; color: var(--text-3)">每次备份（含手动）自动上传，失败自动重试</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="cfg.backup.webdav.enabled" />
          <span class="slider"></span>
        </label>
      </div>
      <div class="form-grid">
        <div class="field">
          <label>服务器地址</label>
          <input v-model="cfg.backup.webdav.url" class="input mono" placeholder="https://dav.example.com/voucher/" />
        </div>
        <div class="field">
          <label>用户名</label>
          <input v-model="cfg.backup.webdav.username" class="input" autocomplete="off" />
        </div>
      </div>
      <div class="form-grid">
        <div class="field">
          <label>密码</label>
          <div style="display: flex; gap: 8px">
            <input :value="cfg.backup.webdav.passwordSet ? '••••••••（已保存在系统凭据库）' : ''" class="input" disabled
              style="flex: 1" />
            <button class="btn" @click="saveWebdavPassword">{{ cfg.backup.webdav.passwordSet ? "修改" : "设置" }}密码</button>
          </div>
          <div class="desc">密码只存系统凭据库，请在弹出输入后点「密钥管理」确认状态。</div>
        </div>
        <div class="field">
          <label>远端保留份数</label>
          <input v-model.number="cfg.backup.webdav.remoteRetention" type="number" min="0" class="input"
            style="max-width: 110px" />
          <div class="desc">超出删远端最旧（0 = 不清理）。备份文件名含时间戳，按名排序即按时间。</div>
        </div>
      </div>
    </div>
  </div>

  <!-- 微信监听 -->
  <div v-else-if="tab === 'wechat'" class="fluent-panel">
    <div class="fluent-panel-body">
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 16px">
        <div>
          <div style="font-size: 13px; font-weight: 600">监听微信收件文件夹</div>
          <div style="font-size: 12px; color: var(--text-3)">文件夹里出现新图片时自动导入建卡（保存后立即生效，含子目录）</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="cfg.wechat.enabled" />
          <span class="slider"></span>
        </label>
      </div>
      <div v-if="wechatStatus" class="field">
        <div v-if="wechatStatus.enabled && wechatStatus.running" class="callout success" style="margin: 0">
          <span class="ci">●</span>
          <div class="ct">监听运行中：{{ wechatStatus.watchDir || "（未设置目录）" }}</div>
        </div>
        <div v-else-if="wechatStatus.enabled && !wechatStatus.running" class="callout danger" style="margin: 0">
          <span class="ci">●</span>
          <div class="ct">
            监听未运行{{ wechatStatus.error ? `：${wechatStatus.error}` : "（保存设置后自动启动）" }}
          </div>
        </div>
        <div v-else class="desc" style="margin: 0">监听未启用</div>
      </div>
      <div class="field">
        <label>监听目录</label>
        <div style="display: flex; gap: 8px">
          <input v-model="cfg.wechat.watchDir" class="input mono" placeholder="如 C:\Users\你\Documents\WeChat Files\...\FileStorage\Image"
            style="flex: 1" />
          <button class="btn" @click="pickWatchDir">选择文件夹</button>
        </div>
        <div class="desc">建议指向微信「文件存储 → 图片」目录（递归监听子目录）；同一张图片重复出现会自动去重。</div>
      </div>
    </div>
  </div>

  <div style="display: flex; justify-content: flex-end; margin-top: 8px">
    <button class="btn btn-primary" :disabled="saving" @click="save">{{ saving ? "保存中…" : "保存设置" }}</button>
  </div>
</template>

<style scoped>
/* 5 步流水线步骤列表（增强 Tab） */
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
  font-size: 13px;
  font-weight: 600;
}
.step-desc {
  font-size: 12px;
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
  font-size: 20px;
  font-weight: 600;
  color: var(--text-1);
  font-variant-numeric: tabular-nums;
  margin-right: 4px;
}
.stat-label {
  font-size: 12px;
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
  font-size: 11px;
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
  color: var(--text-1);
}
.btn-mini.danger {
  background: rgba(220, 38, 38, 0.1);
  border-color: rgba(220, 38, 38, 0.4);
  color: #dc2626;
  font-weight: 600;
  animation: d2-pulse 1.2s ease-in-out infinite;
}
@keyframes d2-pulse {
  0%, 100% { background: rgba(220, 38, 38, 0.1); }
  50%      { background: rgba(220, 38, 38, 0.22); }
}
.btn.btn-danger {
  background: linear-gradient(135deg, #dc2626, #b91c1c);
  border-color: #b91c1c;
  color: #fff;
}
.btn.btn-danger:hover:not(:disabled) {
  background: linear-gradient(135deg, #b91c1c, #991b1b);
}
.btn.btn-danger:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.enhance-hit-bar {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 12px;
  margin: 8px 0 16px;
  padding: 12px;
  background: var(--surface-2);
  border: 1px solid var(--stroke);
  border-radius: 6px;
}
.hit-cell { text-align: center; }
.hit-num {
  font-size: 22px;
  font-weight: 600;
  color: var(--accent);
  font-variant-numeric: tabular-nums;
  line-height: 1.2;
}
.hit-label {
  font-size: 11px;
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

.clear-strategy {
  border: 1px solid var(--stroke);
  border-radius: 6px;
  padding: 12px;
  margin-bottom: 16px;
  background: var(--surface-1);
}
.cs-title {
  font-size: 12px;
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
  font-size: 12px;
  color: var(--text-2);
  cursor: pointer;
}
.cs-opt input[type="radio"] {
  margin: 0 4px 0 0;
  cursor: pointer;
}
.cs-num {
  width: 56px;
  font-size: 12px;
  padding: 2px 6px;
  border: 1px solid var(--stroke);
  border-radius: 3px;
  background: var(--surface-2);
  color: var(--text-1);
  text-align: center;
  font-variant-numeric: tabular-nums;
}
.cs-hint {
  font-size: 11px;
  color: var(--text-3);
  margin-top: 8px;
}

.recent-enhanced {
  border: 1px solid var(--stroke);
  border-radius: 6px;
  padding: 12px;
  background: var(--surface-1);
}
.re-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-2);
  margin-bottom: 8px;
}
.re-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
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
  color: var(--text-1);
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
  font-size: 10px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
  margin-top: 1px;
}
.re-hit {
  display: inline-block;
  min-width: 22px;
  padding: 1px 6px;
  border-radius: 3px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
.re-hit.on  { background: rgba(99, 102, 241, 0.12); color: var(--accent); }
.re-hit.off { background: var(--surface-2); color: var(--text-3); }
.re-empty {
  padding: 16px;
  text-align: center;
  color: var(--text-3);
  font-size: 12px;
}

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
  border-radius: 2px;
  background: var(--stroke);
  overflow: hidden;
}

.ocr-progress-fill {
  height: 100%;
  border-radius: 2px;
  background: var(--accent);
  transition: width 0.2s ease;
}

.ocr-progress-text {
  font-size: 12px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
</style>
