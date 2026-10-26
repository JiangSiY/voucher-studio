/**
 * 前端「配置默认值」单一真源（SettingsPage 编辑缓冲占位 / 后端不可达时的兜底）。
 *
 * ⚠️ 必须与 crates/voucher-core/src/utils/config.rs 的 AppConfig::default() 保持同步。
 * 运行期真源是后端 `get_config`（Rust 侧 serde default 落盘）；本模块仅用于：
 *   1. SettingsPage 构造保存前编辑副本的初始值（load 后会被后端值覆盖）；
 *   2. 后端不可达 / 字段缺失时的界面兜底。
 * 任一方向改动默认值，都必须同步本文件，并同步更新
 * src/__tests__/config-defaults.test.ts 的断言（长期方案：ts-rs 从 Rust 生成 TS 类型）。
 *
 * 命名模板令牌（与 services/naming.rs 一致）：{date}=YYYYMMDD、{date-}=YYYY-MM-DD、
 * {seq}=当日三位序号（001 起）、{n}=当日两位序号（01 起）。
 */
import type { AppConfig } from "./api/client";

/** 默认凭证编号模板：形如 2026-08-19-01 */
export const DEFAULT_NAMING_TEMPLATE = "{date-}-{n}";

/** 默认界面字号倍数（与 Rust `UI_SCALE_DEFAULT` 一致；1 = 标准，另有 1.15/1.3 档） */
export const UI_SCALE_DEFAULT = 1.15;

/** 界面字号档位（设置页选项；值写入 CSS 变量 `--font-scale`） */
export const UI_SCALE_OPTIONS = [
  { value: 1, label: "标准" },
  { value: 1.15, label: "大" },
  { value: 1.3, label: "更大" },
] as const;

/** 默认打印整理单标题 */
export const DEFAULT_PRINT_TITLE = "原始凭证";

/** 侧边栏宽度范围与默认值（必须与 crates/voucher-core/src/utils/config.rs 的同名常量一致） */
export const SIDEBAR_WIDTH_MIN = 184;
export const SIDEBAR_WIDTH_MAX = 400;
export const SIDEBAR_WIDTH_DEFAULT = 220;

/**
 * 侧边栏默认是否收起（收起后 56px 图标栏；用户 2026-10-16 要求）。
 * 必须与 Rust `AppearanceConfig::default()` 的 `sidebar_collapsed` 一致。
 */
export const SIDEBAR_COLLAPSED_DEFAULT = false;

/** 与 Rust AppConfig::default() 逐字段对齐的前端默认配置。 */
export function defaultConfig(): AppConfig {
  return {
    // uiScale：界面字号倍数。**默认偏大**——用户视力不好，明确要求"字号要大一些"
    // （2026-09-24 反馈；当时正文 13px 被评为太小）。与 Rust `UI_SCALE_DEFAULT` 同步。
    appearance: {
      theme: "system",
      sidebarWidth: SIDEBAR_WIDTH_DEFAULT,
      uiScale: UI_SCALE_DEFAULT,
      sidebarCollapsed: SIDEBAR_COLLAPSED_DEFAULT,
    },
    naming: { template: DEFAULT_NAMING_TEMPLATE },
    // OCR 默认云端优先（DeepSeek 视觉），云端不可用自动回退本地 Apple Vision。
    // 与 Rust `OcrConfig::default()` 保持一致（apiUrl/model 留空 = 用 DeepSeek 默认值）。
    ocr: {
      engine: "auto",
      provider: "deepseek",
      enabled: true,
      apiUrl: "",
      model: "",
    },
    backup: {
      autoEnabled: true,
      intervalHours: 24,
      autoBackupOnClose: false,
      keepDaily: 7,
      keepWeekly: 4,
      keepMonthly: 6,
      secondaryDir: "",
      encrypted: false,
      webdav: {
        enabled: false,
        url: "",
        username: "",
        passwordSet: false,
        remoteRetention: 10,
        onceDaily: false,
      },
    },
    print: {
      title: DEFAULT_PRINT_TITLE,
      includeImages: true,
      dailyTemplate: "compact",
      preferEnhancedImages: true,
    },
    enhance: {
      enabled: true,
      onlyScans: false,
      stepDeskew: true,
      stepDewarp: true,
      stepAutocrop: true,
      stepGrayscale: true,
      stepBinarize: true,
      stepSharpen: false,
      stepDenoise: false,
    },
    voucher: { descriptionTemplates: [] },
  };
}
