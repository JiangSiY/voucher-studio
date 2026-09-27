<script setup lang="ts">
/**
 * 设置页骨架（Tab 分区 + 统一保存）：
 * 各分区逻辑在 components/settings/*Tab.vue；编辑缓冲为独立 store
 * `settingsDraft.cfg`（体检 #22 单向流改造：不再把可变 cfg 作为 prop 下发，
 * Tab 直接读写共享 store 状态），保存时整体提交。
 * Tab 用 v-show 保持挂载：与拆分前一致——进页即加载各分区数据，切换不丢状态。
 */
import { onMounted, ref } from "vue";
import { api, type AppConfig } from "../api/client";
import { toast, toastError } from "../stores/toast";
import { useConfigStore } from "../stores/config";
import { useSettingsDraft } from "../stores/settingsDraft";
import GeneralTab from "../components/settings/GeneralTab.vue";
import NamingTab from "../components/settings/NamingTab.vue";
import OcrTab from "../components/settings/OcrTab.vue";
import EnhanceTab from "../components/settings/EnhanceTab.vue";
import SecretsTab from "../components/settings/SecretsTab.vue";
import BackupTab from "../components/settings/BackupTab.vue";

const configStore = useConfigStore();
const draft = useSettingsDraft();

const tabs = [
  { key: "general", label: "通用" },
  { key: "naming", label: "凭证参数" },
  { key: "ocr", label: "OCR" },
  { key: "enhance", label: "扫描矫正" },
  { key: "secrets", label: "密钥管理" },
  { key: "backup", label: "备份与同步" },
] as const;
type TabKey = (typeof tabs)[number]["key"];
const tab = ref<TabKey>("general");

const saving = ref(false);

async function load() {
  try {
    const c = await api.getConfig();
    draft.replace(c);
  } catch (e) {
    toastError(e);
  }
}

async function save() {
  saving.value = true;
  try {
    // 直接把共享编辑缓冲交给后端（reactive 代理会在 invoke 序列化时拍快照）。
    // 但 `appearance.sidebarWidth` 不属于本页 UI（由侧边栏拖拽维护），草稿里是进页时的旧快照：
    // 用 store 的当前值覆盖，否则「拖过侧边栏 → 改任一设置 → 保存」会把宽度回退。
    const payload: AppConfig = {
      ...draft.cfg,
      appearance: {
        ...draft.cfg.appearance,
        sidebarWidth:
          configStore.config?.appearance.sidebarWidth ?? draft.cfg.appearance.sidebarWidth,
      },
    };
    await configStore.save(payload);
    toast.success("设置已保存");
  } catch (e) {
    toastError(e);
  } finally {
    saving.value = false;
  }
}

onMounted(load);
</script>

<template>
  <!-- v0.18：设置的分区切换改用分段控件（规范 §1.5；macOS 的设置/视图切换惯例） -->
  <div class="segmented" role="tablist">
    <button
      v-for="t in tabs"
      :key="t.key"
      class="seg-item"
      role="tab"
      :aria-selected="tab === t.key"
      :class="{ active: tab === t.key }"
      @click="tab = t.key"
    >
      {{ t.label }}
    </button>
  </div>

  <GeneralTab v-show="tab === 'general'" />
  <NamingTab v-show="tab === 'naming'" />
  <OcrTab v-show="tab === 'ocr'" />
  <EnhanceTab v-show="tab === 'enhance'" />
  <SecretsTab v-show="tab === 'secrets'" />
  <BackupTab v-show="tab === 'backup'" />

  <div style="display: flex; justify-content: flex-end; margin-top: 8px">
    <button class="btn btn-primary" :disabled="saving" @click="save">{{ saving ? "保存中…" : "保存设置" }}</button>
  </div>
</template>
