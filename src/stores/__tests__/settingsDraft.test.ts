// @vitest-environment node
/**
 * settingsDraft 编辑缓冲 store 测试（体检 #22 v-model 单向流改造）。
 *
 * 该 store 取代「SettingsPage 下发可变 cfg prop + JSON 深拷贝」的旧方案：
 * - cfg 是唯一共享编辑缓冲，各 Tab 只读写 store 状态（不经 props 边界）；
 * - load 走 replace（整体替换 + 旧配置缺 voucher 段兜底）；
 * - 多个组件拿到的是同一份缓冲（跨 Tab 语义与原共享 buffer 一致）。
 */
import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import type { AppConfig } from "../../api/client";
import { defaultConfig } from "../../config-defaults";
import { useSettingsDraft } from "../settingsDraft";

describe("useSettingsDraft", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("初始缓冲等于默认配置（单一真源 config-defaults）", () => {
    const draft = useSettingsDraft();
    expect(draft.cfg).toEqual(defaultConfig());
  });

  it("replace 整体替换为后端配置；缺 voucher 段时补齐空模板", () => {
    const draft = useSettingsDraft();

    // 模拟极旧配置：无 voucher 段
    const { voucher: _omitted, ...noVoucher } = defaultConfig();
    draft.replace(noVoucher as unknown as AppConfig);
    expect(draft.cfg.voucher).toEqual({ descriptionTemplates: [] });

    // 正常后端配置整体替换
    const backend: AppConfig = { ...defaultConfig(), naming: { template: "{date}-{seq}" } };
    draft.replace(backend);
    expect(draft.cfg.naming.template).toBe("{date}-{seq}");
  });

  it("两个取用方共享同一份缓冲（跨 Tab 编辑可见）", () => {
    const a = useSettingsDraft();
    const b = useSettingsDraft();
    a.cfg.appearance.theme = "dark";
    expect(b.cfg.appearance.theme).toBe("dark");
  });
});
