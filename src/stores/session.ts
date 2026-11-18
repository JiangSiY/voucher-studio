/**
 * 登录会话（网页版专有；桌面版 `loggedIn` 恒为 true，不显示登录页）。
 *
 * 服务端是单用户 + Cookie 会话（见 crates/center-server/src/auth.rs）：
 * - 首次打开：`needsSetup=true`，引导设置访问密码；
 * - 之后：输入密码登录，会话 30 天，服务端可随时清除；
 * - 任何命令返回 401 时由 client.ts 广播 `center:unauthorized`，本 store 收到后
 *   把 `loggedIn` 置 false，路由守卫会把用户送回登录页。
 */
import { acceptHMRUpdate, defineStore } from "pinia";
import { computed, ref } from "vue";
import { isTauri } from "../api/env";

interface AuthStatus {
  needsSetup: boolean;
  loggedIn: boolean;
}

export const useSessionStore = defineStore("session", () => {
  const needsSetup = ref(false);
  const loggedIn = ref(isTauri);
  const checking = ref(!isTauri);
  const error = ref("");

  /** 网页版才需要登录；桌面版永远放行 */
  const authRequired = computed(() => !isTauri);
  const ready = computed(() => !checking.value);

  async function request(path: string, body?: unknown): Promise<void> {
    const resp = await fetch(path, {
      method: body === undefined ? "GET" : "POST",
      headers: body === undefined ? undefined : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      credentials: "same-origin",
    });
    if (!resp.ok) {
      const text = await resp.text();
      let message = text || `HTTP ${resp.status}`;
      try {
        const parsed = JSON.parse(text) as { error?: { message?: string } };
        if (parsed?.error?.message) message = parsed.error.message;
      } catch {
        /* 非 JSON 错误体：保留原文 */
      }
      throw new Error(message);
    }
  }

  async function refresh(): Promise<void> {
    if (!authRequired.value) {
      loggedIn.value = true;
      checking.value = false;
      return;
    }
    try {
      const resp = await fetch("/api/auth/status", { credentials: "same-origin" });
      const data = (await resp.json()) as AuthStatus;
      needsSetup.value = !!data.needsSetup;
      loggedIn.value = !!data.loggedIn;
    } catch {
      loggedIn.value = false;
    } finally {
      checking.value = false;
    }
  }

  async function setup(password: string): Promise<void> {
    error.value = "";
    await request("/api/auth/setup", { password });
    needsSetup.value = false;
    loggedIn.value = true;
  }

  async function login(password: string): Promise<void> {
    error.value = "";
    await request("/api/auth/login", { password });
    loggedIn.value = true;
  }

  async function logout(): Promise<void> {
    try {
      await request("/api/auth/logout", {});
    } catch {
      /* 退出失败也要把本地状态清掉，避免卡在灰屏 */
    }
    loggedIn.value = false;
  }

  function markExpired(): void {
    if (authRequired.value) loggedIn.value = false;
  }

  // 会话过期广播来自 api/client.ts 的 401 分支；store 是单例，但 dev HMR 会重跑 setup，
  // 旧闭包若不摘掉会继续写上一轮 store 的 ref（与 config.ts 的 matchMedia 同因）。
  if (typeof window !== "undefined") {
    window.addEventListener("center:unauthorized", markExpired);
    if (import.meta.hot) {
      import.meta.hot.dispose(() => window.removeEventListener("center:unauthorized", markExpired));
    }
  }

  return {
    needsSetup,
    loggedIn,
    checking,
    error,
    authRequired,
    ready,
    refresh,
    setup,
    login,
    logout,
  };
});

if (import.meta.hot) import.meta.hot.accept(acceptHMRUpdate(useSessionStore, import.meta.hot));
