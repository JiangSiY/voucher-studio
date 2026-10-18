<script setup lang="ts">
/**
 * 设置 → 通用：界面主题、数据目录、打印默认值。
 * draftCfg 为共享编辑缓冲（stores/settingsDraft，体检 #22 单向流改造）：
 * Tab 不再接收可变 cfg prop，而是读写共享 store 状态，保存由设置页整体提交。
 */
import { computed, ref } from "vue";
import type { Update } from "@tauri-apps/plugin-updater";
import { UI_SCALE_OPTIONS } from "../../config-defaults";
import { storeToRefs } from "pinia";
import { api } from "../../api/client";
import { toastError } from "../../stores/toast";
import { useConfigStore } from "../../stores/config";
import { useSettingsDraft } from "../../stores/settingsDraft";

const configStore = useConfigStore();
const { cfg: draftCfg } = storeToRefs(useSettingsDraft());
const dataDir = computed(() => configStore.appInfo?.dataDir ?? "");
const appVersion = computed(() => configStore.appInfo?.version ?? "");

/** 自动更新（挂账 #39）：打包版查 GitHub Release 的 latest.json；开发模式不连更新源 */
type UpdateState =
  "idle" | "checking" | "latest" | "available" | "installing" | "installed" | "dev" | "error";
const updateState = ref<UpdateState>("idle");
const availableVersion = ref("");
const availableNotes = ref("");
const progress = ref(0);
const errorText = ref("");
const checking = computed(() => updateState.value === "checking");
const installing = computed(() => updateState.value === "installing");
let pendingUpdate: Update | null = null;

async function checkUpdate() {
  if (import.meta.env.DEV) {
    updateState.value = "dev";
    return;
  }
  updateState.value = "checking";
  errorText.value = "";
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const up = await check();
    if (!up) {
      updateState.value = "latest";
      return;
    }
    pendingUpdate = up;
    availableVersion.value = up.version;
    availableNotes.value = up.body ?? "";
    updateState.value = "available";
  } catch (e) {
    errorText.value = e instanceof Error ? e.message : String(e);
    updateState.value = "error";
  }
}

async function installUpdate() {
  if (!pendingUpdate) return;
  updateState.value = "installing";
  progress.value = 0;
  try {
    let total = 0;
    let received = 0;
    await pendingUpdate.downloadAndInstall((ev) => {
      if (ev.event === "Started") {
        total = ev.data.contentLength ?? 0;
      } else if (ev.event === "Progress") {
        received += ev.data.chunkLength;
        if (total > 0) progress.value = Math.min(99, Math.round((received / total) * 100));
      } else if (ev.event === "Finished") {
        progress.value = 100;
      }
    });
    updateState.value = "installed";
    await api.restartApp();
  } catch (e) {
    errorText.value = e instanceof Error ? e.message : String(e);
    updateState.value = "error";
  }
}

async function openDataDir() {
  try {
    await api.openPath(dataDir.value);
  } catch (e) {
    toastError(e);
  }
}
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-body">
      <div class="field">
        <label>界面主题</label>
        <select v-model="draftCfg.appearance.theme" class="input settings-narrow">
          <option value="system">跟随系统</option>
          <option value="light">浅色</option>
          <option value="dark">深色</option>
        </select>
      </div>
      <!-- 界面字号（用户视力需要，见设计规范 §1.3）：保存后立即生效，无需重启 -->
      <div class="field">
        <label>界面字号</label>
        <select v-model.number="draftCfg.appearance.uiScale" class="input settings-narrow">
          <option v-for="o in UI_SCALE_OPTIONS" :key="o.value" :value="o.value">
            {{ o.label }}
          </option>
        </select>
        <div class="desc">
          控制正文字号与控件高度（标准 / 大 / 更大）。默认「大」——正文 17px、辅助 15px 左右；
          若仍看不清请选「更大」，或告诉我再往上调。
        </div>
      </div>
      <div class="field">
        <label>数据目录</label>
        <div class="settings-inline">
          <input :value="dataDir" class="input mono settings-grow" disabled />
          <button class="btn" @click="openDataDir">打开目录</button>
        </div>
        <div class="desc">所有数据（数据库、附件图片、备份）都保存在本机此目录下。</div>
      </div>

      <div class="field settings-section">
        <label>打印默认值</label>
        <div class="form-grid">
          <div class="field">
            <label>日报标题</label>
            <input v-model="draftCfg.print.title" class="input" placeholder="如：凭证日报" />
          </div>
        </div>
        <div class="form-grid settings-mt-2">
          <div class="field">
            <label>日报模板（v0.8）</label>
            <select v-model="draftCfg.print.dailyTemplate" class="input">
              <option value="compact">省纸紧凑 · 横幅 + 12pt 汇总表 + 竖票 3×3（推荐）</option>
              <option value="archive">档案装订 · 独立封面 + 12pt 汇总表 + 图片 2×2</option>
            </select>
            <div class="desc">
              两套模板均为 12pt（小四）表格、按行高自动分页；图片等比缩放完整显示不裁内容。
            </div>
          </div>
          <div class="field">
            <label>打印优先增强版图片</label>
            <div class="settings-inline settings-mt-sm">
              <label class="toggle">
                <input type="checkbox" v-model="draftCfg.print.preferEnhancedImages" />
                <span class="slider"></span>
              </label>
              <span class="settings-caption">扫描件用矫正后增强版；关闭则始终用原图</span>
            </div>
          </div>
        </div>
        <div class="settings-inline settings-mt-2">
          <label class="toggle">
            <input type="checkbox" v-model="draftCfg.print.includeImages" />
            <span class="slider"></span>
          </label>
          <span class="settings-caption">附带附件图片页（竖票 3×3 / 横票 2×2 自动分桶）</span>
        </div>
        <div class="desc">进入「打印与导出」时按此默认值初始化；页内的临时调整不影响这里。</div>
      </div>

      <div class="field settings-section">
        <label>软件更新</label>
        <div class="settings-inline">
          <span class="settings-caption">当前版本 v{{ appVersion }}</span>
          <button class="btn" :disabled="checking || installing" @click="checkUpdate">
            {{ checking ? "检查中…" : "检查更新" }}
          </button>
          <button
            v-if="updateState === 'available'"
            class="btn btn-primary"
            :disabled="installing"
            @click="installUpdate"
          >
            {{ installing ? `正在下载 ${progress}%` : "下载并安装" }}
          </button>
        </div>
        <div v-if="updateState === 'available'" class="desc">
          新版本 v{{ availableVersion }} 可用{{
            availableNotes ? `：${availableNotes}` : ""
          }}（安装完成后应用会自动重启）
        </div>
        <div v-else-if="updateState === 'latest'" class="desc">已是最新版本。</div>
        <div v-else-if="updateState === 'dev'" class="desc">
          开发模式不检查更新（打包版才连更新源）。
        </div>
        <div v-else-if="updateState === 'installed'" class="desc">更新已安装，正在重启…</div>
        <div v-else-if="updateState === 'error'" class="desc">检查更新失败：{{ errorText }}</div>
        <div v-else class="desc">
          从 GitHub Release 的 latest.json 取更新（需签名校验）；不想用自动更新时也可直接下载新的
          dmg 覆盖安装。
        </div>
      </div>
    </div>
  </div>
</template>
