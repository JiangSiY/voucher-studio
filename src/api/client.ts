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
  createdAt: string;
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
  voucherStats: () => invoke<VoucherStats>("voucher_stats"),

  monthlySummary: (months?: number) => invoke<MonthlySummary[]>("monthly_summary", { months }),

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
};
