/**
 * 后端 API 封装：类型与 Tauri invoke 一一对应（src-tauri/src/commands）
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";

// 错误归一工具（结构化 ApiError{code,message} 解析）；独立纯模块便于 node 环境测试
export { asApiError, errMsg, type ApiErrorPayload } from "./errors";

/** 是否运行在 Tauri 桌面壳内（纯浏览器打开 dev URL 时为 false） */
export const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/**
 * 统一 invoke 封装：纯浏览器环境（如误开 dev URL）下返回可读的中文错误，
 * 避免 UI 弹出 "Cannot read properties of undefined (reading 'invoke')" 这类裸 JS 错误。
 */
function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) {
    return Promise.reject(
      new Error("桌面功能不可用：当前页面运行在纯浏览器中，请通过 Voucher Studio 桌面应用使用"),
    );
  }
  return tauriInvoke<T>(cmd, args);
}

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
  /**
   * 下发给界面的**绝对路径**（由 Rust 侧用平台原生分隔符拼好）。
   * 前端只把它交给 `convertFileSrc`，**不要自行拼接**——前端拼分隔符曾在 macOS
   * 上把路径拼成 `...VoucherStudio\attachments\...`，被 asset protocol 全部拒绝。
   */
  fileAbs?: string | null;
  /** 增强版绝对路径（不可显示时为 null，与 enhancedPath 同口径） */
  enhancedAbs?: string | null;
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
  /** v0.18：**精确**标签筛选（任一命中）；与 keyword 的模糊匹配区分 */
  tags?: string[];
  /** v0.18：**精确**对方账户（整串相等） */
  counterparty?: string | null;
  /** v0.18：金额区间（分） */
  amountFrom?: number | null;
  amountTo?: number | null;
  /** v0.18：只筛"金额为空"（对应"未填金额"待办） */
  amountEmpty?: boolean;
  /** v0.18：只筛"对方账户未填"（对应月度透视里的「(未填)」行） */
  counterpartyEmpty?: boolean;
  /** v0.18：附件 OCR 状态（**支持多值**，逗号分隔，任一命中；如 `pending,failed`） */
  ocrStatus?: string | null;
  /** v0.18：只筛"陈旧"凭证（updated_at 早于 现在−N 天），与收件箱"陈旧草稿"计数同口径 */
  staleDays?: number | null;
  /** v0.18：导出覆盖度 never | any */
  exported?: "never" | "any" | null;
  /** v0.22：未被佐证（既无任何关联、也不属于任何事项组）；false 只看已有佐证的 */
  unlinked?: boolean | null;
  /** v0.18：排序 date_desc（默认）| date_asc | updated_desc | amount_desc | amount_asc */
  sort?: string | null;
  /** 关键词同时匹配附件 OCR 文本（搜索页进入编辑页后翻张与搜索结果同口径） */
  includeOcr?: boolean;
  page: number;
  pageSize: number;
}

export interface VoucherListResult {
  items: Voucher[];
  total: number;
  /** D-1：当前页每张凭证的首附件缩略图信息（key = voucher.id）。无附件时不在 map 中 */
  thumbs: Record<number, VoucherThumb>;
}

/** v0.9 编辑动线：同筛选/同排序下的相邻凭证与位置（导航条 + 保存并下一张） */
export interface VoucherNeighbor {
  prevId: number | null;
  nextId: number | null;
  /** 当前凭证在筛选结果中的位置（从 1 开始） */
  position: number;
  total: number;
}

/** D-1 列表缩略图：相对路径（保留兼容）+ Rust 侧拼好的绝对路径（界面用这个） */
export interface VoucherThumb {
  filePath: string;
  enhancedPath: string | null;
  enhancedSource: "screenshot" | "scan" | "unknown";
  /** 原图绝对路径（Rust 侧拼好；前端直接用，不要自行拼接） */
  fileAbs?: string | null;
  /** 增强版绝对路径（不可显示时为 null） */
  enhancedAbs?: string | null;
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
  /** v5：字段级置信度（版面几何配对 high / 标签命中 medium / 启发式兜底 low；null=未抽出） */
  confidence: {
    date: "high" | "medium" | "low" | null;
    amount: "high" | "medium" | "low" | null;
    counterparty: "high" | "medium" | "low" | null;
    merchant: "high" | "medium" | "low" | null;
  };
}

export interface RecoveryReport {
  abnormalExit: boolean;
  lastSession: { startedAt: string; pid: number } | null;
  dbOk: boolean;
  integrityErrors: string[];
  attachmentTotal: number;
  attachmentMissingCount: number;
  attachmentMissing: string[];
  /** 增强版文件缺失数（可重建缓存；恢复快照后必然出现，界面自动回退原图） */
  attachmentMissingEnhanced: number;
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
  /** v0.10：导出格式 pdf（默认）| xlsx | csv */
  format?: "pdf" | "xlsx" | "csv";
}

/** v0.10：表格式导出格式 */
export type TableFormat = "xlsx" | "csv";

/** v0.10：高频摘要行（编辑页联想） */
export interface DescriptionRow {
  description: string;
  count: number;
}

/** v0.16：置信度分档（只用于排序与提示强弱，不做过滤） */
export type EvidenceLevel = "high" | "medium" | "low";

/** v0.16：一对疑似重复的证据（人工核对用；判定口径仍是同日期同金额） */
export interface PairEvidence {
  /** 两张凭证的首附件内容哈希相同（唯一强证据） */
  sameImage: boolean;
  sameCounterparty: boolean;
  sameDescription: boolean;
  level: EvidenceLevel;
}

/** v0.16：保存后提示项（对端凭证 + 证据） */
export interface DuplicateSuspect {
  voucher: Voucher;
  evidence: PairEvidence;
}

/** v0.16：待核查的一对 */
export interface ReviewPair {
  a: Voucher;
  b: Voucher;
  evidence: PairEvidence;
  decided?: DupDecisionKind | null;
}

/** v0.16：待核查组（同日期同金额；只含还有未判定对的组） */
export interface ReviewGroup {
  voucherDate: string;
  amountCents: number;
  vouchers: Voucher[];
  /** 未判定的对（高置信度在前） */
  pending: ReviewPair[];
  decidedPairs: number;
}

/** v0.16：待处理统计（侧栏徽标 / 仪表盘面板） */
export interface PendingSummary {
  groups: number;
  pairs: number;
}

/** v0.16：处置结论 */
export type DupDecisionKind = "not_duplicate" | "duplicate_removed";

/** v0.16：一条处置记录（`removedPayload` 为被删凭证留痕快照） */
export interface DupDecision {
  id: number;
  aId: number;
  bId: number;
  decision: DupDecisionKind;
  keepId?: number | null;
  removedId?: number | null;
  removedPayload?: {
    /** 被删凭证的完整快照 */
    voucher?: Voucher;
    /** 被删凭证的附件清单（含内容哈希） */
    attachments?: Attachment[];
    /** 保留侧（被删的依据：留的是哪一张） */
    keptVoucherId?: number;
    keptVoucherNo?: string;
  } | null;
  note: string;
  createdAt: string;
}

/** v0.18：工作台窗口内的状态分布（**口径**：始终是窗口内的完整构成，不随顶部状态筛选收窄） */
export interface StatusCount {
  status: string;
  count: number;
  amountCents: number;
}

/**
 * v0.18：工作台概览（**统一窗口口径**）
 *
 * - 窗口由 `dateFrom`/`dateTo`（`YYYY-MM-DD`，含端点）界定，两端口径与列表页筛选一致；
 * - `prevAmountCents`/`prevCount` 仅在两端口都给时给出（上一**等长天数**窗口，供算环比）；
 * - `total`/`attachments`/`thisMonth` 是**全库口径**的固定项，界面须标注"全库"。
 */
export interface DashboardOverview {
  dateFrom: string | null;
  dateTo: string | null;
  status: string | null;
  count: number;
  amountCents: number;
  prevAmountCents: number | null;
  prevCount: number | null;
  byStatus: StatusCount[];
  total: number;
  attachments: number;
  thisMonth: number;
}

/** v0.18：最近一份本地备份（轻量：只看目录里的文件名与时间，不读 manifest） */
export interface BackupLite {
  fileName: string;
  modifiedAt: string;
}

/** v0.18：待办与健康度（数值为 0 时界面可隐藏该项；每项都能点进对应筛选） */
export interface DashboardTodos {
  ocrPending: number;
  amountMissing: number;
  staleDrafts: number;
  staleDraftDays: number;
  unlockedMonths: number;
  latestBackup: BackupLite | null;
  backupStale: boolean;
  dupPairs: number;
  dupHighPairs: number;
  dupGroups: number;
}

/** v0.18：月度透视行（月份 × 对方/标签；每月只取前 N 个键） */
export interface BreakdownRow {
  month: string;
  key: string;
  count: number;
  amountCents: number;
}

/** v0.18：导出格式分布 */
export interface FormatCount {
  format: string;
  batches: number;
  vouchers: number;
}

/** v0.18：导出覆盖度（口径：`neverExported` 含已归档；界面须标注"含归档"） */
export interface ExportCoverage {
  total: number;
  exported: number;
  neverExported: number;
  byFormat: FormatCount[];
}

/** v0.18：存储占用（原图体积走 SQL；`enhancedBytes` 仅在显式请求时计算） */
export interface StorageUsage {
  originalCount: number;
  originalBytes: number;
  enhancedCount: number;
  enhancedScreenshot: number;
  enhancedScan: number;
  enhancedUnknown: number;
  enhancedBytes: number | null;
  ocrCacheCount: number;
}

/** v0.18：疑似重复轻量计数（徽标 / 待办卡；与核查队列口径一致，第 3 位为高置信度对数） */
export interface DupPendingCount {
  groups: number;
  pairs: number;
  highPairs: number;
}

/** v0.11：会计期间锁（period = YYYY-MM） */
export interface PeriodLock {
  period: string;
  lockedAt: string;
}

/** v0.11：搜索组合筛选（与关键词 AND 组合；金额单位：分） */
export interface SearchFilters {
  dateFrom?: string | null;
  dateTo?: string | null;
  amountFrom?: number | null;
  amountTo?: number | null;
}

/** v0.13：账单记录（微信/支付宝 CSV 解析产物） */
export interface BillRecord {
  date: string;
  amountCents: number;
  direction: string;
  counterparty: string;
  subject: string;
  lineNo: number;
}

/** v0.13：对账匹配（账单 ↔ 凭证） */
export interface ReconcileMatch {
  bill: BillRecord;
  voucher: Voucher;
}

/** v0.13：对账报告（只读匹配；报告 CSV 自动落 exports/） */
export interface ReconcileReport {
  billCount: number;
  matched: ReconcileMatch[];
  billOnly: BillRecord[];
  voucherOnly: Voucher[];
  dateFrom: string;
  dateTo: string;
  reportPath: string | null;
}

export interface ExportOptions {
  title: string;
  cardStyle: "standard" | "clean" | "ledger";
  columns: number;
  includeImages: boolean;
  /**
   * v0.22 批次③：日报是否附带「相关凭证清单」页（按关系类型分类小计）。
   * **用户定的默认是"带"**；后端字段级默认同样为 true（老调用不带这个键＝带）。
   */
  includeLinks?: boolean;
}

export interface AppConfig {
  /** 外观：主题 / 侧栏宽度 / 界面字号倍数（1 = 标准，1.15 / 1.3 为大字号档）/ 侧栏是否收起 */
  appearance: {
    theme: "light" | "dark" | "system";
    sidebarWidth: number;
    uiScale: number;
    /** 收起后侧栏只显示图标（56px 图标栏）；默认 false = 展开 */
    sidebarCollapsed: boolean;
  };
  naming: { template: string };
  ocr: { engine: "cloud" | "local"; enabled: boolean; apiUrl: string; model: string };
  backup: {
    autoEnabled: boolean;
    intervalHours: number;
    autoBackupOnClose: boolean;
    keepDaily: number;
    keepWeekly: number;
    keepMonthly: number;
    secondaryDir: string;
    /** v0.13 引入；v0.14-A 起备份文件以 age 信封整文件密封（ChaCha20-Poly1305 AEAD；密码存凭据管理器；本地/异盘/云端全覆盖） */
    encrypted: boolean;
    webdav: {
      enabled: boolean;
      url: string;
      username: string;
      passwordSet: boolean;
      remoteRetention: number;
      /** v0.8：云端每日最多上传一份（流量保护，当日已有则跳过） */
      onceDaily: boolean;
    };
  };
  print: {
    title: string;
    cardStyle: string;
    columns: number;
    includeImages: boolean;
    groupByDay: boolean;
    /** v0.8 日报模板：compact（省纸）| archive（档案装订） */
    dailyTemplate: "compact" | "archive";
    /** v0.8 打印是否优先增强版图片（仅扫描件；关闭始终用原图） */
    preferEnhancedImages: boolean;
  };
  /** v0.6：文档扫描矫正（5 步算法 + 2 占位） */
  enhance: EnhanceConfig;
  /** v0.10：凭证录入辅助 */
  voucher: {
    /** 常用摘要模板（编辑页联想 = 模板 + 历史高频摘要合并去重） */
    descriptionTemplates: string[];
  };
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

export interface BackupInfo {
  fileName: string;
  path: string;
  size: number;
  createdAt: string;
  attachmentCount: number;
  /** 非阻断性告警（异盘/WebDAV 副本失败，本地主备份不受影响） */
  warnings: string[];
  /** v0.9：snapshot（快照，附件在共享附件包里）| full（全量单文件，含旧备份） */
  kind?: "snapshot" | "full";
}

export interface RestoreSummary {
  batch: string;
  createdAt: string;
  restoredFiles: string[];
  /** 实际释放的附件数（≠ 备份引用过的附件数：快照只含原图） */
  attachmentCount: number;
  /** 本次备份是否携带附件实体；false 时界面必须如实提示，不得宣称已覆盖附件目录 */
  attachmentsIncluded: boolean;
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

/** v0.22 凭证关系类型：explains 附件说明 / detail 明细（都不带金额，不改合计口径） */
export type VoucherLinkKind = "explains" | "detail";

/** 一条两两关联（a_id < b_id 归一化后落库，两向都算） */
export interface VoucherLink {
  id: number;
  aId: number;
  bId: number;
  kind: VoucherLinkKind;
  note: string;
  createdAt: string;
}

/** 右栏「相关凭证」一行：关系 + 对方凭证（用于展示与跳转） */
export interface LinkedVoucher {
  linkId: number;
  kind: VoucherLinkKind;
  note: string;
  createdAt: string;
  other: Voucher;
  isA: boolean;
}

/** 关联审计事件（append-only，独立于凭证生命周期） */
export interface VoucherLinkEvent {
  id: number;
  linkId: number;
  aId: number;
  bId: number;
  kind: string;
  action: string;
  note: string;
  at: string;
}

/** 事项组（组内成员＝「明细」；组内合计只是派生展示值） */
export interface VoucherGroup {
  id: number;
  title: string;
  note: string;
  createdAt: string;
  members: Voucher[];
}

/** v0.22：新建入参（留空字段由后端补默认：编号自动生成、日期取今天） */
export interface NewVoucherInput {
  voucherNo?: string;
  voucherDate?: string;
  description?: string;
  counterparty?: string;
  amountCents?: number | null;
  status?: VoucherStatus | null;
  tags?: string[];
  note?: string;
}

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
  deleteSecret: (alias: string) => invoke<boolean>("delete_secret", { alias }),

  listVouchers: (query: VoucherQuery) => invoke<VoucherListResult>("list_vouchers", { query }),
  getVoucher: (id: number) => invoke<VoucherDetail>("get_voucher", { id }),
  /** v0.9 编辑动线：同筛选/同排序下的相邻凭证与位置（「保存并下一张」用） */
  getVoucherNeighbor: (id: number, query: VoucherQuery) =>
    invoke<VoucherNeighbor>("get_voucher_neighbor", { id, query }),
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

  /** v0.10：高频摘要（编辑页联想；自定义模板在 config.voucher.descriptionTemplates） */
  frequentDescriptions: (limit?: number) =>
    invoke<DescriptionRow[]>("frequent_descriptions", { limit }),
  /**
   * v0.16：单凭证疑似重复（保存后提示用）——同日期同金额、**尚未判定**的对端 + 证据。
   * 已判定（不是重复 / 已确认并删除）的对不再返回，故提示不会反复重现。
   */
  checkVoucherDuplicates: (id: number) =>
    invoke<DuplicateSuspect[]>("check_voucher_duplicates", { id }),

  // ---- v0.16：疑似重复处置闭环 ----
  /** 待核查队列：只含还有未判定对的组，组内对按置信度排序 */
  scanDuplicateReview: () => invoke<ReviewGroup[]>("scan_duplicate_review"),
  /** 待处理统计（侧栏徽标 / 仪表盘面板） */
  duplicatePendingSummary: () => invoke<PendingSummary>("duplicate_pending_summary"),
  /** 处置历史（新 → 旧；含被删凭证留痕） */
  listDuplicateDecisions: (limit?: number) =>
    invoke<DupDecision[]>("list_duplicate_decisions", { limit }),
  /** 判定"不是重复"（不动凭证，故不受封账限制）；返回处置记录 id */
  resolveDuplicatePair: (aId: number, bId: number, note?: string) =>
    invoke<number>("resolve_duplicate_pair", { aId, bId, note }),
  /** 确认重复：保留 keepId、删除 removeId（单事务留痕 + 删除）；返回删除数 */
  removeDuplicateVoucher: (keepId: number, removeId: number, note?: string) =>
    invoke<number>("remove_duplicate_voucher", { keepId, removeId, note }),

  // ---- v0.18：工作台（仪表盘）只读聚合 ----
  /**
   * 工作台概览：**统一窗口口径**（`dateFrom`/`dateTo` 为 `YYYY-MM-DD`，含端点；
   * 两端都给时才附上一等长窗口的金额/张数供算环比）。
   * `byStatus` 始终是窗口内的**完整构成**，不随 `status` 收窄（供堆叠条展示）。
   */
  dashboardOverview: (dateFrom?: string | null, dateTo?: string | null, status?: VoucherStatus | null) =>
    invoke<DashboardOverview>("dashboard_overview", { dateFrom, dateTo, status }),
  /** 待办与健康度：待识别 / 未填金额 / 陈旧草稿 / 未封账往月 / 备份新鲜度 / 疑似重复 */
  dashboardTodos: () => invoke<DashboardTodos>("dashboard_todos"),
  /** 疑似重复**轻量**计数（徽标 / 待办卡）：不构造队列，口径与核查队列一致 */
  duplicatePendingCount: () => invoke<DupPendingCount>("duplicate_pending_count"),
  /** 按当前筛选只取凭证 id（列表页「按筛选全选 N 条」的跨页选择集；顺序同列表） */
  listVoucherIds: (query: VoucherQuery) => invoke<number[]>("list_voucher_ids", { query }),
  /** v0.18：月度透视（dim = counterparty | tag；每月前 perMonth 个键，按金额倒序） */
  monthlyBreakdown: (dateFrom?: string | null, dateTo?: string | null, dim?: "counterparty" | "tag", perMonth?: number) =>
    invoke<BreakdownRow[]>("monthly_breakdown", { dateFrom, dateTo, dim, perMonth }),
  /** v0.18：导出覆盖度（已导出 / 从未导出 + 格式分布） */
  exportCoverage: (dateFrom?: string | null, dateTo?: string | null) =>
    invoke<ExportCoverage>("export_coverage", { dateFrom, dateTo }),
  /** v0.18：存储占用（withEnhancedBytes 开启才逐文件 stat，较慢） */
  storageUsage: (withEnhancedBytes?: boolean) =>
    invoke<StorageUsage>("storage_usage", { withEnhancedBytes }),

  /** v0.11：会计期间锁（period = YYYY-MM；锁定月凭证禁止增删改） */
  listPeriodLocks: () => invoke<PeriodLock[]>("list_period_locks"),
  lockPeriod: (period: string) => invoke<void>("lock_period", { period }),
  unlockPeriod: (period: string) => invoke<void>("unlock_period", { period }),

  /** v0.11：年度汇总（该自然年 12 个月，空月补零） */
  yearlySummary: (year: number) => invoke<MonthlySummary[]>("yearly_summary", { year }),

  /** v0.7 R-1：Top 对方账户（months=0 全部历史；limit 默认 10）。
      2026-10-16 起可传 `dateFrom`/`dateTo` 日期闭区间（洞察页时间窗）——给了区间就以区间为准 */
  topCounterparties: (months?: number, limit?: number, dateFrom?: string | null, dateTo?: string | null) =>
    invoke<CounterpartyRow[]>("top_counterparties", { months, limit, dateFrom, dateTo }),
  tagStats: (months?: number, limit?: number, dateFrom?: string | null, dateTo?: string | null) =>
    invoke<TagRow[]>("tag_stats", { months, limit, dateFrom, dateTo }),

  getRecoveryReport: () => invoke<RecoveryReport>("get_recovery_report"),

  searchVouchers: (keyword: string, filters?: SearchFilters, limit?: number) =>
    invoke<SearchHit[]>("search_vouchers", { keyword, filters: filters ?? null, limit }),

  /** Rust 侧对话框选图并直接导入（绕开 JS 对话框桥的竞态）。null = 用户取消 */
  pickAndImport: (runOcr: boolean) =>
    invoke<ImportSummary | null>("pick_and_import", { runOcr }),
  /** Rust 侧对话框选图更换凭证图片（换图不换卡）。null = 用户取消 */
  pickAndReplaceImage: (voucherId: number) =>
    invoke<VoucherDetail | null>("pick_and_replace_image", { voucherId }),
  /** 附件绝对路径（打开原图用） */
  attachmentAbsPath: (id: number) => invoke<string>("attachment_abs_path", { id }),
  /** v0.12：手动裁剪/旋转（旋转→裁剪→覆盖写增强版；原图不动）。rect 为旋转后图像像素坐标 */
  cropAttachment: (
    attachmentId: number,
    rect: { x: number; y: number; w: number; h: number },
    rotationDeg: number,
  ) => invoke<Attachment>("crop_attachment", { attachmentId, rect, rotationDeg }),
  /** v0.13：选账单 CSV → 对账（只读匹配；报告自动落 exports/）。null = 用户取消 */
  pickAndReconcile: () =>
    invoke<ReconcileReport | null>("pick_and_reconcile"),
  /** 识别附件并自动填单（只填空字段）；附件可能被语义化重命名，返回整份凭证详情供刷新 */
  runOcr: (attachmentId: number) => invoke<VoucherDetail>("run_ocr", { attachmentId }),
  /** v0.7 S-2：OCR 字段提取预览（只读） */
  ocrPreviewFields: (attachmentId: number) =>
    invoke<ExtractedFields>("ocr_preview_fields", { attachmentId }),
  runOcrPending: (limit?: number) => invoke<number>("run_ocr_pending", { limit }),
  getLocalOcrStatus: () => invoke<LocalOcrStatus>("local_ocr_status"),

  exportPdf: (voucherIds: number[], options?: ExportOptions) =>
    invoke<ExportBatch>("export_pdf", { voucherIds, options }),
  /** v0.10：表格式导出（Excel / CSV；列：凭证号/日期/摘要/对方/金额/状态/标签/备注 + 合计行） */
  exportTable: (voucherIds: number[], format: TableFormat) =>
    invoke<ExportBatch>("export_table", { voucherIds, format }),
  /** v0.9 导出历史反查：批次包含的凭证（按导出时排序；已删除的跳过） */
  exportBatchVouchers: (batchId: number) =>
    invoke<Voucher[]>("export_batch_vouchers", { batchId }),
  /** v0.8：按天日报 PDF（每天一个批次） */
  exportPdfDaily: (voucherIds: number[], options?: ExportOptions) =>
    invoke<ExportBatch[]>("export_pdf_daily", { voucherIds, options }),
  listExportBatches: (limit?: number) => invoke<ExportBatch[]>("list_export_batches", { limit }),

  runBackup: () => invoke<BackupInfo>("run_backup"),
  skipClosingBackup: () => invoke<void>("skip_closing_backup"),
  /** v0.8：批量重新增强存量附件（autocrop 修复后重算） */
  reEnhanceAll: () => invoke<{ ok: number; failed: number }>("re_enhance_all"),
  listBackups: () => invoke<BackupInfo[]>("list_backups"),
  inspectBackup: (path: string) => invoke<RestoreSummary>("inspect_backup", { path }),
  /** v0.9 导出完整备份：快照 + 附件包合成单个全量 zip 落入 backups/（供拷走），返回路径 */
  exportFullBackup: (path: string) => invoke<string>("export_full_backup", { path }),
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
  /**
   * v0.22：新建「说明条目」（**无图凭证**：说明 / 备忘 / 原始凭证分割单）。
   * 导入路径刻意不留空卡，所以"没有图片的凭证"只能由用户从这里显式创建。
   */
  createMemoVoucher: (newVoucher: NewVoucherInput) =>
    invoke<Voucher>("create_memo_voucher", { newVoucher }),

  // ---- v0.22 凭证关联 / 事项组（都不改金额合计口径）----
  linkVouchers: (aId: number, bId: number, kind: VoucherLinkKind, note?: string) =>
    invoke<VoucherLink>("link_vouchers", { aId, bId, kind, note }),
  unlinkVouchers: (aId: number, bId: number, kind: VoucherLinkKind) =>
    invoke<boolean>("unlink_vouchers", { aId, bId, kind }),
  listVoucherLinks: (voucherId: number) =>
    invoke<LinkedVoucher[]>("list_voucher_links", { voucherId }),
  listVoucherLinkEvents: (voucherId: number) =>
    invoke<VoucherLinkEvent[]>("list_voucher_link_events", { voucherId }),
  createVoucherGroup: (title: string, note?: string) =>
    invoke<VoucherGroup>("create_voucher_group", { title, note }),
  addVouchersToGroup: (groupId: number, voucherIds: number[]) =>
    invoke<VoucherGroup>("add_vouchers_to_group", { groupId, voucherIds }),
  removeVoucherFromGroup: (groupId: number, voucherId: number) =>
    invoke<VoucherGroup>("remove_voucher_from_group", { groupId, voucherId }),
  voucherGroupsOf: (voucherId: number) =>
    invoke<VoucherGroup[]>("voucher_groups_of", { voucherId }),
  listVoucherGroups: () => invoke<VoucherGroup[]>("list_voucher_groups"),
  deleteVoucherGroup: (groupId: number) => invoke<boolean>("delete_voucher_group", { groupId }),
};
