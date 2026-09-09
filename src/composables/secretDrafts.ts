/**
 * 设置页密钥草稿与凭据状态（跨 Tab 共享）：
 * 密钥管理 Tab 录入/展示；备份 Tab 的「设置密码」按钮读取同一份草稿。
 * （拆分前二者同居 SettingsPage；模块级单例保持同一行为语义。）
 */
import { reactive, ref } from "vue";
import { api, type SecretStatus } from "../api/client";

export const secretDrafts = reactive<Record<string, string>>({});
export const secrets = ref<SecretStatus[]>([]);

export async function refreshSecrets() {
  secrets.value = await api.listSecrets();
}
