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
  amountCents: number | null;
  status: VoucherStatus;
  tags: string[];
  note: string;
  createdAt: string;
  updatedAt: string;
}

export type VoucherStatus = "draft" | "reviewed" | "exported" | "archived";

export interface NewVoucher {
  voucherNo: string;
  voucherDate: string;
  description: string;
  amountCents: number | null;
  status: VoucherStatus | null;
  tags: string[];
  note: string;
}

export interface VoucherUpdate {
  voucherNo?: string;
  voucherDate?: string;
  description?: string;
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

export interface SearchHit {
  voucher: Voucher;
  matchedIn: string[];
}

export interface ImportSummary {
  voucherId: number;
  voucherNo: string;
  imported: Attachment[];
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
  print: { title: string; cardStyle: string; columns: number; includeImages: boolean };
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
  createVoucher: (newVoucher: NewVoucher) => invoke<Voucher>("create_voucher", { newVoucher }),
  updateVoucher: (id: number, update: VoucherUpdate) => invoke<Voucher>("update_voucher", { id, update }),
  deleteVoucher: (id: number) => invoke<void>("delete_voucher", { id }),
  /** 批量删除（单事务 + 附件文件清理），返回实际删除数 */
  deleteVouchers: (ids: number[]) => invoke<number>("delete_vouchers", { ids }),
  setVoucherStatus: (id: number, status: VoucherStatus) => invoke<void>("set_voucher_status", { id, status }),
  voucherStats: () => invoke<VoucherStats>("voucher_stats"),

  searchVouchers: (keyword: string, limit?: number) => invoke<SearchHit[]>("search_vouchers", { keyword, limit }),

  importImages: (paths: string[], voucherId: number | null, runOcr: boolean) =>
    invoke<ImportSummary>("import_images", { paths, voucherId, runOcr }),
  deleteAttachment: (id: number) => invoke<void>("delete_attachment", { id }),
  attachmentAbsPath: (id: number) => invoke<string>("attachment_abs_path", { id }),
  runOcr: (attachmentId: number) => invoke<Attachment>("run_ocr", { attachmentId }),
  runOcrPending: (limit?: number) => invoke<number>("run_ocr_pending", { limit }),
  getLocalOcrStatus: () => invoke<LocalOcrStatus>("local_ocr_status"),

  exportPdf: (voucherIds: number[], options?: ExportOptions) =>
    invoke<ExportBatch>("export_pdf", { voucherIds, options }),
  listExportBatches: (limit?: number) => invoke<ExportBatch[]>("list_export_batches", { limit }),

  runBackup: () => invoke<BackupInfo>("run_backup"),
  listBackups: () => invoke<BackupInfo[]>("list_backups"),
  inspectBackup: (path: string) => invoke<RestoreSummary>("inspect_backup", { path }),
  restoreBackup: (path: string) => invoke<RestoreSummary>("restore_backup", { path }),
  drillBackup: (path?: string) =>
    invoke<DrillReport>("drill_backup", { path: path ?? null }),
  uploadBackup: (path: string) => invoke<void>("upload_backup", { path }),
};
