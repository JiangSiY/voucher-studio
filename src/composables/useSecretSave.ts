/**
 * 密钥落库（体检 Q-15 收口：密钥管理 Tab 与备份 Tab 此前各抄一份）。
 * 判空 → setSecret → 清草稿 → 刷新共享状态；返回是否写入成功，
 * 供调用方决定后续动作（如 WebDAV 的 passwordSet 打标只在成功时进行）。
 */
import { api } from "../api/client";
import { refreshSecrets, secretDrafts } from "./secretDrafts";
import { toast, toastError } from "../stores/toast";

export async function useSecretSave(alias: string): Promise<boolean> {
  const value = secretDrafts[alias]?.trim();
  if (!value) {
    toast.warning("请输入内容");
    return false;
  }
  try {
    await api.setSecret(alias, value);
    secretDrafts[alias] = "";
    await refreshSecrets();
    toast.success("已写入系统凭据库");
    return true;
  } catch (e) {
    toastError(e);
    return false;
  }
}
