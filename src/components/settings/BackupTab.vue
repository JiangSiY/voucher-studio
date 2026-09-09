<script setup lang="ts">
/**
 * 设置 → 备份与同步：自动备份、异盘双写、关闭前备份、备份加密（v0.13）、
 * 分层保留 GFS、WebDAV 云备份与连接测试。
 */
import { ref } from "vue";
import { api, errMsg, type AppConfig } from "../../api/client";
import FluentIcon from "../FluentIcon.vue";
import { toast, toastError } from "../../stores/toast";
import { useConfigStore } from "../../stores/config";
import { refreshSecrets, secretDrafts } from "../../composables/secretDrafts";

const props = defineProps<{ cfg: AppConfig }>();
const configStore = useConfigStore();

async function pickSecondaryDir() {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const dir = await open({ directory: true });
  if (typeof dir === "string") {
    props.cfg.backup.secondaryDir = dir;
  }
}

// WebDAV 密码写入凭据库后同步 passwordSet 标记（草稿在「密钥管理」Tab 录入）
async function saveWebdavPassword() {
  const alias = "webdav_password";
  const value = secretDrafts[alias]?.trim();
  if (!value) {
    toast.warning("请先在「密钥管理」页输入 WebDAV 密码");
    props.cfg.backup.webdav.passwordSet = true;
    return;
  }
  try {
    await api.setSecret(alias, value);
    secretDrafts[alias] = "";
    await refreshSecrets();
    toast.success("已写入系统凭据库");
  } catch (e) {
    toastError(e);
  }
  props.cfg.backup.webdav.passwordSet = true;
}

// WebDAV 连接测试：先保存当前配置（后端读已落盘配置），再 PROPFIND 远端
const webdavTesting = ref(false);
const webdavTestResult = ref<{ ok: boolean; text: string } | null>(null);

async function testWebdavConnection() {
  if (webdavTesting.value) return;
  webdavTesting.value = true;
  webdavTestResult.value = null;
  try {
    await configStore.save(JSON.parse(JSON.stringify(props.cfg)));
    const count = await api.testWebdav();
    webdavTestResult.value = { ok: true, text: `连接成功，远端已有 ${count} 份备份` };
    toast.success(`WebDAV 连接成功，远端已有 ${count} 份备份`);
  } catch (e) {
    webdavTestResult.value = { ok: false, text: errMsg(e) };
    toastError(e);
  } finally {
    webdavTesting.value = false;
  }
}
</script>

<template>
  <div class="fluent-panel">
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
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 4px 0 12px">
        <div>
          <div style="font-size: 13px; font-weight: 600">关闭前自动备份</div>
          <div style="font-size: 12px; color: var(--text-3)">每次退出应用前执行一次本地快速备份（跳过 WebDAV/异盘双写，避免关闭卡顿）；60 分钟内已备份则跳过</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="cfg.backup.autoBackupOnClose" />
          <span class="slider"></span>
        </label>
      </div>

      <!-- v0.13：备份加密（AES-256；密码存凭据管理器） -->
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 4px 0 12px">
        <div>
          <div style="font-size: 13px; font-weight: 600">备份加密 <FluentIcon name="lock" :size="13" /></div>
          <div style="font-size: 12px; color: var(--text-3)">
            开启后备份 zip 以 AES-256 加密（本地 / 异盘 / WebDAV 云端全覆盖），恢复时自动解密。
            密码在「密钥管理 → 备份加密密码」设置，<strong style="color: var(--warning)">忘记密码将无法恢复备份，请牢记</strong>。
            旧明文备份仍可直接恢复。
          </div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="cfg.backup.encrypted" />
          <span class="slider"></span>
        </label>
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
          <div class="desc">密码只存系统凭据库：在「密钥管理」页输入 WebDAV 密码后点这里落库。</div>
        </div>
        <div class="field">
          <label>远端保留份数</label>
          <input v-model.number="cfg.backup.webdav.remoteRetention" type="number" min="0" class="input"
            style="max-width: 110px" />
          <div class="desc">超出删远端最旧（0 = 不清理）。备份文件名含时间戳，按名排序即按时间。</div>
        </div>
      </div>
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 0">
        <div>
          <div style="font-size: 13px; font-weight: 600">云端每日一份（流量保护）</div>
          <div style="font-size: 12px; color: var(--text-3)">坚果云等免费服务有上传流量配额；开启后当日云端已有备份则跳过上传（本地备份不受影响）</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="cfg.backup.webdav.onceDaily" />
          <span class="slider"></span>
        </label>
      </div>
      <div style="display: flex; align-items: center; gap: 12px; margin-top: 12px">
        <button class="btn" :disabled="webdavTesting" @click="testWebdavConnection">
          {{ webdavTesting ? "测试中…" : "测试连接" }}
        </button>
        <div v-if="webdavTestResult" class="callout"
          :class="webdavTestResult.ok ? 'success' : 'danger'" style="margin: 0; flex: 1">
          <span class="ci">{{ webdavTestResult.ok ? "●" : "!" }}</span>
          <div class="ct">{{ webdavTestResult.text }}</div>
        </div>
      </div>
    </div>
  </div>
</template>
