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

/** 默认打印整理单标题 */
export const DEFAULT_PRINT_TITLE = "原始凭证";

/** 侧边栏宽度范围与默认值（必须与 crates/voucher-core/src/utils/config.rs 的同名常量一致） */
export const SIDEBAR_WIDTH_MIN = 220;
export const SIDEBAR_WIDTH_MAX = 400;
export const SIDEBAR_WIDTH_DEFAULT = 260;

/** 与 Rust AppConfig::default() 逐字段对齐的前端默认配置。 */
export function defaultConfig(): AppConfig {
  return {
    appearance: { theme: "system", sidebarWidth: SIDEBAR_WIDTH_DEFAULT },
    naming: { template: DEFAULT_NAMING_TEMPLATE },
    ocr: { engine: "local", enabled: true, apiUrl: "", model: "" },
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
      cardStyle: "standard",
      columns: 3,
      includeImages: true,
      groupByDay: true,
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
