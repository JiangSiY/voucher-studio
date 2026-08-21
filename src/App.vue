<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useRouter } from "vue-router";
import { api, type BackupInfo, type ImportSummary, type RecoveryReport } from "./api/client";
import { fmtDate } from "./composables/format";
import { useConfigStore } from "./stores/config";
import { toast } from "./stores/toast";
import ToastHost from "./components/ToastHost.vue";

const configStore = useConfigStore();
const router = useRouter();

// 全局事件告警（任何页面都可见）
let unlistenDone: UnlistenFn | null = null;
let unlistenFailed: UnlistenFn | null = null;
let unlistenWxImported: UnlistenFn | null = null;
let unlistenWxError: UnlistenFn | null = null;

// 崩溃恢复：上次会话异常退出时的健康报告对话框
const recovery = ref<RecoveryReport | null>(null);

onMounted(async () => {
  configStore.init();

  // 恢复检查：报告由后端启动时生成缓存（marker 检测 + 只读健康检查）
  try {
    const report = await api.getRecoveryReport();
    if (report.abnormalExit) {
      recovery.value = report;
    }
  } catch (e) {
    console.warn("恢复检查失败", e);
  }

  // 自动备份：失败必报；副本告警提示 3-2-1 不完整
  unlistenDone = await listen<BackupInfo>("backup:done", (e) => {
    if (e.payload.warnings?.length) {
      toast.warning(`自动备份完成，但副本存在问题：${e.payload.warnings.join("；")}`);
    }
  });
  unlistenFailed = await listen<string>("backup:failed", (e) => {
    toast.danger(`自动备份失败：${e.payload}`);
  });

  // 微信监听：导入结果与错误实时提示（事件内容哈希去重，重复图片静默跳过）
  unlistenWxImported = await listen<ImportSummary>("wechat:imported", (e) => {
    const s = e.payload;
    if (s.vouchers.length) {
      const parts = [`微信图片已入库：${s.vouchers.length} 张图片 → ${s.vouchers.length} 张凭证`];
      if (s.skippedDuplicates.length) parts.push(`跳过重复 ${s.skippedDuplicates.length} 张`);
      toast.success(parts.join(" · "));
    }
  });
  unlistenWxError = await listen<string>("wechat:error", (e) => {
    toast.danger(`微信监听异常：${e.payload}`);
  });
});

function dismissRecovery() {
  recovery.value = null;
}

function gotoRestore() {
  recovery.value = null;
  router.push("/restore");
}

onUnmounted(() => {
  unlistenDone?.();
  unlistenFailed?.();
  unlistenWxImported?.();
  unlistenWxError?.();
});
</script>

<template>
  <router-view />
  <ToastHost />

  <!-- 崩溃恢复对话框（recovery_window）：上次异常退出时展示数据健康报告 -->
  <div v-if="recovery" class="dialog-mask">
    <div class="dialog" role="alertdialog" aria-modal="true" aria-label="上次会话异常退出">
      <h3>检测到上次异常退出</h3>
      <div class="dialog-body">
        <p style="margin-bottom: 12px">
          上次会话（{{ recovery.lastSession ? fmtDate(recovery.lastSession.startedAt) : "时间未知" }}）未能正常关闭，
          可能由崩溃、强制结束进程或断电引起。已对数据做了只读健康检查：
        </p>
        <ul style="list-style: none; display: flex; flex-direction: column; gap: 8px; font-size: 13px">
          <li>
            <template v-if="recovery.dbOk">✅ 数据库完整性检查通过</template>
            <template v-else>
              ⚠️ 数据库完整性检查发现问题：
              <span class="mono" style="color: var(--danger)">{{
                recovery.integrityErrors.join("；") || "未知错误"
              }}</span>
            </template>
          </li>
          <li v-if="recovery.attachmentTotal > 0">
            <template v-if="recovery.attachmentMissingCount === 0">
              ✅ 附件文件核对通过（{{ recovery.attachmentTotal }} 张图片全部在位）
            </template>
            <template v-else>
              ⚠️ 有 {{ recovery.attachmentMissingCount }} 张附件图片文件缺失（如
              {{ recovery.attachmentMissing[0] }}）
            </template>
          </li>
          <li v-else>ℹ️ 尚无附件图片</li>
          <li v-if="recovery.lastBackup">
            💾 最近备份：{{ fmtDate(recovery.lastBackup.createdAt) }}
          </li>
          <li v-else>⚠️ 尚无任何备份</li>
        </ul>
        <p
          v-if="!recovery.dbOk || recovery.attachmentMissingCount > 0"
          style="margin-top: 12px; color: var(--danger); font-size: 13px"
        >
          建议从最近的备份恢复数据。
        </p>
      </div>
      <div class="dialog-ops">
        <button v-if="recovery.lastBackup" class="btn" @click="gotoRestore">前往恢复</button>
        <button class="btn btn-primary" @click="dismissRecovery">继续使用</button>
      </div>
    </div>
  </div>
</template>
