<script setup lang="ts">
/** 备份与恢复（restore_panel）：备份列表 / 立即备份 / 校验 / 演练 / 恢复（重启生效） / WebDAV 上传与下载 */
import { onMounted, onUnmounted, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { safeListen } from "../composables/safeListen";
import { api, errMsg, type BackupInfo, type DrillReport, type RestoreSummary, type WebdavBackupRow } from "../api/client";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import EmptyState from "../components/EmptyState.vue";
import FluentIcon from "../components/FluentIcon.vue";
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
    // 备份含 WebDAV 自动上传：本地 + 云端列表都要刷新
    await Promise.all([load(), loadRemote()]);
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
  // G3-4：演练开始即清旧报告——失败/进行中都不残留上一次的「演练通过」
  drill.value = null;
  try {
    drill.value = await api.drillBackup();
    toast.success("演练通过：备份可完整恢复");
  } catch (e) {
    // G3-4：后端术语翻译成用户语言（原样弹「IO 错误: Invalid checksum」不友好）
    const msg = errMsg(e);
    if (/checksum|校验/i.test(msg)) {
      toast.danger("演练失败：备份文件校验不一致（可能已损坏），请换用其他备份或重新备份");
    } else {
      toastError(e);
    }
  } finally {
    drilling.value = false;
  }
}

async function uploadWebdav(b: BackupInfo) {
  try {
    await api.uploadBackup(b.path);
    toast.success(`已上传：${b.fileName}`);
    await loadRemote();
  } catch (e) {
    toastError(e);
  }
}

// ---------- v0.9 导出完整备份（快照 + 附件包 → 单个全量 zip，便于拷走） ----------

const exporting = ref("");

async function exportFull(b: BackupInfo) {
  if (exporting.value) return;
  exporting.value = b.fileName;
  try {
    const path = await api.exportFullBackup(b.path);
    const name = path.split(/[\\/]/).pop() ?? path;
    toast.success(`已导出完整备份：${name}`, {
      label: "打开目录",
      onClick: () => {
        // 去掉最后一段文件名，**保留原分隔符**：此前用 `join("\\")`，在 macOS 上会拼出
        // `\Users\x\...` 这种非法路径（NSWorkspace 打不开），且 `void` 把失败吞掉了。
        // 2026-09-23 审计 A7。
        const dir = path.replace(/[\\/][^\\/]*$/, "");
        void api.openPath(dir).catch((e) => toastError(e));
      },
    });
    await load();
  } catch (e) {
    toastError(e);
  } finally {
    exporting.value = "";
  }
}

// ---------- WebDAV 远端备份：列表 / 下载（下载后进入本地列表即可校验/恢复） ----------

const remoteBackups = ref<WebdavBackupRow[]>([]);
const remoteLoading = ref(false);
const remoteError = ref("");
const downloading = ref("");

async function loadRemote() {
  remoteLoading.value = true;
  remoteError.value = "";
  try {
    remoteBackups.value = await api.listWebdavBackups();
  } catch (e) {
    // 未启用/未配置是正常态：内联提示而不是弹错误
    remoteError.value = errMsg(e);
    remoteBackups.value = [];
  } finally {
    remoteLoading.value = false;
  }
}

/** 远端文件名是否已存在于本地列表（同名即同一份，幂等跳过下载） */
function isLocal(fileName: string) {
  return backups.value.some((b) => b.fileName === fileName);
}

async function downloadRemote(fileName: string) {
  if (downloading.value) return;
  downloading.value = fileName;
  try {
    await api.downloadWebdavBackup(fileName);
    toast.success(`已下载到本地备份目录：${fileName}，可在上方列表校验或恢复`);
    await load();
  } catch (e) {
    toastError(e);
  } finally {
    downloading.value = "";
  }
}

// 自动备份后台完成时，新备份应出现在可恢复列表
let unlistenBackupDone: UnlistenFn | null = null;

onMounted(async () => {
  load();
  loadRemote();
  // 自动/手动备份完成（backup:done 由手动备份也触发）：云端可能新增上传副本
  unlistenBackupDone = await safeListen("backup:done", () => {
    load();
    loadRemote();
  });
});

onUnmounted(() => unlistenBackupDone?.());
</script>

<template>
  <div class="callout warning">
    <span class="ci"><FluentIcon name="warning" :size="18" /></span>
    <div class="ct">
      <strong>数据是生命线：</strong>恢复会用备份内容覆盖当前数据库与配置；<strong>备份含附件时</strong>才会同时替换附件目录（
      快照只含原图，增强版属可重建缓存、不入备份）。流程为「就绪 → 重启应用 → 生效」。
      恢复前建议先做一次当前数据的备份。
    </div>
  </div>

  <div v-if="staged" class="callout info">
    <span class="ci"><FluentIcon name="checkCircle" :size="18" /></span>
    <div class="ct">
      <strong>备份已就绪：</strong>{{ staged.batch }}（创建于 {{ staged.createdAt }}，
      <template v-if="staged.attachmentsIncluded">含 {{ staged.attachmentCount }} 个附件</template>
      <template v-else><strong>不含附件实体</strong></template>
      {{ staged.checksumsVerified ? "，已通过完整性校验" : "" }}）。
      <button class="btn btn-sm btn-primary" style="margin-left: 8px" @click="restart">立即重启应用</button>
      <span v-if="staged.attachmentsIncluded" style="display: block; font-size: 12px; margin-top: 4px; color: var(--text-3)">
        重启后自动替换数据库与附件目录；如需取消，重启前删除数据目录下 voucher.db.pending / attachments.pending 即可。
      </span>
      <span v-else style="display: block; font-size: 12px; margin-top: 4px; color: var(--warning)">
        该备份不含附件实体：重启后只替换数据库与配置，附件目录保持不变。凭证信息可恢复，图片将自动回退到原图显示；
        如需增强版，可在 设置 → 扫描矫正 一键重建。如需取消，重启前删除数据目录下 voucher.db.pending 即可。
      </span>
    </div>
  </div>

  <div v-if="drill" class="callout success">
    <span class="ci"><FluentIcon name="check" :size="18" /></span>
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
        <button class="btn btn-sm" @click="pickAndBackup"><FluentIcon name="search" :size="14" />校验外部备份</button>
        <button class="btn btn-sm" :disabled="drilling || !backups.length" @click="runDrill">
          <FluentIcon v-if="!drilling" name="flask" :size="14" />{{ drilling ? "演练中…" : "演练恢复" }}
        </button>
        <button class="btn btn-sm btn-primary" :disabled="busy" @click="backupNow">
          <FluentIcon v-if="!busy" name="save" :size="14" />{{ busy ? "备份中…" : "立即备份" }}
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
              <td>
                <span class="mono" style="color: var(--text-2)">{{ b.fileName }}</span>
                <!-- v0.9：快照标记（附件在同目录附件包里，恢复/演练自动合成） -->
                <span v-if="b.kind === 'snapshot'" class="tag tag-info" style="margin-left: 6px"
                  title="增量快照：附件存于共享附件包，恢复时自动合成完整备份">快照</span>
                <span v-else class="tag tag-muted" style="margin-left: 6px" title="单文件完整备份">全量</span>
              </td>
              <td>{{ fmtDate(b.createdAt) || "—" }}</td>
              <td>{{ fmtSize(b.size) }}</td>
              <td>{{ b.attachmentCount || "—" }}</td>
              <td>
                <div class="action-row">
                  <button class="icon-btn" title="恢复此备份" @click="askRestore(b)"><FluentIcon name="undo" :size="15" /></button>
                  <button class="icon-btn" title="上传到 WebDAV" @click="uploadWebdav(b)"><FluentIcon name="cloudUp" :size="15" /></button>
                  <!-- v0.9：快照可导出为单个完整 zip（拷走一份不依赖附件包） -->
                  <button v-if="b.kind === 'snapshot'" class="icon-btn" :title="exporting === b.fileName ? '导出中…' : '导出完整备份 zip（合成单文件，便于拷走）'"
                    :disabled="exporting === b.fileName" @click="exportFull(b)"><FluentIcon name="download" :size="15" /></button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <EmptyState v-else icon="save" title="还没有备份" desc="点击右上角「立即备份」创建第一份备份">
        <button class="btn btn-primary" :disabled="busy" @click="backupNow"><FluentIcon v-if="!busy" name="save" :size="14" />立即备份</button>
      </EmptyState>
    </div>
  </div>

  <!-- WebDAV 云端备份：异地副本，下载后进入本地列表即可校验/恢复 -->
  <div class="fluent-panel">
    <div class="fluent-panel-header">
      <h3>WebDAV 云端备份（{{ remoteBackups.length }}）</h3>
      <div style="display: flex; gap: 8px">
        <button class="btn btn-sm" :disabled="remoteLoading" @click="loadRemote">
          <FluentIcon name="refresh" :size="14" />{{ remoteLoading ? "加载中…" : "刷新远端" }}
        </button>
      </div>
    </div>
    <div class="fluent-panel-body" style="padding: 0">
      <div v-if="remoteError" class="callout warning" style="margin: 12px">
        <span class="ci"><FluentIcon name="warning" :size="18" /></span>
        <div class="ct">
          无法读取云端列表：{{ remoteError }}。
          请在「设置 → 备份与同步」检查 WebDAV 配置并点「测试连接」。
        </div>
      </div>
      <div class="dtable-wrap" v-else-if="remoteBackups.length">
        <table class="dtable">
          <thead>
            <tr>
              <th>远端备份文件</th>
              <th>状态</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="r in remoteBackups" :key="r.fileName">
              <td><span class="mono" style="color: var(--text-2)">{{ r.fileName }}</span></td>
              <td>
                <span v-if="isLocal(r.fileName)" style="color: var(--success)">已在本地</span>
                <span v-else style="color: var(--text-3)">仅云端</span>
              </td>
              <td>
                <div class="action-row">
                  <button class="icon-btn" title="下载到本地备份目录"
                    :disabled="downloading !== '' || isLocal(r.fileName)" @click="downloadRemote(r.fileName)">
                    <FluentIcon name="download" :size="15" />
                  </button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <EmptyState v-else-if="!remoteLoading" icon="cloud" title="云端还没有备份"
        desc="在「设置 → 备份与同步」启用 WebDAV 后，每次备份会自动上传云端；也可在上方列表手动补传" />
    </div>
  </div>

  <ConfirmDialog :visible="confirmVisible" title="恢复备份"
    :body="`将用选中的备份覆盖当前数据库、配置与附件目录。这一步只把备份写成「待恢复文件」，点「立即重启应用」后才真正替换当前数据；重启前删掉待恢复文件即可取消。确定继续吗？`"
    confirm-text="写入待恢复文件" @confirm="doRestore" @cancel="confirmVisible = false" />
</template>
