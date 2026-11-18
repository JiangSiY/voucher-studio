import { createRouter, createWebHashHistory } from "vue-router";
import AppShell from "../layouts/AppShell.vue";
import { useSessionStore } from "../stores/session";

/**
 * v0.27 工作台重构后的路由表：
 * - 四个固定页签：/workbench、/data、/reconcile、/accounting；
 * - 辅助功能全部改 URL 驱动的弹层（?modal=settings|backup|export|duplicates|help）；
 * - 旧路由全部做重定向，书签/深链不断。
 */
export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    {
      path: "/login",
      name: "login",
      component: () => import("../pages/LoginPage.vue"),
      meta: { title: "登录" },
    },
    {
      path: "/",
      component: AppShell,
      children: [
        { path: "", redirect: "/workbench" },
        {
          path: "workbench",
          name: "workbench",
          component: () => import("../pages/WorkbenchPage.vue"),
          meta: { title: "工作台" },
        },
        {
          path: "data",
          name: "data",
          component: () => import("../pages/DataPage.vue"),
          meta: { title: "数据" },
        },
        {
          path: "reconcile",
          name: "reconcile",
          component: () => import("../pages/ReconcilePage.vue"),
          meta: { title: "对账" },
        },
        {
          path: "accounting",
          name: "accounting",
          component: () => import("../pages/AccountingPage.vue"),
          meta: { title: "会计投影" },
        },

        // ===== 旧路由 → 新位置（2026-11-18 工作台重构） =====
        { path: "inbox", redirect: "/workbench" },
        { path: "dashboard", redirect: "/workbench" },
        { path: "insights", redirect: "/workbench" },
        { path: "search", redirect: "/workbench" },
        { path: "vouchers", redirect: "/data" },
        { path: "vouchers/new", redirect: "/data?new=1" },
        {
          path: "vouchers/:id",
          redirect: (to) => ({ path: "/data", query: { focus: String(to.params.id) } }),
        },
        { path: "ledger", redirect: "/reconcile" },
        {
          path: "duplicates",
          redirect: (to) => ({
            path: "/workbench",
            query: { ...to.query, modal: "duplicates" },
          }),
        },
        { path: "print", redirect: "/data?modal=export" },
        { path: "exports", redirect: "/data?modal=export&tab=history" },
        { path: "settings", redirect: "/workbench?modal=settings" },
        { path: "restore", redirect: "/workbench?modal=backup" },
        { path: "help", redirect: "/workbench?modal=help" },

        { path: ":pathMatch(.*)*", redirect: "/workbench" },
      ],
    },
  ],
});

/**
 * 网页版会话守卫：桌面版（isTauri）永远放行；网页版未登录一律回 /login。
 * `session.refresh()` 只在首次导航时 await 一次，之后只看内存状态。
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
