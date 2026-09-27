<script setup lang="ts">
import { computed } from "vue";
import { useRoute } from "vue-router";
import { useConfigStore } from "../stores/config";
import { useDupStore } from "../stores/dup";
import { useInboxStore } from "../stores/inbox";
import { useUiStore } from "../stores/ui";
import FluentIcon from "../components/FluentIcon.vue";
import SidebarResizeHandle from "../components/layout/SidebarResizeHandle.vue";

const route = useRoute();
const configStore = useConfigStore();
const dupStore = useDupStore();
const ui = useUiStore();
const inboxStore = useInboxStore();

// 阅读型页面（表单 / 设置 / 帮助）保留 1100px 上限；宽内容页（列表/仪表盘/打印中心）铺满
const isReading = computed(() => route.meta.reading === true);

const version = computed(() => configStore.appInfo?.version ?? "…");

const navGroups = computed(() => [
  {
    group: "收件箱",
    items: [
      // v0.18 收件箱驱动：首页即"今天要处理什么"，badge 显示待处理项数
      { to: "/inbox", icon: "inbox", label: "待处理", badge: inboxStore.pendingCount },
    ],
  },
  {
    group: "资料库",
    items: [
      { to: "/vouchers", icon: "list", label: "全部凭证", badge: 0 },
      { to: "/duplicates", icon: "duplicate", label: "重复核查", badge: dupStore.pending.groups },
      { to: "/print", icon: "print", label: "打印与导出", badge: 0 },
      { to: "/exports", icon: "chart", label: "导出历史", badge: 0 },
    ],
  },
  {
    group: "设置",
    items: [
      { to: "/settings", icon: "settings", label: "设置", badge: 0 },
      { to: "/restore", icon: "save", label: "备份与恢复", badge: 0 },
      { to: "/help", icon: "help", label: "帮助", badge: 0 },
    ],
  },
]);

const pageTitle = computed(() => (route.meta.title as string) ?? "");

/** 工具栏右侧动作：打开命令面板（与 ⌘K 同一入口） */
function openPalette() {
  ui.openPalette();
}
</script>

<template>
  <div class="app-shell">
    <aside class="sidebar">
      <div class="sidebar-header">
        <div class="sidebar-logo">
          <span class="badge">VS</span>
          <span>
            Voucher Studio
            <br />
            <small style="font-weight: 400; font-size: 11px; color: var(--text-3)">凭证整理打印</small>
          </span>
        </div>
      </div>
      <nav class="sidebar-nav">
        <template v-for="g in navGroups" :key="g.group">
          <div class="nav-group">{{ g.group }}</div>
          <router-link v-for="item in g.items" :key="item.to" :to="item.to"
            :title="item.badge > 0 ? `${item.label}（${item.badge} 组待核查）` : item.label">
            <span class="nav-icon"><FluentIcon :name="item.icon" :size="17" /></span>
            <span class="nav-label">{{ item.label }}</span>
            <span v-if="item.badge > 0" class="nav-badge" :title="`${item.badge} 组疑似重复待核查`">{{ item.badge }}</span>
          </router-link>
        </template>
      </nav>
      <div class="sidebar-footer">v{{ version }} · 数据本地存储</div>
    </aside>

    <!-- 侧边栏宽度拖拽手柄：挂在 shell 上（侧栏自身滚动，放里面会被滚走） -->
    <SidebarResizeHandle />

    <div class="main-area">
      <!-- 统一工具栏（自绘）：整条可拖动窗口、双击最大化；左侧 84px 给交通灯留白
           （trafficLightPosition 14,18）。拖拽依赖 `core:window:allow-start-dragging`
           权限——它不在 core:default 里，见 capabilities/default.json。 -->
      <header class="unified-toolbar" data-tauri-drag-region>
        <h1 class="toolbar-title">{{ pageTitle }}</h1>
        <div class="toolbar-actions">
          <button class="toolbar-btn" title="命令面板（⌘K）" @click="openPalette">
            <FluentIcon name="search" :size="15" />
            <span class="toolbar-hint">⌘K</span>
          </button>
        </div>
      </header>
      <div class="main-scroll">
        <div class="page" :class="{ 'page--reading': isReading }">
          <router-view />
        </div>
      </div>
    </div>
  </div>
</template>
