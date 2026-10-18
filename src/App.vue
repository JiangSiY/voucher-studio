<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { safeListen } from "./composables/safeListen";
import { useRouter } from "vue-router";
import { api, errMsg, isTauri, type BackupInfo, type ImportSummary, type RecoveryReport } from "./api/client";
import { fmtDate } from "./composables/format";
import { useConfigStore } from "./stores/config";
import { useDupStore } from "./stores/dup";
import { useInboxStore } from "./stores/inbox";
import { useUiStore } from "./stores/ui";
import { toast } from "./stores/toast";
import ToastHost from "./components/ToastHost.vue";
import FluentIcon from "./components/FluentIcon.vue";
import CommandPalette from "./components/CommandPalette.vue";
import TaskCenter from "./components/TaskCenter.vue";
import { useTaskStore } from "./stores/tasks";

const configStore = useConfigStore();
const dupStore = useDupStore();
const inboxStore = useInboxStore();
const router = useRouter();

// v0.13：全局命令面板（Ctrl+K）
const ui = useUiStore();
const paletteVisible = computed(() => ui.paletteVisible);

// 全局任务中心（规范 §1.5）：导入 / 备份 / 导出 / 批量识别这类长任务的进度都汇到这里，
// 换页也看得见；toast 只负责"一次性结果"，两者分工见 stores/tasks.ts 头注释。
const taskStore = useTaskStore();
/** 当前进行中的导入/备份任务 id（一次只可能有一个：Rust 侧同一时刻只跑一条导入/一条关闭备份） */
let importTaskId: number | null = null;
let closingBackupTaskId: number | null = null;

function onGlobalKeydown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "k") {
    e.preventDefault();
    ui.togglePalette();
  }
}

onMounted(() => window.addEventListener("keydown", onGlobalKeydown));
onUnmounted(() => window.removeEventListener("keydown", onGlobalKeydown));

// 全局事件告警（任何页面都可见）
let unlistenDone: UnlistenFn | null = null;
let unlistenFailed: UnlistenFn | null = null;
let unlistenClosing: UnlistenFn | null = null;
// 拖放导入（Rust 侧处理）：遮罩与结果反馈
let unlistenDragEnter: UnlistenFn | null = null;
let unlistenDragLeave: UnlistenFn | null = null;
let unlistenImportStarted: UnlistenFn | null = null;
let unlistenImportDone: UnlistenFn | null = null;
let unlistenImportError: UnlistenFn | null = null;
// v0.16：疑似重复待处理数（侧栏徽标 / 仪表盘面板）
let unlistenDupChanged: UnlistenFn | null = null;
const draggingOver = ref(false);
const importing = ref(false);
// 关闭前备份进行中（遮罩 + 跳过按钮）
const closingBackup = ref(false);

// 崩溃恢复：上次会话异常退出时的健康报告对话框
const recovery = ref<RecoveryReport | null>(null);


onMounted(async () => {
  // 纯浏览器打开 dev URL：跳过所有 Tauri 相关初始化，避免裸 JS 错误刷屏
  if (!isTauri) {
    toast.info("浏览器预览模式：仅界面可浏览，请通过桌面应用使用完整功能");
    return;
  }
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
  unlistenDone = await safeListen<BackupInfo>("backup:done", (e) => {
    if (closingBackupTaskId !== null) {
      taskStore.finish(closingBackupTaskId, {
        detail: e.payload.warnings?.length ? "副本存在问题，见提示" : "本地备份已就绪",
      });
      closingBackupTaskId = null;
    }
    if (e.payload.warnings?.length) {
      toast.warning(`自动备份完成，但副本存在问题：${e.payload.warnings.join("；")}`);
    }
  });
  unlistenFailed = await safeListen<string>("backup:failed", (e) => {
    if (closingBackupTaskId !== null) {
      taskStore.fail(closingBackupTaskId, e.payload);
      closingBackupTaskId = null;
    }
    toast.danger(`自动备份失败：${e.payload}`);
  });

  // 关闭前自动备份：Rust 拦截关闭请求 → 遮罩提示 + 可跳过
  unlistenClosing = await safeListen("backup:closing", () => {
    closingBackup.value = true;
    closingBackupTaskId = taskStore.start("backup", "关闭前自动备份");
  });

  // 拖放导入（Rust 侧处理）：进入/离开遮罩 → 开始提示 → 结果反馈
  unlistenDragEnter = await safeListen("import:drag-enter", () => {
    draggingOver.value = true;
  });
  unlistenDragLeave = await safeListen("import:drag-leave", () => {
    draggingOver.value = false;
  });
  unlistenImportStarted = await safeListen<number>("import:started", (e) => {
    importing.value = true;
    draggingOver.value = false;
    // 进度由 Rust 侧按批事件驱动，任务中心先起一条不确定态；导入期间换页也不丢
    importTaskId = taskStore.start("import", `导入 ${e.payload} 张图片`, e.payload);
    toast.info?.(`正在导入 ${e.payload} 张图片…`);
  });
  unlistenImportDone = await safeListen<ImportSummary>("import:done", (e) => {
    importing.value = false;
    void inboxStore.refresh(); // 导入即产生"待识别/待核对"，侧栏待办徽标随之刷新
    const s = e.payload;
    const parts: string[] = [];
    if (s.vouchers.length) parts.push(`已导入 ${s.vouchers.length} 张图片，各建一张凭证`);
    if (s.skippedDuplicates.length) parts.push(`跳过重复 ${s.skippedDuplicates.length} 张`);
    if (s.failed.length) parts.push(`失败 ${s.failed.length} 张`);
    const goReview = () => {
      if (s.vouchers.length)
        router.push({ path: "/vouchers", query: { focus: String(s.vouchers[0].voucherId) } });
    };
    if (importTaskId !== null) {
      taskStore.finish(importTaskId, {
        detail: parts.join(" · ") || "没有新的图片",
        action: s.vouchers.length ? { label: "开始核对", onClick: goReview } : undefined,
      });
      importTaskId = null;
    }
    if (parts.length) {
      // 动线：导入完成即引导核对（跳到第一张新凭证，其余可用「下一张」连续翻）
      toast.success(parts.join(" · "), { label: "开始核对", onClick: goReview });
    }
  });
  unlistenImportError = await safeListen<string>("import:error", (e) => {
    importing.value = false;
    if (importTaskId !== null) {
      taskStore.fail(importTaskId, e.payload);
      importTaskId = null;
    }
    toast.danger(`导入失败：${e.payload}`);
  });


  // v0.16 疑似重复：启动拉一次待处理数（侧栏徽标 / 仪表盘面板共用），
  // 任意页面处置后由后端 dup:changed 驱动刷新（含仪表盘的面板与徽标）
  // 侧栏两个徽标：疑似重复（dupStore）与待处理项数（inboxStore）——启动各拉一次，
  // 之后由事件驱动刷新（导入/备份/处置都会改变待办构成）
  void dupStore.refresh();
  void inboxStore.refresh();
  unlistenDupChanged = await safeListen("dup:changed", () => {
    void dupStore.refresh();
    void inboxStore.refresh();
  });
});

function dismissRecovery() {
  recovery.value = null;
}

/** 跳过关闭备份直接退出（数据无损；本次备份不完整） */
async function skipClosingBackup() {
  try {
    await api.skipClosingBackup();
  } catch (e) {
    toast.danger(`退出失败：${errMsg(e)}`);
  }
}

function gotoRestore() {
  recovery.value = null;
  router.push("/restore");
}

onUnmounted(() => {
  unlistenDone?.();
  unlistenFailed?.();
  unlistenClosing?.();
  unlistenDragEnter?.();
  unlistenDragLeave?.();
  unlistenImportStarted?.();
  unlistenImportDone?.();
  unlistenImportError?.();
  unlistenDupChanged?.();
});
</script>

<template>
  <router-view />

  <!-- v0.13：全局命令面板（Ctrl+K）：页面跳转 / 快速动作 / 凭证搜索 -->
  <CommandPalette :visible="paletteVisible" @close="ui.closePalette()" />

  <!-- 拖放导入遮罩（Rust 侧 drag-enter/leave 事件驱动） -->
  <div v-if="draggingOver" class="drop-overlay">松手导入图片（一图一凭证：每张图片各建一张凭证）</div>
  <div v-else-if="importing" class="drop-overlay" style="pointer-events: none">正在导入图片，请稍候…</div>

  <!-- 关闭前自动备份遮罩（Rust 拦截关闭请求后触发；完成后窗口自动销毁，此遮罩随之消失） -->
  <div v-else-if="closingBackup" class="closing-backup-overlay">
    <div class="closing-backup-card">
      <div class="closing-backup-title">正在执行关闭前备份…</div>
      <div class="closing-backup-hint">本地快速备份（跳过云端），完成后自动退出</div>
      <button class="btn" @click="skipClosingBackup">跳过备份，直接退出</button>
    </div>
  </div>

  <TaskCenter />

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
        <ul style="list-style: none; display: flex; flex-direction: column; gap: 8px; font-size: var(--fs-caption)">
          <li style="display: flex; align-items: flex-start; gap: 6px">
            <template v-if="recovery.dbOk"><span style="color: var(--success); flex-shrink: 0"><FluentIcon name="checkCircle" :size="16" /></span><span>数据库完整性检查通过</span></template>
            <template v-else>
              <span style="color: var(--danger); flex-shrink: 0"><FluentIcon name="warning" :size="16" /></span>
              <span>数据库完整性检查发现问题：
                <span class="mono" style="color: var(--danger)">{{
                  recovery.integrityErrors.join("；") || "未知错误"
                }}</span>
              </span>
            </template>
          </li>
          <li v-if="recovery.attachmentTotal > 0" style="display: flex; align-items: flex-start; gap: 6px">
            <template v-if="recovery.attachmentMissingCount === 0">
              <span style="color: var(--success); flex-shrink: 0"><FluentIcon name="checkCircle" :size="16" /></span>
              <span>附件文件核对通过（{{ recovery.attachmentTotal }} 张图片全部在位）</span>
            </template>
            <template v-else>
              <span style="color: var(--danger); flex-shrink: 0"><FluentIcon name="warning" :size="16" /></span>
              <span>有 {{ recovery.attachmentMissingCount }} 张附件图片文件缺失（如
                {{ recovery.attachmentMissing[0] }}）
              </span>
            </template>
          </li>
          <li v-else style="display: flex; align-items: center; gap: 6px">
            <span style="color: var(--accent); flex-shrink: 0"><FluentIcon name="info" :size="16" /></span>尚无附件图片
          </li>
          <li v-if="recovery.attachmentMissingEnhanced > 0" style="display: flex; align-items: flex-start; gap: 6px">
            <span style="color: var(--warning); flex-shrink: 0"><FluentIcon name="info" :size="16" /></span>
            <span>有 {{ recovery.attachmentMissingEnhanced }} 张图片的「增强版」文件不在磁盘上——增强版是可重建缓存、
              不随备份保存（恢复备份后必然出现）。界面已自动改用原图显示，可在 设置 → 扫描矫正 一键重建增强。</span>
          </li>
          <li v-if="recovery.lastBackup" style="display: flex; align-items: center; gap: 6px">
            <span style="color: var(--accent); flex-shrink: 0"><FluentIcon name="save" :size="16" /></span>
            最近备份：{{ fmtDate(recovery.lastBackup.createdAt) }}
          </li>
          <li v-else style="display: flex; align-items: center; gap: 6px">
            <span style="color: var(--danger); flex-shrink: 0"><FluentIcon name="warning" :size="16" /></span>尚无任何备份
          </li>
        </ul>
        <p
          v-if="!recovery.dbOk || recovery.attachmentMissingCount > 0"
          style="margin-top: 12px; color: var(--danger); font-size: var(--fs-caption)"
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
