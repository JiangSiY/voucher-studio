<script setup lang="ts">
import { computed, ref } from "vue";
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
      { to: "/insights", icon: "chart", label: "洞察", badge: 0 },
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

/**
 * 侧栏分组折叠（用户 2026-09-27 要求：**可折叠 + 默认折叠**）。
 *
 * 规则：
 * - 默认全部收起（`manual` 无记录 = 收起）；
 * - **当前页面所在组自动展开**——否则收起后看不到"我在哪一页"，高亮也失去意义；
 * - 用户手动点过以手动状态为准（切到别的组时，那一组仍按规则自动展开）。
 *
 * 注意：窄窗（≤760px）的 56px 图标栏里**分组标题本就隐藏**，故那里必须忽略折叠状态、
 * 始终显示图标——由 base.css 的媒体查询用类名覆盖实现（`v-show` 的内联样式覆盖不掉）。
 */
const manualCollapsed = ref<Record<string, boolean>>({});

function groupHasRoute(items: { to: string }[]): boolean {
  const path = route.path;
  return items.some((it) => path === it.to || path.startsWith(it.to + "/"));
}

function isOpen(group: string, items: { to: string }[]): boolean {
  const manual = manualCollapsed.value[group];
  if (manual !== undefined) return !manual;
  return groupHasRoute(items);
}

function toggleGroup(group: string, items: { to: string }[]) {
  manualCollapsed.value = { ...manualCollapsed.value, [group]: isOpen(group, items) };
}

/** 收起时把组内徽标合计显示在组标题上（不展开也知道有事要办） */
function groupBadge(items: { badge: number }[]): number {
  return items.reduce((n, it) => n + (it.badge || 0), 0);
}

/** 工具栏右侧动作：打开命令面板（与 ⌘K 同一入口） */
function openPalette() {
  ui.openPalette();
}
</script>

<template>
  <div class="app-shell">
    <aside class="sidebar">
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
      <nav class="sidebar-nav">
        <template v-for="g in navGroups" :key="g.group">
          <button
            class="nav-group"
            :class="{ collapsed: !isOpen(g.group, g.items) }"
            :aria-expanded="isOpen(g.group, g.items)"
            :title="isOpen(g.group, g.items) ? `收起「${g.group}」` : `展开「${g.group}」`"
            @click="toggleGroup(g.group, g.items)"
          >
            <FluentIcon name="chevronRight" :size="12" />
            <span>{{ g.group }}</span>
            <span v-if="groupBadge(g.items) > 0 && !isOpen(g.group, g.items)" class="nav-group-badge">
              {{ groupBadge(g.items) }}
            </span>
          </button>
          <router-link
            v-for="item in g.items"
            :key="item.to"
            :to="item.to"
            :class="{ 'nav-item-collapsed': !isOpen(g.group, g.items) }"
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
           （trafficLightPosition 14,18）。

           ⚠️ 必须是 `data-tauri-drag-region="deep"`：Tauri 的注入脚本（tauri/src/window/scripts/drag.js）
           对**无值/`"true"`** 的写法只认"直接点在这个元素本身"（`el === composedPath[0]`），
           而本行中间是 `<h1>` 标题（flex:1，占绝大部分宽度）→ 点标题就落在子元素上、拖不动。
           `"deep"` 表示**子树任意位置**都可拖；其中的按钮/输入框等可点元素（button/input/select/a、
           或带 role=button/tabindex）会**自动阻止拖拽**，所以工具按钮照常点击。

           拖拽还依赖 `core:window:allow-start-dragging` 权限——它不在 core:default 里，
           见 src-tauri/capabilities/default.json。 -->
      <header class="unified-toolbar" data-tauri-drag-region="deep">
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
