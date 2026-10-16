<script setup lang="ts">
import { computed } from "vue";
import { useRoute } from "vue-router";
import { useConfigStore } from "../stores/config";
import { useDupStore } from "../stores/dup";
import { useInboxStore } from "../stores/inbox";
import { useUiStore } from "../stores/ui";
import { toastError } from "../stores/toast";
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

/**
 * 侧栏导航：**扁平一列，不再分组**（用户 2026-10-16 要求「去掉侧边栏分组功能」）。
 *
 * 分组标题（收件箱 / 资料库 / 设置）与可折叠组是 v0.18 引入的；用户明确不要分组后，
 * 分区语义改由**条目顺序与文案**表达，导航本身只保留条目。
 * 下方 `sidebarCollapsed` 才是 v0.18 起"侧栏开合"需求的正解：收起整条侧栏，而不是收起某一组。
 */
const navItems = computed(() => [
  // v0.18 收件箱驱动：首页即"今天要处理什么"，badge 显示待处理项数
  { to: "/inbox", icon: "inbox", label: "待处理", badge: inboxStore.pendingCount },
  { to: "/vouchers", icon: "list", label: "全部凭证", badge: 0 },
  { to: "/duplicates", icon: "duplicate", label: "重复核查", badge: dupStore.pending.groups },
  { to: "/insights", icon: "chart", label: "洞察", badge: 0 },
  { to: "/print", icon: "print", label: "打印与导出", badge: 0 },
  { to: "/exports", icon: "chart", label: "导出历史", badge: 0 },
  { to: "/settings", icon: "settings", label: "设置", badge: 0 },
  { to: "/restore", icon: "save", label: "备份与恢复", badge: 0 },
  { to: "/help", icon: "help", label: "帮助", badge: 0 },
]);

const pageTitle = computed(() => (route.meta.title as string) ?? "");

/**
 * 侧栏收起（用户 2026-10-16 要求：可收起，收起后只显示图标）。
 *
 * 状态与落盘都在 config store（`appearance.sidebarCollapsed`，跨启动保持）；
 * CSS 侧由 `data-sidebar-collapsed` 根属性驱动（见 composables/sidebarCollapse.ts）。
 * 收起后条目只剩图标：文字由 `.nav-label` 隐藏，可发现性靠 `title` 原生提示兜底。
 */
const sidebarCollapsed = computed(() => configStore.sidebarCollapsed);

function toggleSidebar() {
  void configStore.toggleSidebarCollapsed().catch((e) => toastError(e));
}

/** 工具栏右侧动作：打开命令面板（与 ⌘K 同一入口） */
function openPalette() {
  ui.openPalette();
}
</script>

<template>
  <div class="app-shell">
    <aside id="app-sidebar" class="sidebar">
      <!-- 侧栏顶部同样可拖（交通灯就在这条上；里面没有可点元素，deep 不会误吞点击） -->
      <div class="sidebar-header" data-tauri-drag-region="deep">
        <div class="sidebar-logo">
          <span class="badge">VS</span>
          <span>
            Voucher Studio
            <br />
            <small style="font-weight: 400; font-size: 11px; color: var(--text-3)">凭证整理打印</small>
          </span>
        </div>
      </div>
      <!-- 扁平导航（无分组，用户 2026-10-16 要求）；收起时 CSS 只留图标 -->
      <nav class="sidebar-nav" aria-label="主导航">
        <router-link
          v-for="item in navItems"
          :key="item.to"
          :to="item.to"
          :title="item.badge > 0 ? `${item.label}（${item.badge} 组待核查）` : item.label"
        >
          <span class="nav-icon"><FluentIcon :name="item.icon" :size="17" /></span>
          <span class="nav-label">{{ item.label }}</span>
          <span v-if="item.badge > 0" class="nav-badge" :title="`${item.badge} 组疑似重复待核查`">{{ item.badge }}</span>
        </router-link>
      </nav>
      <!-- 收起开关（用户 2026-10-16 第二次定位：从工具栏最左移到**版本行上方那一行**——
           原来那个位置被用户评为"反人类"）。
           整行按钮：面板图标 + 文案，点一下收起 / 再点展开；收起后这一行**仍在**
           （CSS 只留图标，否则没有按钮能展开回来）。 -->
      <button
        class="sidebar-toggle"
        :title="sidebarCollapsed ? '展开侧边栏' : '收起侧边栏'"
        :aria-label="sidebarCollapsed ? '展开侧边栏' : '收起侧边栏'"
        :aria-expanded="!sidebarCollapsed"
        aria-controls="app-sidebar"
        @click="toggleSidebar"
      >
        <span class="nav-icon"><FluentIcon name="sidebar" :size="17" /></span>
        <span class="nav-label">{{ sidebarCollapsed ? "展开侧边栏" : "收起侧边栏" }}</span>
      </button>
      <div class="sidebar-footer">v{{ version }} · 数据本地存储</div>
    </aside>

    <!-- 侧边栏宽度拖拽手柄：挂在 shell 上（侧栏自身滚动，放里面会被滚走） -->
    <SidebarResizeHandle />

    <div class="main-area">
      <!-- 统一工具栏（自绘）：整条可拖动窗口、双击最大化；左侧 84px 给交通灯留白
           （trafficLightPosition 14,18）。

           ⚠️ 必须是 `data-tauri-drag-region="deep"`：Tauri 的注入脚本（tauri/src/window/scripts/drag.js）
           对**无值/`"true"`** 的写法只认"直接点在这个元素本身"（`el === composedPath[0]`），
           而本行中间是 `<h1>` 标题（flex:1，占绝大部分宽度）→ 点标题就落在子元素上、拖不动。
           `"deep"` 表示**子树任意位置**都可拖；其中的按钮/输入框等可点元素（button/input/select/a、
           或带 role=button/tabindex）会**自动阻止拖拽**，所以工具按钮照常点击。

           拖拽还依赖 `core:window:allow-start-dragging` 权限——它不在 core:default 里，
           见 src-tauri/capabilities/default.json。 -->
      <header class="unified-toolbar" data-tauri-drag-region="deep">
        <!-- 侧栏开关不在这里：按用户 2026-10-16 的要求，它已移到侧栏底部版本行上方那一行
             （整行按钮，见 `<aside>` 内 `.sidebar-toggle`）。 -->
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
