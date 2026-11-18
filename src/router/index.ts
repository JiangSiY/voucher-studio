import { createRouter, createWebHashHistory } from "vue-router";
import WorkbenchLayout from "../layouts/WorkbenchLayout.vue";
import { useSessionStore } from "../stores/session";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    // 网页版登录页：不进工作台布局（没有侧栏/工具栏，服务端还没认人）
    {
      path: "/login",
      name: "login",
      component: () => import("../pages/LoginPage.vue"),
      meta: { title: "登录" },
    },
    {
      path: "/",
      component: WorkbenchLayout,
      children: [
        { path: "", redirect: "/inbox" },
        // v0.18：收件箱成为首页（流程为中心）；旧 /dashboard 链接重定向保留
        {
          path: "inbox",
          name: "inbox",
          component: () => import("../pages/InboxPage.vue"),
          meta: { title: "待处理", subtitle: "今天要处理的事" },
        },
        { path: "dashboard", redirect: "/inbox" },
        // v0.18：资料库 = 双行列表 + 检视器（取代旧 10 列表格）；旧 /vouchers 仍指到这里
        {
          path: "vouchers",
          name: "vouchers",
          component: () => import("../pages/LibraryPage.vue"),
          meta: { title: "全部凭证", subtitle: "双行列表 · 右栏即编辑器" },
        },
        // v0.16 疑似重复核查（静态段须在 vouchers/:id 之前，避免被动态段吃掉）
        {
          path: "duplicates",
          name: "duplicates",
          component: () => import("../pages/DuplicateReviewPage.vue"),
          meta: { title: "重复核查", subtitle: "同图或同字段的证据比对" },
        },
        // v13（2026-11-14）：台账页 = 清单导入 → 补附件 → 等入账的一条流水线
        {
          path: "ledger",
          name: "ledger",
          component: () => import("../pages/LedgerPage.vue"),
          meta: { title: "台账", subtitle: "月度清单导入 · 补附件 · 等入账" },
        },
        // v0.26：会计投影 = 台账凭证 → 借贷分录 → KIS 引入 / 抄单纸 / V5 分录
        {
          path: "accounting",
          name: "accounting",
          component: () => import("../pages/AccountingPage.vue"),
          meta: { title: "会计投影", subtitle: "预演分录 · 导出 KIS · 回写台账" },
        },
        {
          path: "vouchers/new",
          name: "voucher-new",
          component: () => import("../pages/VoucherFormPage.vue"),
          meta: { title: "新建凭证", subtitle: "多步录入 · 自动存档", reading: true },
        },
        // v0.19（2026-09-28）：编辑页已并入资料库右栏（components/voucher/VoucherEditor.vue）。
        // 旧链接/书签 `/vouchers/:id` 保留为**重定向**：跳到资料库并聚焦该张（不 404）。
        // 注意路径顺序：静态段 `vouchers/new` 必须排在本条之前，否则会被动态段吃掉。
        {
          path: "vouchers/:id",
          redirect: (to) => ({ path: "/vouchers", query: { focus: String(to.params.id) } }),
        },
        {
          path: "search",
          name: "search",
          component: () => import("../pages/SearchPage.vue"),
          meta: { title: "搜索", subtitle: "全库检索" },
        },
        {
          path: "print",
          name: "print",
          component: () => import("../pages/PrintCenterPage.vue"),
          meta: { title: "打印与导出", subtitle: "PDF 日报与表格" },
        },
        {
          path: "insights",
          name: "insights",
          component: () => import("../pages/InsightsPage.vue"),
          meta: { title: "洞察", subtitle: "趋势 · 构成 · 覆盖度" },
        },
        {
          path: "exports",
          name: "exports",
          component: () => import("../pages/ExportHistoryPage.vue"),
          meta: { title: "导出历史", subtitle: "历史导出批次" },
        },
        {
          path: "settings",
          name: "settings",
          component: () => import("../pages/SettingsPage.vue"),
          meta: { title: "设置", subtitle: "外观 · 识别 · 备份", reading: true },
        },
        {
          path: "restore",
          name: "restore",
          component: () => import("../pages/RestorePage.vue"),
          meta: { title: "备份与恢复", subtitle: "备份、恢复与迁移" },
        },
        {
          path: "help",
          name: "help",
          component: () => import("../pages/HelpPage.vue"),
          meta: { title: "帮助", subtitle: "使用说明与快捷键", reading: true },
        },
      ],
    },
  ],
});

/**
 * 网页版会话守卫：桌面版（isTauri）永远放行；网页版未登录一律回 /login。
 *
 * `session.refresh()` 只在首次导航时 await 一次（`checking` 初值为 true），
 * 之后每次导航只看内存状态，避免每个路由都打一次 /api/auth/status。
 * 这里 `try` 兜底是为了单测：测试里可能没有挂 Pinia，守卫不应因此崩掉。
 */
router.beforeEach(async (to) => {
  if (to.name === "login") return true;
  let session: ReturnType<typeof useSessionStore>;
  try {
    session = useSessionStore();
  } catch {
    return true;
  }
  if (!session.authRequired) return true;
  if (session.checking) await session.refresh();
  if (session.loggedIn) return true;
  return { name: "login", query: { next: to.fullPath } };
});
