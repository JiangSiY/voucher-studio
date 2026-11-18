<script setup lang="ts">
/**
 * 登录 / 首次设置密码（仅网页版；桌面版不会走到这里）。
 *
 * 单用户系统（见 crates/center-server/src/auth.rs）：一个访问密码 + 30 天会话。
 * 首次打开时服务端 `needsSetup=true`，这一页顺便当「初始化向导」，
 * 免得用户第一次打开看到一个 404 或空白页。
 */
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useSessionStore } from "../stores/session";
import { errMsg } from "../api/client";
import FluentIcon from "../components/FluentIcon.vue";

const route = useRoute();
const router = useRouter();
const session = useSessionStore();

const password = ref("");
const confirm = ref("");
const busy = ref(false);
const error = ref("");

const isSetup = computed(() => session.needsSetup);
const title = computed(() => (isSetup.value ? "设置访问密码" : "登录 Center"));
const hint = computed(() =>
  isSetup.value
    ? "这是你自己的账务中枢，先设一个至少 8 位的密码。密码只保存在这台服务器上，不经过任何第三方。"
    : "输入访问密码。登录后 30 天内不用重复输入，可在右上角头像菜单「退出登录」随时结束会话。",
);
const canSubmit = computed(
  () =>
    password.value.length >= 8 &&
    (!isSetup.value || password.value === confirm.value) &&
    !busy.value,
);

/** 登录后回原页面（守卫带来的 `?next=`），没有就回工作台 */
function landing(): string {
  const next = typeof route.query.next === "string" ? route.query.next : "";
  return next.startsWith("/") && !next.startsWith("/login") ? next : "/inbox";
}

onMounted(async () => {
  await session.refresh();
  // 已登录还来 /login：直接回去，避免「登录成功后又被守卫送回登录页」的死循环错觉
  if (session.loggedIn) void router.replace(landing());
});

async function submit() {
  if (!canSubmit.value) return;
  busy.value = true;
  error.value = "";
  try {
    if (isSetup.value) await session.setup(password.value);
    else await session.login(password.value);
    await router.replace(landing());
  } catch (e) {
    error.value = errMsg(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="login-shell">
    <form class="login-card" @submit.prevent="submit">
      <div class="brand">
        <FluentIcon name="lock" :size="24" />
        <span>Center</span>
      </div>
      <h1>{{ title }}</h1>
      <p class="hint">{{ hint }}</p>

      <label class="field">
        <span>访问密码</span>
        <input
          v-model="password"
          type="password"
          autocomplete="current-password"
          placeholder="至少 8 位"
          autofocus
        />
      </label>

      <label v-if="isSetup" class="field">
        <span>再输一次</span>
        <input
          v-model="confirm"
          type="password"
          autocomplete="new-password"
          placeholder="确认密码"
        />
      </label>

      <p v-if="isSetup && confirm && password !== confirm" class="warn">两次输入不一致</p>
      <p v-if="error" class="error">{{ error }}</p>

      <button type="submit" class="submit" :disabled="!canSubmit">
        <FluentIcon :name="busy ? 'refresh' : 'check'" :size="16" />
        {{ busy ? "处理中…" : isSetup ? "设置并进入" : "登录" }}
      </button>

      <p class="foot">数据留在这台机器上 · 经 Tailscale 私网访问</p>
    </form>
  </div>
</template>

<style scoped>
/* 登录页是服务端的「门」：不进工作台布局，所以这里自带一屏背景与卡片。
   样式全部走设计令牌（字号 / 圆角 / 控件高度 / 颜色），否则 tokenContract 与
   designScalesContract 两道契约测试会直接变红。 */
.login-shell {
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: 24px;
  background: var(--canvas-gradient);
}

.login-card {
  width: min(380px, 100%);
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 24px;
  border-radius: var(--radius-card);
  background: var(--bg-card-strong);
  border: 1px solid var(--card-hairline);
  box-shadow: var(--shadow-card);
}

.brand {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-caption);
  font-weight: 600;
  color: var(--label-secondary);
}

h1 {
  margin: 4px 0 0;
  font-size: var(--fs-display);
  font-weight: 650;
  color: var(--label-primary);
}

.hint,
.foot {
  margin: 0;
  font-size: var(--fs-caption);
  line-height: 1.6;
  color: var(--label-secondary);
}

.foot {
  text-align: center;
  color: var(--label-tertiary);
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: var(--fs-caption);
  color: var(--label-secondary);
}

.field input {
  height: var(--control-h-lg);
  padding: 0 12px;
  border-radius: var(--radius-control);
  border: 1px solid var(--separator-strong);
  background: var(--bg-field);
  color: var(--label-primary);
  font-size: var(--fs-body);
}

.field input:focus {
  outline: 2px solid var(--accent-soft-2);
  border-color: var(--accent);
}

.warn {
  margin: 0;
  font-size: var(--fs-caption);
  color: var(--warning);
}

.error {
  margin: 0;
  font-size: var(--fs-caption);
  color: var(--danger);
}

.submit {
  margin-top: 6px;
  height: var(--control-h-lg);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  border: none;
  border-radius: var(--radius-control);
  background: var(--accent);
  color: var(--label-on-accent);
  font-size: var(--fs-body);
  font-weight: 600;
  cursor: pointer;
}

.submit:disabled {
  opacity: 0.55;
  cursor: default;
}
</style>
