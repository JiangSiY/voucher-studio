<script setup lang="ts">
/**
 * 设置页骨架（Tab 分区 + 统一保存）：
 * 各分区逻辑在 components/settings/*Tab.vue，cfg 为共享编辑缓冲（保存时整体提交）。
 * Tab 用 v-show 保持挂载：与拆分前一致——进页即加载各分区数据，切换不丢状态。
 */
import { onMounted, reactive, ref } from "vue";
import { api, type AppConfig } from "../api/client";
import { toast, toastError } from "../stores/toast";
import { useConfigStore } from "../stores/config";
import GeneralTab from "../components/settings/GeneralTab.vue";
import NamingTab from "../components/settings/NamingTab.vue";
import OcrTab from "../components/settings/OcrTab.vue";
import EnhanceTab from "../components/settings/EnhanceTab.vue";
import SecretsTab from "../components/settings/SecretsTab.vue";
import BackupTab from "../components/settings/BackupTab.vue";

const configStore = useConfigStore();

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
      autoBackupOnClose: false,
      keepDaily: 7,
      keepWeekly: 4,
      keepMonthly: 6,
      secondaryDir: "",
      encrypted: false,
      webdav: { enabled: false, url: "", username: "", passwordSet: false, remoteRetention: 10, onceDaily: false },
    },
    print: {
      title: "凭证日报",
      cardStyle: "standard",
      columns: 3,
      includeImages: true,
      groupByDay: true,
      dailyTemplate: "compact",
      preferEnhancedImages: true,
    },
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
    voucher: { descriptionTemplates: [] },
  };
}

const saving = ref(false);

async function load() {
  try {
    const c = await api.getConfig();
    Object.assign(cfg, c);
    // v0.10 兜底：极旧配置文件可能无 voucher 段（后端 serde default 已保证，此处双保险）
    if (!cfg.voucher) cfg.voucher = { descriptionTemplates: [] };
  } catch (e) {
    toastError(e);
  }
}

async function save() {
  saving.value = true;
  try {
    await configStore.save(JSON.parse(JSON.stringify(cfg)));
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
  <div class="tab-nav">
    <button v-for="t in tabs" :key="t.key" class="tab-item" :class="{ active: tab === t.key }" @click="tab = t.key">
      {{ t.label }}
    </button>
  </div>

  <GeneralTab v-show="tab === 'general'" :cfg="cfg" />
  <NamingTab v-show="tab === 'naming'" :cfg="cfg" />
  <OcrTab v-show="tab === 'ocr'" :cfg="cfg" />
  <EnhanceTab v-show="tab === 'enhance'" :cfg="cfg" />
  <SecretsTab v-show="tab === 'secrets'" />
  <BackupTab v-show="tab === 'backup'" :cfg="cfg" />

  <div style="display: flex; justify-content: flex-end; margin-top: 8px">
    <button class="btn btn-primary" :disabled="saving" @click="save">{{ saving ? "保存中…" : "保存设置" }}</button>
  </div>
</template>
