<script setup lang="ts">
/**
 * 应用壳层（v0.27）：顶部页签 + 单一内容滚动区 + 弹层宿主。
 *
 * 与旧 WorkbenchLayout 的差别：
 * - 没有侧栏、没有每页标题栏、没有移动底部 tab；
 * - 页签是固定四项，靠左；动作在右（搜索/备份/设置/头像）；
 * - 弹层由 URL query 驱动，刷新/后退都能恢复。
 */
import { onMounted } from "vue";
import TopBar from "../components/shell/TopBar.vue";
import ModalHost from "../components/shell/ModalHost.vue";
import { useInboxStore } from "../stores/inbox";

const inbox = useInboxStore();

onMounted(() => {
  // 网页版登录后不会走 App.vue 的桌面初始化分支；这里补一次待办，
  // 备份按钮的状态点与工作台待办才有值。桌面版 App.vue 已拉过，重复调用无害。
  if (!inbox.todos) void inbox.refresh();
});
</script>

<template>
  <div class="app-shell">
    <a class="skip-link" href="#main-content">跳到主要内容</a>
    <TopBar />
    <main id="main-content" class="content" tabindex="-1">
      <router-view />
    </main>
    <ModalHost />
  </div>
</template>

<style scoped>
.skip-link {
  position: absolute;
  left: -9999px;
  top: 8px;
  z-index: 200;
  padding: 8px 12px;
  border-radius: var(--radius-control);
  background: var(--bg-popover);
  color: var(--text);
  font-size: var(--fs-caption);
}

.skip-link:focus {
  left: 12px;
}
</style>
