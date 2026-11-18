<script setup lang="ts">
import { computed, ref } from "vue";
import { useRoute } from "vue-router";
import { useBreakpoint } from "../composables/useBreakpoint";
import { useConfigStore } from "../stores/config";
import { useDupStore } from "../stores/dup";
import { useInboxStore } from "../stores/inbox";
import { useLedgerStore } from "../stores/ledger";
import { toastError } from "../stores/toast";
import { useSessionStore } from "../stores/session";
import { isTauri } from "../api/env";
import { useRouter } from "vue-router";
import FluentIcon from "../components/FluentIcon.vue";
import SidebarResizeHandle from "../components/layout/SidebarResizeHandle.vue";

const route = useRoute();
const router = useRouter();
const session = useSessionStore();
const configStore = useConfigStore();
const dupStore = useDupStore();
const inboxStore = useInboxStore();
const ledgerStore = useLedgerStore();

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
/** 角标数字：超过 99 显示 99+（侧栏只有 15px 见方，三位数会糊） */
function badgeText(n: number): string {
  return n > 99 ? "99+" : String(n);
}

const navItems = computed(() => [
  // v0.18 收件箱驱动：首页即"今天要处理什么"，角标显示待处理项数
  {
    to: "/inbox",
    icon: "inbox",
    label: "待处理",
    badge: inboxStore.pendingCount,
    badgeLabel: "项待办",
  },
  { to: "/vouchers", icon: "list", label: "全部凭证", badge: 0, badgeLabel: "" },
  // v13：台账 = 月度清单导入 → 补附件 → 等入账（角标 = 待补附件张数）
  {
    to: "/ledger",
    icon: "file",
    label: "台账",
    badge: ledgerStore.unattached,
    badgeLabel: "张待补附件",
  },
  // v0.26：台账之后就是会计投影——清单补完附件，下一步就是把它们折成分录
  { to: "/accounting", icon: "chart", label: "会计投影", badge: 0, badgeLabel: "" },
  {
    to: "/duplicates",
    icon: "duplicate",
    label: "重复核查",
    badge: dupStore.pending.groups,
    badgeLabel: "组疑似重复",
  },
  { to: "/insights", icon: "chart", label: "洞察", badge: 0, badgeLabel: "" },
  { to: "/print", icon: "print", label: "打印与导出", badge: 0, badgeLabel: "" },
  { to: "/exports", icon: "chart", label: "导出历史", badge: 0, badgeLabel: "" },
  { to: "/settings", icon: "settings", label: "设置", badge: 0, badgeLabel: "" },
  { to: "/restore", icon: "save", label: "备份与恢复", badge: 0, badgeLabel: "" },
  { to: "/help", icon: "help", label: "帮助", badge: 0, badgeLabel: "" },
]);

const pageTitle = computed(() => (route.meta.title as string) ?? "");
/** Loon 式副标题：说明「这一页此刻能做什么」，只在路由声明，不在页面里重复写。 */
const pageSubtitle = computed(() => (route.meta.subtitle as string) ?? "");

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

/** 网页版才有「退出登录」；桌面版是本地单机应用，没有会话概念 */
async function signOut() {
  await session.logout();
  await router.replace({ name: "login" });
}

// ===== 移动壳（<768px）=====
// 三档断点的唯一真源在 composables/useBreakpoint.ts；CSS 侧对应 base.css 末尾的
// `.app-shell--compact` 一节。手机上侧栏整体隐藏，导航改底部 5 项 tab bar，
// 「更多」用底部浮层装工具区入口（避免再加一个真实路由与页面）。
const { isCompact } = useBreakpoint();
const moreOpen = ref(false);

const workItems = computed(() => navItems.value.slice(0, 5));
const toolItems = computed(() => navItems.value.slice(5));

/** 底部 tab：工作区前 4 项 + 「更多」 */
const tabItems = computed(() => [
  ...workItems.value.slice(0, 4).map((item) => ({ ...item, key: item.to })),
  { to: "__more__", key: "__more__", icon: "settings", label: "更多", badge: 0, badgeLabel: "" },
]);

const tabActive = (key: string): boolean => {
  if (key === "__more__") return toolItems.value.some((item) => route.path.startsWith(item.to));
  return route.path.startsWith(key);
};

/** 手机详情页需要返回；工作区首页没有可返回的层级 */
const canGoBack = computed(() => route.path !== "/inbox");

function goBack() {
  if (window.history.length > 1) router.back();
  else void router.push("/inbox");
}

function openTab(key: string) {
  if (key === "__more__") {
    moreOpen.value = true;
    return;
  }
  void router.push(key);
}
</script>

<template>
  <div class="app-shell" :class="{ 'app-shell--compact': isCompact }">
    <aside id="app-sidebar" class="sidebar">
      <!-- 侧栏顶部：**只留一段可拖窗口的留白**（交通灯就在这条上，里面没有可点元素，
           deep 不会误吞点击）。用户 2026-10-16 要求去掉左上角图标（原 VS 徽标 + 字标），
           留白高度仍按 `calc(var(--toolbar-h) - var(--s-2))` 让开红绿灯带（y≈18–30）。 -->
      <div class="sidebar-header" data-tauri-drag-region="deep"></div>
      <!-- 扁平导航（无分组，用户 2026-10-16 要求）；收起时 CSS 只留图标 -->
      <nav class="sidebar-nav" aria-label="主导航">
        <router-link
          v-for="item in navItems"
          :key="item.to"
          :to="item.to"
          :title="item.badge > 0 ? `${item.label}（${item.badge} ${item.badgeLabel}）` : item.label"
          :aria-label="
            item.badge > 0 ? `${item.label}，${item.badge} ${item.badgeLabel}` : undefined
          "
        >
          <!-- 角标压在图标右上角：不再占行尾宽度，侧栏因此可以更窄（用户 2026-10-16 要求） -->
          <span class="nav-icon">
            <FluentIcon :name="item.icon" :size="18" />
            <span
              v-if="item.badge > 0"
              class="nav-badge"
              :title="`${item.badge} ${item.badgeLabel}`"
            >
              {{ badgeText(item.badge) }}
            </span>
          </span>
          <span class="nav-label">{{ item.label }}</span>
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
        <span class="nav-icon"><FluentIcon name="sidebar" :size="18" /></span>
        <span class="nav-label">{{ sidebarCollapsed ? "展开侧边栏" : "收起侧边栏" }}</span>
      </button>
      <button
        v-if="!isTauri"
        class="sidebar-toggle"
        title="退出登录"
        aria-label="退出登录"
        @click="signOut"
      >
        <span class="nav-icon"><FluentIcon name="lock" :size="18" /></span>
        <span class="nav-label">退出登录</span>
      </button>
      <div class="sidebar-footer">
        v{{ version }} · {{ isTauri ? "数据本地存储" : "数据在本机服务器" }}
      </div>
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
        <!-- 工具栏里不再放按钮：命令面板按钮 2026-10-16 按用户要求去掉
             （"基本没用过，没有价值"）；⌘K 仍可用（`App.vue` 全局快捷键 → `stores/ui.ts`）。 -->
        <!-- 手机档（<768px）：左侧给返回键，标题降一级，副标题隐藏（见 base.css `.app-shell--compact`） -->
        <button
          v-if="isCompact && canGoBack"
          class="mobile-back"
          type="button"
          aria-label="返回"
          title="返回"
          @click="goBack"
        >
          <FluentIcon name="arrowLeft" :size="18" />
        </button>
        <div class="toolbar-heading">
          <h1 class="toolbar-title">{{ pageTitle }}</h1>
          <p v-if="pageSubtitle" class="toolbar-subtitle">{{ pageSubtitle }}</p>
        </div>
      </header>
      <div class="main-scroll" @click="moreOpen = false">
        <div class="page" :class="{ 'page--reading': isReading }">
          <router-view />
        </div>
      </div>

      <!-- 底部 5 项 tab bar（仅手机档）：工作区前 4 项 + 更多；点当前项不重复跳转 -->
      <nav v-if="isCompact && !moreOpen" class="mobile-tabbar" aria-label="主导航">
        <button
          v-for="item in tabItems"
          :key="item.key"
          type="button"
          class="mobile-tab"
          :class="{ 'is-active': tabActive(item.key) }"
          :aria-current="tabActive(item.key) ? 'page' : undefined"
          :title="item.badge > 0 ? `${item.label}（${item.badge} ${item.badgeLabel}）` : item.label"
          @click="openTab(item.key)"
        >
          <span class="tab-icon">
            <FluentIcon :name="item.icon" :size="18" />
            <span v-if="item.badge > 0" class="tab-badge">{{ badgeText(item.badge) }}</span>
          </span>
          <span class="tab-label">{{ item.label }}</span>
        </button>
      </nav>

      <!-- 「更多」浮层：工具区 6 项 + 退出登录（网页版才有会话） -->
      <div v-if="moreOpen && isCompact" class="mobile-more-scrim" @click.self="moreOpen = false">
        <div class="mobile-more-sheet" role="dialog" aria-label="更多">
          <div class="mobile-more-grip" aria-hidden="true"></div>
          <router-link
            v-for="item in toolItems"
            :key="item.to"
            :to="item.to"
            class="mobile-more-item"
            @click="moreOpen = false"
          >
            <FluentIcon :name="item.icon" :size="18" />
            <span>{{ item.label }}</span>
          </router-link>
          <button v-if="!isTauri" type="button" class="mobile-more-item" @click="signOut">
            <FluentIcon name="lock" :size="18" />
            <span>退出登录</span>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
