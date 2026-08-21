<script setup lang="ts">
/** 备份与恢复（restore_panel）：备份列表 / 立即备份 / 校验 / 演练 / 恢复（重启生效） / WebDAV 上传 */
import { onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, type BackupInfo, type DrillReport, type RestoreSummary } from "../api/client";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import EmptyState from "../components/EmptyState.vue";
import { fmtDate, fmtSize } from "../composables/format";
import { toast, toastError } from "../stores/toast";

const backups = ref<BackupInfo[]>([]);
const busy = ref(false);
const drilling = ref(false);
const drill = ref<DrillReport | null>(null);

const confirmVisible = ref(false);
let pendingRestore: BackupInfo | null = null;
const staged = ref<RestoreSummary | null>(null);

async function load() {
  try {
    backups.value = await api.listBackups();
  } catch (e) {
    toastError(e);
  }
}

async function backupNow() {
  busy.value = true;
  try {
    const info = await api.runBackup();
    if (info.warnings?.length) {
      toast.warning(`备份完成（本地），但副本存在问题：${info.warnings.join("；")}`);
    } else {
      toast.success(`备份完成：${info.fileName}（含 ${info.attachmentCount} 个附件）`);
    }
    await load();
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = false;
  }
}

async function pickAndBackup() {
  // 从外部选择一个备份 zip 校验（比如从网盘下载回来的）
  const { open } = await import("@tauri-apps/plugin-dialog");
  const file = await open({ filters: [{ name: "Voucher Studio 备份", extensions: ["zip"] }] });
  if (typeof file !== "string") return;
  try {
    const summary = await api.inspectBackup(file);
    toast.success(`有效备份：创建于 ${summary.createdAt}，含 ${summary.restoredFiles.length} 个文件`);
  } catch (e) {
    toastError(e);
  }
}

function askRestore(b: BackupInfo) {
  pendingRestore = b;
  confirmVisible.value = true;
}

async function doRestore() {
  confirmVisible.value = false;
  if (!pendingRestore) return;
  try {
    staged.value = await api.restoreBackup(pendingRestore.path);
    toast.success("备份已就绪，重启应用后生效");
  } catch (e) {
    toastError(e);
  }
}

async function restart() {
  await api.restartApp();
}

async function runDrill() {
  drilling.value = true;
  try {
    drill.value = await api.drillBackup();
    toast.success("演练通过：备份可完整恢复");
  } catch (e) {
    toastError(e);
  } finally {
    drilling.value = false;
  }
}

async function uploadWebdav(b: BackupInfo) {
  try {
    await api.uploadBackup(b.path);
    toast.success(`已上传：${b.fileName}`);
  } catch (e) {
    toastError(e);
  }
}

// 自动备份后台完成时，新备份应出现在可恢复列表
let unlistenBackupDone: UnlistenFn | null = null;

onMounted(async () => {
  load();
  unlistenBackupDone = await listen("backup:done", () => load());
});

onUnmounted(() => unlistenBackupDone?.());
</script>

<template>
  <div class="callout warning">
    <span class="ci">⚠</span>
    <div class="ct">
      <strong>数据是生命线：</strong>恢复会用备份内容覆盖当前数据库、配置与附件，流程为「就绪 → 重启应用 → 生效」。
      恢复前建议先做一次当前数据的备份。
    </div>
  </div>

  <div v-if="staged" class="callout info">
    <span class="ci">✅</span>
    <div class="ct">
      <strong>备份已就绪：</strong>{{ staged.batch }}（创建于 {{ staged.createdAt }}，含
      {{ staged.attachmentCount }} 个附件{{ staged.checksumsVerified ? "，已通过完整性校验" : "" }}）。
      <button class="btn btn-sm btn-primary" style="margin-left: 8px" @click="restart">立即重启应用</button>
      <span style="display: block; font-size: 12px; margin-top: 4px; color: var(--text-3)">
        重启后自动替换数据库与附件目录；如需取消，重启前删除数据目录下 voucher.db.pending / attachments.pending 即可。
      </span>
    </div>
  </div>

  <div v-if="drill" class="callout success">
    <span class="ci">✓</span>
    <div class="ct">
      <strong>演练通过：</strong>{{ drill.batch }}（{{ fmtDate(drill.createdAt) || drill.createdAt }}）
      校验 {{ drill.filesChecked }} 个文件、{{ drill.attachmentCount }} 个附件，数据库可正常打开。
      <span style="display: block; font-size: 12px; margin-top: 4px; color: var(--text-3)">
        {{ drill.checksumsVerified ? "sha256 完整性校验全部通过" : "v1 备份：无校验和清单，未做完整性校验" }}
      </span>
    </div>
  </div>

  <div class="fluent-panel">
    <div class="fluent-panel-header">
      <h3>本地备份（{{ backups.length }}）</h3>
      <div style="display: flex; gap: 8px">
        <button class="btn btn-sm" @click="pickAndBackup">🔎 校验外部备份</button>
        <button class="btn btn-sm" :disabled="drilling || !backups.length" @click="runDrill">
          {{ drilling ? "演练中…" : "🧪 演练恢复" }}
        </button>
        <button class="btn btn-sm btn-primary" :disabled="busy" @click="backupNow">
          {{ busy ? "备份中…" : "💾 立即备份" }}
        </button>
      </div>
    </div>
    <div class="fluent-panel-body" style="padding: 0">
      <div class="dtable-wrap" v-if="backups.length">
        <table class="dtable">
          <thead>
            <tr>
              <th>备份文件</th>
              <th>创建时间</th>
              <th>大小</th>
              <th>附件</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="b in backups" :key="b.path">
              <td><span class="mono" style="color: var(--text-2)">{{ b.fileName }}</span></td>
              <td>{{ fmtDate(b.createdAt) || "—" }}</td>
              <td>{{ fmtSize(b.size) }}</td>
              <td>{{ b.attachmentCount || "—" }}</td>
              <td>
                <div class="action-row">
                  <button class="icon-btn" title="恢复此备份" @click="askRestore(b)">⏪</button>
                  <button class="icon-btn" title="上传到 WebDAV" @click="uploadWebdav(b)">☁</button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <EmptyState v-else icon="💾" title="还没有备份" desc="点击右上角「立即备份」创建第一份备份">
        <button class="btn btn-primary" :disabled="busy" @click="backupNow">💾 立即备份</button>
      </EmptyState>
    </div>
  </div>

  <ConfirmDialog :visible="confirmVisible" title="恢复备份"
    :body="`将用选中的备份覆盖当前数据库、配置与附件目录，恢复后当前数据将被替换。确定继续吗？`"
    confirm-text="就绪并重启" @confirm="doRestore" @cancel="confirmVisible = false" />
</template>
