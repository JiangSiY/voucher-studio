<script setup lang="ts">
/**
 * 设置 → 密钥管理：系统凭据库（Windows 凭据管理器）读写。
 * 草稿与状态跨 Tab 共享（备份 Tab 的 WebDAV「设置密码」读取同一份草稿）。
 */
import { onMounted } from "vue";
import { api } from "../../api/client";
import FluentIcon from "../FluentIcon.vue";
import { toast, toastError } from "../../stores/toast";
import { refreshSecrets, secretDrafts, secrets } from "../../composables/secretDrafts";

async function saveSecret(alias: string) {
  const value = secretDrafts[alias]?.trim();
  if (!value) {
    toast.warning("请输入内容");
    return;
  }
  try {
    await api.setSecret(alias, value);
    secretDrafts[alias] = "";
    await refreshSecrets();
    toast.success("已写入系统凭据库");
  } catch (e) {
    toastError(e);
  }
}

async function removeSecret(alias: string) {
  try {
    await api.deleteSecret(alias);
    await refreshSecrets();
    toast.success("已删除");
  } catch (e) {
    toastError(e);
  }
}

onMounted(async () => {
  try {
    await refreshSecrets();
  } catch (e) {
    toastError(e);
  }
});
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-header"><h3>密钥管理</h3></div>
    <div class="fluent-panel-body">
      <div class="callout info">
        <span class="ci"><FluentIcon name="lock" :size="18" /></span>
        <div class="ct">
          <strong>密钥保存在系统凭据库</strong>（Windows 凭据管理器 / macOS 钥匙串），不写进任何文件——
          换电脑需要重新录入。
        </div>
      </div>
      <div v-for="s in secrets" :key="s.alias" class="field"
        style="border: 1px solid var(--stroke); border-radius: 6px; padding: 12px">
        <label>{{ s.label }} <span class="tag" :class="s.exists ? 'tag-success' : 'tag-muted'">{{ s.exists ? "已设置" : "未设置" }}</span></label>
        <div style="display: flex; gap: 8px">
          <input v-model="secretDrafts[s.alias]" class="input" type="password" autocomplete="new-password"
            :placeholder="s.exists ? '输入新值覆盖' : '输入密钥'" style="flex: 1" />
          <button class="btn btn-primary" @click="saveSecret(s.alias)">保存</button>
          <button class="btn" :disabled="!s.exists" @click="removeSecret(s.alias)">删除</button>
        </div>
      </div>
    </div>
  </div>
</template>
