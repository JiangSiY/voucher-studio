<script setup lang="ts">
/**
 * 设置 → 通用：界面主题、数据目录、打印默认值。
 * draftCfg 为共享编辑缓冲（stores/settingsDraft，体检 #22 单向流改造）：
 * Tab 不再接收可变 cfg prop，而是读写共享 store 状态，保存由设置页整体提交。
 */
import { computed } from "vue";
import { UI_SCALE_OPTIONS } from "../../config-defaults";
import { storeToRefs } from "pinia";
import { api } from "../../api/client";
import { toastError } from "../../stores/toast";
import { useConfigStore } from "../../stores/config";
import { useSettingsDraft } from "../../stores/settingsDraft";

const configStore = useConfigStore();
const { cfg: draftCfg } = storeToRefs(useSettingsDraft());
const dataDir = computed(() => configStore.appInfo?.dataDir ?? "");

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
          <option v-for="o in UI_SCALE_OPTIONS" :key="o.value" :value="o.value">{{ o.label }}</option>
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
            <div class="desc">两套模板均为 12pt（小四）表格、按行高自动分页；图片等比缩放完整显示不裁内容。</div>
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
    </div>
  </div>
</template>
