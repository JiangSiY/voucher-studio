<script setup lang="ts">
/**
 * 设置（Tab 分区，对齐 UI 方案的配置页模式）：
 * 通用 / 凭证参数 / OCR / 密钥管理 / 备份与同步 / 微信监听
 */
import { computed, onMounted, reactive, ref } from "vue";
import { api, type AppConfig, type LocalOcrStatus, type SecretStatus } from "../api/client";
import { toast, toastError } from "../stores/toast";
import { useConfigStore } from "../stores/config";

const configStore = useConfigStore();

const tabs = [
  { key: "general", label: "通用" },
  { key: "naming", label: "凭证参数" },
  { key: "ocr", label: "OCR" },
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
      retention: 30,
      webdav: { enabled: false, url: "", username: "", passwordSet: false },
    },
    wechat: { enabled: false, watchDir: "", mode: "events" },
    print: { title: "凭证整理单", cardStyle: "standard", columns: 3, includeImages: true },
  };
}

const secrets = ref<SecretStatus[]>([]);
const localOcr = ref<LocalOcrStatus | null>(null);
const saving = ref(false);
const templatePreview = ref("");
const templateError = ref("");

async function load() {
  try {
    const c = await api.getConfig();
    Object.assign(cfg, c);
    secrets.value = await api.listSecrets();
    localOcr.value = await api.getLocalOcrStatus();
    validateTemplate();
  } catch (e) {
    toastError(e);
  }
}

async function validateTemplate() {
  templateError.value = "";
  try {
    templatePreview.value = await api.validateNamingTemplate(cfg.naming.template);
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
    toast.success("设置已保存");
  } catch (e) {
    toastError(e);
  } finally {
    saving.value = false;
  }
}

// 密钥管理
const secretDrafts = reactive<Record<string, string>>({});

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
  const { open } = await import("@tauri-apps/plugin-dialog");
  const dir = await open({ directory: true });
  if (typeof dir === "string") cfg.wechat.watchDir = dir;
}

const dataDir = computed(() => configStore.appInfo?.dataDir ?? "");

async function openDataDir() {
  try {
    await api.openPath(dataDir.value);
  } catch (e) {
    toastError(e);
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
          ✓ 保存后新凭证编号形如：{{ templatePreview.replace("{seq}", "001").replace(/\{seq\}/g, "001") }}-001
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
          <div style="font-size: 12px; color: var(--text-3)">应用运行期间按间隔自动打包数据库与配置</div>
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
        </div>
        <div class="field">
          <label>本地保留份数</label>
          <input v-model.number="cfg.backup.retention" type="number" min="1" class="input" />
        </div>
      </div>

      <div style="border-top: 1px solid var(--stroke); margin: 8px 0 16px"></div>

      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 16px">
        <div>
          <div style="font-size: 13px; font-weight: 600">WebDAV 云备份</div>
          <div style="font-size: 12px; color: var(--text-3)">备份完成后自动上传到你的 WebDAV 网盘</div>
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
      <div class="field">
        <label>密码</label>
        <div style="display: flex; gap: 8px">
          <input :value="cfg.backup.webdav.passwordSet ? '••••••••（已保存在系统凭据库）' : ''" class="input" disabled
            style="flex: 1" />
          <button class="btn" @click="saveWebdavPassword">{{ cfg.backup.webdav.passwordSet ? "修改" : "设置" }}密码</button>
        </div>
        <div class="desc">密码只存系统凭据库，请在弹出输入后点「密钥管理」确认状态。</div>
      </div>
    </div>
  </div>

  <!-- 微信监听 -->
  <div v-else-if="tab === 'wechat'" class="fluent-panel">
    <div class="fluent-panel-body">
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 16px">
        <div>
          <div style="font-size: 13px; font-weight: 600">监听微信收件文件夹</div>
          <div style="font-size: 12px; color: var(--text-3)">文件夹里出现新图片时自动导入建卡（保存并重启应用后生效）</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="cfg.wechat.enabled" />
          <span class="slider"></span>
        </label>
      </div>
      <div class="field">
        <label>监听目录</label>
        <div style="display: flex; gap: 8px">
          <input v-model="cfg.wechat.watchDir" class="input mono" placeholder="如 C:\Users\你\Documents\WeChat Files\...\FileStorage\Image"
            style="flex: 1" />
          <button class="btn" @click="pickWatchDir">选择文件夹</button>
        </div>
        <div class="desc">建议指向微信「文件存储 → 图片」目录；同一张图片重复出现会自动去重。</div>
      </div>
    </div>
  </div>

  <div style="display: flex; justify-content: flex-end; margin-top: 8px">
    <button class="btn btn-primary" :disabled="saving" @click="save">{{ saving ? "保存中…" : "保存设置" }}</button>
  </div>
</template>
