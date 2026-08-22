/**
 * 后端 API 封装：类型与 Tauri invoke 一一对应（src-tauri/src/commands）
 */
import { invoke } from "@tauri-apps/api/core";

// ---------- 类型 ----------

export interface Voucher {
  id: number;
  voucherNo: string;
  voucherDate: string;
  description: string;
  /** 对方账户（收款方/商户，OCR 可自动抽取） */
  counterparty: string;
  amountCents: number | null;
  status: VoucherStatus;
  tags: string[];
  note: string;
  createdAt: string;
  updatedAt: string;
}

export type VoucherStatus = "draft" | "reviewed" | "exported" | "archived";

export interface VoucherUpdate {
  voucherNo?: string;
  voucherDate?: string;
  description?: string;
  counterparty?: string;
  amountCents?: number | null;
  clearAmount?: boolean;
  status?: VoucherStatus;
  tags?: string[];
  note?: string;
}

export interface Attachment {
  id: number;
  voucherId: number;
  fileName: string;
  filePath: string;
  fileHash: string;
  fileSize: number;
  mimeType: string;
  ocrStatus: "pending" | "running" | "done" | "failed";
  ocrText: string | null;
  /** v0.6：5 步增强后的 JPEG 相对路径（null = 未增强，原图回退） */
  enhancedPath: string | null;
  /** v0.6：来源分类（screenshot=截图 / scan=扫描件 / unknown=未识别） */
  enhancedSource: "screenshot" | "scan" | "unknown";
  createdAt: string;
}

/** 功能深化 2：批量编辑补丁（undefined = 不修改该字段；标签整体替换） */
export interface BatchEditPatch {
  voucherDate?: string;
  note?: string;
  tags?: string[];
}

export interface VoucherSnapshot {
  id: number;
  voucherId: number;
  payload: Record<string, unknown>;
  reason: string;
  createdAt: string;
}

export interface VoucherDetail {
  voucher: Voucher;
  attachments: Attachment[];
  snapshots: VoucherSnapshot[];
}

export interface VoucherQuery {
  status?: VoucherStatus | null;
  keyword?: string | null;
  dateFrom?: string | null;
  dateTo?: string | null;
  page: number;
  pageSize: number;
}

export interface VoucherListResult {
  items: Voucher[];
  total: number;
  /** D-1：当前页每张凭证的首附件缩略图信息（key = voucher.id）。无附件时不在 map 中 */
  thumbs: Record<number, VoucherThumb>;
}

/** D-1 列表缩略图：相对附件根的路径，前端用 convertFileSrc 拼 URL */
export interface VoucherThumb {
  filePath: string;
  enhancedPath: string | null;
  enhancedSource: "screenshot" | "scan" | "unknown";
}

export interface VoucherStats {
  total: number;
  draft: number;
  reviewed: number;
  exported: number;
  archived: number;
  attachments: number;
  thisMonth: number;
}

export interface MonthlySummary {
  month: string;
  count: number;
  amountCents: number;
  draft: number;
  reviewed: number;
  exported: number;
  archived: number;
}

/** v0.7 R-1：Top 对方账户行（按金额降序） */
export interface CounterpartyRow {
  name: string;
  count: number;
  amountCents: number;
}

/** 功能深化 3：标签分类统计行（tag 为空串 = 未分类，固定排末尾） */
export interface TagRow {
  tag: string;
  count: number;
  amountCents: number;
}

/** v0.7 S-2：OCR 提取预览（只读，不写库） */
export interface ExtractedFields {
  date: string | null;
  amountCents: number | null;
  counterparty: string | null;
  merchant: string | null;
  /** 金额来源：labeled=标签命中（可信） uniqueonly=全文唯一兜底（低置信） */
  amountSource: "labeled" | "uniqueonly" | null;
}

export interface RecoveryReport {
  abnormalExit: boolean;
  lastSession: { startedAt: string; pid: number } | null;
  dbOk: boolean;
  integrityErrors: string[];
  attachmentTotal: number;
  attachmentMissingCount: number;
  attachmentMissing: string[];
  lastBackup: BackupInfo | null;
}

export interface SearchHit {
  voucher: Voucher;
  matchedIn: string[];
}

/** 一次导入产生的一张凭证（一图一凭证：每张图片一张凭证） */
export interface ImportedVoucher {
  voucherId: number;
  voucherNo: string;
  attachment: Attachment;
}

export interface ImportSummary {
  /** 本次导入创建的凭证（每张图片一张） */
  vouchers: ImportedVoucher[];
  skippedDuplicates: string[];
  failed: { fileName: string; reason: string }[];
}

export interface ExportBatch {
  id: number;
  batchNo: string;
  voucherCount: number;
  pdfPath: string;
  status: "done" | "failed";
  createdAt: string;
}

export interface ExportOptions {
  title: string;
  cardStyle: "standard" | "clean" | "ledger";
  columns: number;
  includeImages: boolean;
}

export interface AppConfig {
  appearance: { theme: "light" | "dark" | "system" };
  naming: { template: string };
  ocr: { engine: "cloud" | "local"; enabled: boolean; apiUrl: string; model: string };
  backup: {
    autoEnabled: boolean;
    intervalHours: number;
    keepDaily: number;
    keepWeekly: number;
    keepMonthly: number;
    secondaryDir: string;
    webdav: {
      enabled: boolean;
      url: string;
      username: string;
      passwordSet: boolean;
      remoteRetention: number;
    };
  };
  wechat: { enabled: boolean; watchDir: string; mode: string };
  print: { title: string; cardStyle: string; columns: number; includeImages: boolean; groupByDay: boolean };
  /** v0.6：文档扫描矫正（5 步算法 + 2 占位） */
  enhance: EnhanceConfig;
}

/** v0.6 文档扫描矫正设置 */
export interface EnhanceConfig {
  /** 总开关：关闭后导入图片不再跑流水线 */
  enabled: boolean;
  /** 仅对扫描件（非截图）走流水线；true = 截图跳过 */
  onlyScans: boolean;
  stepDeskew: boolean;
  stepDewarp: boolean;
  stepAutocrop: boolean;
  stepGrayscale: boolean;
  stepBinarize: boolean;
  /** v0.6.1 评估：UI 占位，v0.6 不生效 */
  stepSharpen: boolean;
  /** v0.6.1 评估：UI 占位，v0.6 不生效 */
  stepDenoise: boolean;
}

/** v0.6 增强缓存状态（设置页显示） */
export interface EnhanceStatus {
  total: number;
  totalSize: number;
  screenshotCount: number;
  scanCount: number;
}

/** D-2 增强缓存状态 v2：摘要 + 命中率（D-8 起命中按来源分列） */
export interface EnhanceStatusV2 {
  total: number;
  totalSize: number;
  screenshotCount: number;
  scanCount: number;
  hitCount: number;
  hitCount30d: number;
  /** D-8：OCR 用增强版识别的次数 */
  hitCountOcr: number;
  /** D-8：增强版进 PDF 的次数（D-6 起仅扫描件） */
  hitCountExport: number;
  /** D-8：查看器打开增强版的次数（D-7 接通） */
  hitCountViewer: number;
  oldestEnhancedAt: string | null;
  newestEnhancedAt: string | null;
}

/** D-2 释放策略（D-11 新增 smart：零命中 + 宽限期） */
export type ClearStrategy = "all" | "older_than_days" | "larger_than_mb" | "smart";

/** D-2 释放结果 */
export interface ClearSummary {
  deletedFiles: number;
  freedBytes: number;
  strategy: ClearStrategy;
  threshold: number;
  candidateCount: number;
}

/** D-2 最近增强（设置页表格行） */
export interface RecentEnhancedRow {
  attachmentId: number;
  voucherId: number;
  voucherNo: string;
  fileName: string;
  fileSize: number;
  enhancedSize: number;
  enhancedSource: "screenshot" | "scan" | "unknown";
  enhancedAt: string;
  hitCount: number;
}

/** D-10 命中趋势单日点（ocr / export / viewer 分列） */
export interface HitTrendPoint {
  day: string;
  ocr: number;
  export: number;
  viewer: number;
  total: number;
}

/** D-10 命中趋势（近 N 天，缺日补零，maxTotal 供柱状图定标） */
export interface HitTrend {
  days: number;
  points: HitTrendPoint[];
  maxTotal: number;
}

export interface AppInfo {
  version: string;
  dataDir: string;
  dbPath: string;
  attachmentsDir: string;
  backupsDir: string;
  exportsDir: string;
  logsDir: string;
}

export interface SecretStatus {
  alias: string;
  label: string;
  exists: boolean;
}

/** 微信监听运行状态 */
export interface WechatStatus {
  enabled: boolean;
  running: boolean;
  watchDir: string;
  error: string | null;
}

export interface BackupInfo {
  fileName: string;
  path: string;
  size: number;
  createdAt: string;
  attachmentCount: number;
  /** 非阻断性告警（异盘/WebDAV 副本失败，本地主备份不受影响） */
  warnings: string[];
}

export interface RestoreSummary {
  batch: string;
  createdAt: string;
  restoredFiles: string[];
  attachmentCount: number;
  checksumsVerified: boolean;
}

export interface DrillReport {
  batch: string;
  createdAt: string;
  dbOk: boolean;
  filesChecked: number;
  attachmentCount: number;
  checksumsVerified: boolean;
}

/** WebDAV 远端备份行（PROPFIND 解析结果） */
export interface WebdavBackupRow {
  fileName: string;
}

export interface LocalOcrStatus {
  installed: boolean;
  modelsDir: string;
  files: { name: string; size: number }[];
  totalSize: number;
}

// ---------- API ----------

export const api = {
  getAppInfo: () => invoke<AppInfo>("get_app_info"),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  restartApp: () => invoke<void>("restart_app"),

  getConfig: () => invoke<AppConfig>("get_config"),
  saveConfig: (config: AppConfig) => invoke<AppConfig>("save_config", { config }),
  validateNamingTemplate: (template: string) => invoke<string>("validate_naming_template", { template }),
  previewVoucherNo: (date?: string) => invoke<string>("preview_voucher_no", { date: date ?? "" }),

  listSecrets: () => invoke<SecretStatus[]>("list_secrets"),
  setSecret: (alias: string, value: string) => invoke<void>("set_secret", { alias, value }),
  /** 微信监听运行状态（保存配置后热重载，无需重启） */
  getWechatStatus: () => invoke<WechatStatus>("get_wechat_status"),
  deleteSecret: (alias: string) => invoke<boolean>("delete_secret", { alias }),

  listVouchers: (query: VoucherQuery) => invoke<VoucherListResult>("list_vouchers", { query }),
  getVoucher: (id: number) => invoke<VoucherDetail>("get_voucher", { id }),
  updateVoucher: (id: number, update: VoucherUpdate) => invoke<Voucher>("update_voucher", { id, update }),
  /** 批量删除（单事务 + 附件文件清理），返回实际删除数 */
  deleteVouchers: (ids: number[]) => invoke<number>("delete_vouchers", { ids }),
  /** v0.7 B-1：批量状态流转（单事务逐张快照），返回实际变更数 */
  batchUpdateStatus: (ids: number[], status: VoucherStatus) =>
    invoke<number>("batch_update_status", { ids, status }),
  batchEdit: (ids: number[], patch: BatchEditPatch) =>
    invoke<number>("batch_edit", { ids, patch }),
  voucherStats: () => invoke<VoucherStats>("voucher_stats"),

  monthlySummary: (months?: number) => invoke<MonthlySummary[]>("monthly_summary", { months }),

  /** v0.7 R-1：Top 对方账户（months=0 全部历史；limit 默认 10） */
  topCounterparties: (months?: number, limit?: number) =>
    invoke<CounterpartyRow[]>("top_counterparties", { months, limit }),
  tagStats: (months?: number, limit?: number) =>
    invoke<TagRow[]>("tag_stats", { months, limit }),

  getRecoveryReport: () => invoke<RecoveryReport>("get_recovery_report"),

  searchVouchers: (keyword: string, limit?: number) => invoke<SearchHit[]>("search_vouchers", { keyword, limit }),

  /** Rust 侧对话框选图并直接导入（绕开 JS 对话框桥的竞态）。null = 用户取消 */
  pickAndImport: (runOcr: boolean) =>
    invoke<ImportSummary | null>("pick_and_import", { runOcr }),
  /** Rust 侧对话框选图更换凭证图片（换图不换卡）。null = 用户取消 */
  pickAndReplaceImage: (voucherId: number) =>
    invoke<VoucherDetail | null>("pick_and_replace_image", { voucherId }),
  /** Rust 侧对话框选目录。null = 用户取消 */
  pickDirectory: () => invoke<string | null>("pick_directory"),
  attachmentAbsPath: (id: number) => invoke<string>("attachment_abs_path", { id }),
  /** 识别附件并自动填单（只填空字段）；附件可能被语义化重命名，返回整份凭证详情供刷新 */
  runOcr: (attachmentId: number) => invoke<VoucherDetail>("run_ocr", { attachmentId }),
  /** v0.7 S-2：OCR 字段提取预览（只读） */
  ocrPreviewFields: (attachmentId: number) =>
    invoke<ExtractedFields>("ocr_preview_fields", { attachmentId }),
  runOcrPending: (limit?: number) => invoke<number>("run_ocr_pending", { limit }),
  getLocalOcrStatus: () => invoke<LocalOcrStatus>("local_ocr_status"),

  exportPdf: (voucherIds: number[], options?: ExportOptions) =>
    invoke<ExportBatch>("export_pdf", { voucherIds, options }),
  /** 按天分组导出：每个日期一份「凭证日报」PDF（一图一凭证的默认导出形态） */
  exportPdfDaily: (voucherIds: number[], options?: ExportOptions) =>
    invoke<ExportBatch[]>("export_pdf_daily", { voucherIds, options }),
  listExportBatches: (limit?: number) => invoke<ExportBatch[]>("list_export_batches", { limit }),

  runBackup: () => invoke<BackupInfo>("run_backup"),
  listBackups: () => invoke<BackupInfo[]>("list_backups"),
  inspectBackup: (path: string) => invoke<RestoreSummary>("inspect_backup", { path }),
  restoreBackup: (path: string) => invoke<RestoreSummary>("restore_backup", { path }),
  drillBackup: (path?: string) =>
    invoke<DrillReport>("drill_backup", { path: path ?? null }),
  uploadBackup: (path: string) => invoke<void>("upload_backup", { path }),
  testWebdav: () => invoke<number>("test_webdav"),
  listWebdavBackups: () => invoke<WebdavBackupRow[]>("list_webdav_backups"),
  downloadWebdavBackup: (fileName: string) =>
    invoke<string>("download_webdav_backup", { fileName }),

  // v0.6 文档扫描矫正
  enhanceStatus: () => invoke<EnhanceStatus>("enhance_status"),
  clearEnhanceCache: () => invoke<number>("clear_enhance_cache"),

  // v0.6.5 D-2 增强缓存管理
  enhanceStatusV2: () => invoke<EnhanceStatusV2>("enhance_status_v2"),
  clearEnhanceCacheV2: (strategy: ClearStrategy, threshold: number) =>
    invoke<ClearSummary>("clear_enhance_cache_v2", { strategy, threshold }),
  deleteEnhancedOne: (attachmentId: number) =>
    invoke<number>("delete_enhanced_one", { attachmentId }),
  recentEnhanced: (limit: number) =>
    invoke<RecentEnhancedRow[]>("recent_enhanced", { limit }),
  /** D-10：命中趋势（近 N 天按日聚合） */
  enhanceHitTrend: (days: number) =>
    invoke<HitTrend>("enhance_hit_trend", { days }),

  // v0.6.5 D-7 查看器命中埋点（静默调用，失败不打扰用户）
  recordViewerHit: (attachmentId: number) =>
    invoke<void>("record_viewer_hit", { attachmentId }),
};
