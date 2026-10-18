/**
 * 视觉回归夹具：模拟 Tauri 后端（Rust commands）的返回值。
 *
 * 为什么需要它：前端在纯浏览器下 `isTauri === false`，所有 api 调用都会 reject；
 * 截图前由 Playwright 的 addInitScript 注入 `window.__TAURI_INTERNALS__`，
 * 让 `src/api/client.ts` 走真实的 invoke 分支，再用本文件的假数据渲染出**有内容**的界面。
 *
 * 约定：
 * - 数据全部是**确定性**的（无随机、无当前时间），截图才能与基线逐像素对比；
 * - 命令名与 `src/api/client.ts` 中的 `invoke("...")` 一一对应；
 * - 未知命令一律返回 null（调用方若需要数组，已在此显式覆盖）。
 */

/** 截图时冻结的“现在”。与证据目录日期一致，保证月度区间等文案稳定。 */
export const SHOT_NOW = "2026-10-18T10:00:00+08:00";

export const APP_VERSION = "0.22.2";

const DATA_DIR = "/Users/apple/Library/Application Support/com.voucherstudio.app";

const APP_INFO = {
  version: APP_VERSION,
  dataDir: DATA_DIR,
  dbPath: `${DATA_DIR}/voucher.db`,
  attachmentsDir: `${DATA_DIR}/attachments`,
  backupsDir: `${DATA_DIR}/backups`,
  exportsDir: `${DATA_DIR}/exports`,
  logsDir: `${DATA_DIR}/logs`,
};

/** 与 crates/voucher-core/src/utils/config.rs 的 AppConfig::default() 对齐（深色由 colorScheme 模拟系统） */
const APP_CONFIG = {
  appearance: { theme: "system", sidebarWidth: 220, uiScale: 1.15, sidebarCollapsed: false },
  naming: { template: "{date-}-{n}" },
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
    title: "原始凭证",
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
  voucher: {
    descriptionTemplates: ["办公用品采购（{item}）", "差旅住宿费（{city}）", "客户招待晚餐"],
  },
};

/** 12 张凭证：金额、日期、状态、收支方向齐全，覆盖列表/编辑器/打印/洞察。 */
const VOUCHERS = [
  {
    id: 101,
    voucherNo: "2026-10-16-01",
    voucherDate: "2026-10-16",
    description: "办公用品采购（A4 纸、签字笔）",
    counterparty: "文一办公用品店",
    amountCents: 48650,
    status: "reviewed",
    tags: ["支出", "办公"],
    note: "季度补货：A4 纸 5 包、签字笔 2 盒。",
    createdAt: "2026-10-16T10:24:00+08:00",
    updatedAt: "2026-10-16T18:20:00+08:00",
    source: "scan",
  },
  {
    id: 102,
    voucherNo: "2026-10-15-02",
    voucherDate: "2026-10-15",
    description: "客户招待晚餐",
    counterparty: "海底捞（国贸店）",
    amountCents: 128000,
    status: "exported",
    tags: ["支出", "招待"],
    note: "云启科技三位同事，项目二期启动沟通。",
    createdAt: "2026-10-15T21:10:00+08:00",
    updatedAt: "2026-10-16T09:02:00+08:00",
    source: "screenshot",
  },
  {
    id: 103,
    voucherNo: "2026-10-14-01",
    voucherDate: "2026-10-14",
    description: "差旅住宿费（上海）",
    counterparty: "全季酒店上海虹桥店",
    amountCents: 89900,
    status: "reviewed",
    tags: ["支出", "差旅"],
    note: "10 月 13–14 日，两晚含早。",
    createdAt: "2026-10-14T22:36:00+08:00",
    updatedAt: "2026-10-15T08:41:00+08:00",
    source: "scan",
  },
  {
    id: 104,
    voucherNo: "2026-10-12-03",
    voucherDate: "2026-10-12",
    description: "软件订阅 · Figma 团队版",
    counterparty: "Figma Inc.",
    amountCents: 15800,
    status: "archived",
    tags: ["支出", "软件"],
    note: "年度订阅，按美元结算。",
    createdAt: "2026-10-12T09:18:00+08:00",
    updatedAt: "2026-10-13T11:05:00+08:00",
    source: "screenshot",
  },
  {
    id: 105,
    voucherNo: "2026-10-11-01",
    voucherDate: "2026-10-11",
    description: "设计外包尾款",
    counterparty: "杭州像素工坊",
    amountCents: 350000,
    status: "draft",
    tags: ["支出", "外包"],
    note: "验收通过后支付尾款，待补发票号。",
    createdAt: "2026-10-11T16:02:00+08:00",
    updatedAt: "2026-10-18T09:12:00+08:00",
    source: "scan",
  },
  {
    id: 106,
    voucherNo: "2026-10-09-02",
    voucherDate: "2026-10-09",
    description: "客户回款 · 项目二期",
    counterparty: "上海云启科技",
    amountCents: 1280000,
    status: "exported",
    tags: ["收入", "回款"],
    note: "合同 YQ-2026-018，第二期 40%。",
    createdAt: "2026-10-09T14:30:00+08:00",
    updatedAt: "2026-10-09T15:00:00+08:00",
    source: "screenshot",
  },
  {
    id: 107,
    voucherNo: "2026-10-08-01",
    voucherDate: "2026-10-08",
    description: "快递费（顺丰）",
    counterparty: "顺丰速运",
    amountCents: 3600,
    status: "reviewed",
    tags: ["支出", "物流"],
    note: "合同寄送北京。",
    createdAt: "2026-10-08T17:44:00+08:00",
    updatedAt: "2026-10-08T17:44:00+08:00",
    source: "scan",
  },
  {
    id: 108,
    voucherNo: "2026-10-07-02",
    voucherDate: "2026-10-07",
    description: "团队午餐",
    counterparty: "老乡鸡（望京店）",
    amountCents: 24600,
    status: "reviewed",
    tags: ["支出", "餐饮"],
    note: "四人工作餐。",
    createdAt: "2026-10-07T12:51:00+08:00",
    updatedAt: "2026-10-07T13:10:00+08:00",
    source: "screenshot",
  },
  {
    id: 109,
    voucherNo: "2026-09-30-03",
    voucherDate: "2026-09-30",
    description: "云服务器续费",
    counterparty: "阿里云",
    amountCents: 178800,
    status: "exported",
    tags: ["支出", "云服务"],
    note: "华东 2 区，包年。",
    createdAt: "2026-09-30T10:05:00+08:00",
    updatedAt: "2026-09-30T10:05:00+08:00",
    source: "scan",
  },
  {
    id: 110,
    voucherNo: "2026-09-28-01",
    voucherDate: "2026-09-28",
    description: "咨询服务收入",
    counterparty: "北京青禾咨询",
    amountCents: 600000,
    status: "archived",
    tags: ["收入", "咨询"],
    note: "9 月驻场咨询 5 天。",
    createdAt: "2026-09-28T18:22:00+08:00",
    updatedAt: "2026-09-29T09:30:00+08:00",
    source: "screenshot",
  },
  {
    id: 111,
    voucherNo: "2026-09-25-02",
    voucherDate: "2026-09-25",
    description: "打印耗材",
    counterparty: "京东商城",
    amountCents: 12990,
    status: "draft",
    tags: ["支出", "办公"],
    note: "墨盒 + 相纸，电子发票待归档。",
    createdAt: "2026-09-25T13:40:00+08:00",
    updatedAt: "2026-10-02T10:11:00+08:00",
    source: "scan",
  },
  {
    id: 112,
    voucherNo: "2026-09-22-01",
    voucherDate: "2026-09-22",
    description: "办公室绿植租赁",
    counterparty: "花点时间",
    amountCents: 9800,
    status: "reviewed",
    tags: ["支出", "行政"],
    note: "三季度绿植养护。",
    createdAt: "2026-09-22T11:20:00+08:00",
    updatedAt: "2026-09-22T11:20:00+08:00",
    source: "scan",
  },
];

// ---------- 收据图片（SVG data URL） ----------
// 截图里的缩略图/附件必须“像真的”，否则看不出布局是否正常。
// 用确定性 SVG 生成，避免依赖外部文件或网络。

const ESC = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ESC[c]);

function receiptSvg({ no, date, party, desc, amountCents, accent, enhanced }) {
  const amount = (amountCents / 100).toFixed(2);
  const bg = enhanced ? "#f8f8f5" : "#f4efe2";
  const ink = "#2a2620";
  const sub = "#6f675b";
  const bars = Array.from({ length: 34 }, (_, i) => {
    const w = 2 + ((i * 7 + no.length) % 5);
    const x = 74 + i * 14;
    return `<rect x="${x}" y="812" width="${w}" height="72" fill="${ink}"/>`;
  }).join("");
  const lines = ["办公用品", "差旅住宿", "餐饮服务", "软件服务", "设计服务", "物流服务"];
  const item1 = lines[no.length % lines.length];
  const item2 = lines[(no.length + 3) % lines.length];
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="720" height="960" viewBox="0 0 720 960">
  <rect width="720" height="960" fill="${bg}"/>
  <rect x="0" y="0" width="720" height="118" fill="${accent}"/>
  <text x="48" y="62" font-family="PingFang SC, Helvetica, sans-serif" font-size="30" font-weight="700" fill="#ffffff">${esc(party)}</text>
  <text x="48" y="96" font-family="PingFang SC, Helvetica, sans-serif" font-size="17" fill="#ffffffcc">电子发票 · 收据样例（假数据）</text>
  <text x="660" y="68" text-anchor="end" font-family="Helvetica" font-size="15" fill="#ffffffcc">${enhanced ? "SCAN · ENHANCED" : "ORIGINAL"}</text>
  <text x="48" y="176" font-family="Helvetica" font-size="15" fill="${sub}">NO. ${esc(no)}</text>
  <text x="672" y="176" text-anchor="end" font-family="Helvetica" font-size="15" fill="${sub}">DATE ${esc(date)}</text>
  <line x1="48" y1="204" x2="672" y2="204" stroke="#c9c0ad" stroke-width="2" stroke-dasharray="8 8"/>
  <text x="48" y="258" font-family="PingFang SC, Helvetica, sans-serif" font-size="22" font-weight="600" fill="${ink}">${esc(desc)}</text>
  <text x="48" y="306" font-family="Helvetica" font-size="16" fill="${sub}">ITEM</text>
  <text x="470" y="306" font-family="Helvetica" font-size="16" fill="${sub}">QTY</text>
  <text x="672" y="306" text-anchor="end" font-family="Helvetica" font-size="16" fill="${sub}">AMOUNT</text>
  <text x="48" y="352" font-family="PingFang SC, Helvetica, sans-serif" font-size="20" fill="${ink}">${item1}</text>
  <text x="482" y="352" font-family="Helvetica" font-size="18" fill="${ink}">2</text>
  <text x="672" y="352" text-anchor="end" font-family="Helvetica" font-size="18" fill="${ink}">${((amountCents * 0.42) / 100).toFixed(2)}</text>
  <text x="48" y="398" font-family="PingFang SC, Helvetica, sans-serif" font-size="20" fill="${ink}">${item2}</text>
  <text x="482" y="398" font-family="Helvetica" font-size="18" fill="${ink}">1</text>
  <text x="672" y="398" text-anchor="end" font-family="Helvetica" font-size="18" fill="${ink}">${((amountCents * 0.58) / 100).toFixed(2)}</text>
  <line x1="48" y1="444" x2="672" y2="444" stroke="#c9c0ad" stroke-width="2"/>
  <text x="48" y="500" font-family="PingFang SC, Helvetica, sans-serif" font-size="22" font-weight="700" fill="${ink}">合计</text>
  <text x="672" y="500" text-anchor="end" font-family="PingFang SC, Helvetica, sans-serif" font-size="40" font-weight="700" fill="${ink}">¥ ${amount}</text>
  <text x="48" y="552" font-family="PingFang SC, Helvetica, sans-serif" font-size="16" fill="${sub}">支付方式：对公转账</text>
  <text x="48" y="584" font-family="PingFang SC, Helvetica, sans-serif" font-size="16" fill="${sub}">备注：此图由视觉回归夹具生成，非真实凭证</text>
  <rect x="48" y="640" width="624" height="140" rx="10" fill="#ffffff88" stroke="#d9d0bd"/>
  <text x="72" y="690" font-family="PingFang SC, Helvetica, sans-serif" font-size="18" fill="${sub}">开票方</text>
  <text x="72" y="726" font-family="PingFang SC, Helvetica, sans-serif" font-size="20" fill="${ink}">${esc(party)}</text>
  <text x="72" y="758" font-family="Helvetica" font-size="14" fill="${sub}">TAX ID 9133 0108 MA2X **** 7Q</text>
  ${bars.replace(/height="72"/g, 'height="56"')}
  <text x="672" y="902" text-anchor="end" font-family="Helvetica" font-size="13" fill="${sub}">VOUCHER STUDIO · MOCK</text>
</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

const ACCENTS = ["#8a5a44", "#3f6b5a", "#4a5d82", "#8b6a2f", "#6d4f78", "#41636e"];

const ATTACHMENTS = [];
const IMAGES = {};
const EXTRACTED = {};

VOUCHERS.forEach((v, idx) => {
  const fileRel = `attachments/${v.voucherNo}.png`;
  const fileAbs = `/mock/${fileRel}`;
  const enhancedRel = v.source === "scan" ? `attachments/${v.voucherNo}-enhanced.png` : null;
  const enhancedAbs = enhancedRel ? `/mock/${enhancedRel}` : null;
  IMAGES[fileAbs] = receiptSvg({
    no: v.voucherNo,
    date: v.voucherDate,
    party: v.counterparty,
    desc: v.description,
    amountCents: v.amountCents,
    accent: ACCENTS[idx % ACCENTS.length],
    enhanced: false,
  });
  if (enhancedAbs) {
    IMAGES[enhancedAbs] = receiptSvg({
      no: v.voucherNo,
      date: v.voucherDate,
      party: v.counterparty,
      desc: v.description,
      amountCents: v.amountCents,
      accent: "#5c5c5c",
      enhanced: true,
    });
  }
  const attachmentId = v.id * 10 + 1;
  ATTACHMENTS.push({
    id: attachmentId,
    voucherId: v.id,
    fileName: `${v.voucherNo}.png`,
    filePath: fileRel,
    fileHash: `sha256:${String(v.id).repeat(8)}${"ab12".repeat(12)}`.slice(0, 71),
    fileSize: 420000 + idx * 13711,
    mimeType: "image/png",
    ocrStatus: "done",
    ocrText: `${v.counterparty} ${v.voucherDate} 合计 ¥${(v.amountCents / 100).toFixed(2)}`,
    enhancedPath: enhancedRel,
    enhancedSource: v.source,
    fileAbs,
    enhancedAbs,
    createdAt: v.createdAt,
  });
  EXTRACTED[attachmentId] = {
    date: v.voucherDate,
    amountCents: v.amountCents,
    counterparty: v.counterparty,
    merchant: v.counterparty,
    amountSource: "labeled",
    confidence: { date: "high", amount: "high", counterparty: "high", merchant: "medium" },
  };
});

const THUMBS = {};
for (const v of VOUCHERS) {
  const a = ATTACHMENTS.find((x) => x.voucherId === v.id);
  THUMBS[v.id] = {
    filePath: a.filePath,
    enhancedPath: a.enhancedPath,
    enhancedSource: a.enhancedSource,
    fileAbs: a.fileAbs,
    enhancedAbs: a.enhancedAbs,
  };
}

// ---------- 洞察：月度 / 年度 / 透视 / 覆盖度 / 存储 ----------

const MONTHLY = [
  {
    month: "2026-05",
    count: 9,
    amountCents: 812300,
    draft: 1,
    reviewed: 5,
    exported: 2,
    archived: 1,
  },
  {
    month: "2026-06",
    count: 11,
    amountCents: 1204800,
    draft: 0,
    reviewed: 6,
    exported: 3,
    archived: 2,
  },
  {
    month: "2026-07",
    count: 8,
    amountCents: 956600,
    draft: 2,
    reviewed: 3,
    exported: 2,
    archived: 1,
  },
  {
    month: "2026-08",
    count: 13,
    amountCents: 1542000,
    draft: 1,
    reviewed: 7,
    exported: 3,
    archived: 2,
  },
  {
    month: "2026-09",
    count: 4,
    amountCents: 801590,
    draft: 1,
    reviewed: 1,
    exported: 1,
    archived: 1,
  },
  {
    month: "2026-10",
    count: 8,
    amountCents: 1940550,
    draft: 1,
    reviewed: 4,
    exported: 2,
    archived: 1,
  },
];

const YEARLY = [
  {
    month: "2026-01",
    count: 10,
    amountCents: 986300,
    draft: 0,
    reviewed: 5,
    exported: 3,
    archived: 2,
  },
  {
    month: "2026-02",
    count: 6,
    amountCents: 512400,
    draft: 1,
    reviewed: 2,
    exported: 2,
    archived: 1,
  },
  {
    month: "2026-03",
    count: 12,
    amountCents: 1320800,
    draft: 1,
    reviewed: 6,
    exported: 3,
    archived: 2,
  },
  {
    month: "2026-04",
    count: 9,
    amountCents: 1042700,
    draft: 0,
    reviewed: 4,
    exported: 3,
    archived: 2,
  },
  ...MONTHLY,
  { month: "2026-11", count: 0, amountCents: 0, draft: 0, reviewed: 0, exported: 0, archived: 0 },
  { month: "2026-12", count: 0, amountCents: 0, draft: 0, reviewed: 0, exported: 0, archived: 0 },
];

const BREAKDOWN = [
  { month: "2026-10", key: "上海云启科技", count: 1, amountCents: 1280000 },
  { month: "2026-10", key: "杭州像素工坊", count: 1, amountCents: 350000 },
  { month: "2026-10", key: "海底捞（国贸店）", count: 1, amountCents: 128000 },
  { month: "2026-10", key: "全季酒店上海虹桥店", count: 1, amountCents: 89900 },
  { month: "2026-10", key: "文一办公用品店", count: 1, amountCents: 48650 },
  { month: "2026-09", key: "北京青禾咨询", count: 1, amountCents: 600000 },
  { month: "2026-09", key: "阿里云", count: 1, amountCents: 178800 },
  { month: "2026-09", key: "京东商城", count: 1, amountCents: 12990 },
  { month: "2026-09", key: "花点时间", count: 1, amountCents: 9800 },
  { month: "2026-08", key: "上海云启科技", count: 2, amountCents: 720000 },
  { month: "2026-08", key: "北京青禾咨询", count: 1, amountCents: 380000 },
  { month: "2026-08", key: "杭州像素工坊", count: 1, amountCents: 260000 },
  { month: "2026-08", key: "阿里云", count: 1, amountCents: 182000 },
  { month: "2026-07", key: "上海云启科技", count: 1, amountCents: 560000 },
  { month: "2026-07", key: "全季酒店上海虹桥店", count: 2, amountCents: 168000 },
  { month: "2026-07", key: "文一办公用品店", count: 1, amountCents: 62800 },
  { month: "2026-06", key: "北京青禾咨询", count: 2, amountCents: 680000 },
  { month: "2026-06", key: "阿里云", count: 1, amountCents: 176800 },
  { month: "2026-06", key: "海底捞（国贸店）", count: 1, amountCents: 132000 },
  { month: "2026-05", key: "上海云启科技", count: 1, amountCents: 480000 },
  { month: "2026-05", key: "京东商城", count: 2, amountCents: 88400 },
  { month: "2026-05", key: "顺丰速运", count: 3, amountCents: 12900 },
];

const COVERAGE = {
  total: 12,
  exported: 5,
  neverExported: 7,
  byFormat: [
    { format: "PDF", batches: 4, vouchers: 7 },
    { format: "Excel", batches: 2, vouchers: 3 },
    { format: "CSV", batches: 1, vouchers: 2 },
  ],
};

const STORAGE = {
  originalCount: 12,
  originalBytes: 8734208,
  enhancedCount: 9,
  enhancedScreenshot: 3,
  enhancedScan: 9,
  enhancedUnknown: 0,
  enhancedBytes: null,
  ocrCacheCount: 12,
};

const COUNTERPARTIES = [
  { name: "上海云启科技", count: 2, amountCents: 1760000 },
  { name: "北京青禾咨询", count: 2, amountCents: 1280000 },
  { name: "杭州像素工坊", count: 1, amountCents: 350000 },
  { name: "阿里云", count: 2, amountCents: 355600 },
  { name: "全季酒店上海虹桥店", count: 2, amountCents: 258000 },
  { name: "海底捞（国贸店）", count: 1, amountCents: 128000 },
  { name: "文一办公用品店", count: 1, amountCents: 48650 },
  { name: "京东商城", count: 1, amountCents: 12990 },
];

const TAGS = [
  { tag: "支出", count: 9, amountCents: 2816500 },
  { tag: "收入", count: 3, amountCents: 1880000 },
  { tag: "办公", count: 2, amountCents: 61640 },
  { tag: "差旅", count: 1, amountCents: 89900 },
  { tag: "外包", count: 1, amountCents: 350000 },
];

// ---------- 设置页 / 全局状态 ----------

const RECENT_ENHANCED = [
  {
    attachmentId: 1051,
    voucherId: 105,
    voucherNo: "2026-10-11-01",
    fileName: "2026-10-11-01.png",
    fileSize: 468443,
    enhancedSize: 312880,
    enhancedSource: "scan",
    enhancedAt: "2026-10-18T09:12:00+08:00",
    hitCount: 6,
  },
  {
    attachmentId: 1011,
    voucherId: 101,
    voucherNo: "2026-10-16-01",
    fileName: "2026-10-16-01.png",
    fileSize: 481154,
    enhancedSize: 304219,
    enhancedSource: "scan",
    enhancedAt: "2026-10-16T18:20:00+08:00",
    hitCount: 11,
  },
  {
    attachmentId: 1031,
    voucherId: 103,
    voucherNo: "2026-10-14-01",
    fileName: "2026-10-14-01.png",
    fileSize: 494865,
    enhancedSize: 297442,
    enhancedSource: "scan",
    enhancedAt: "2026-10-14T22:36:00+08:00",
    hitCount: 4,
  },
  {
    attachmentId: 1091,
    voucherId: 109,
    voucherNo: "2026-09-30-03",
    fileName: "2026-09-30-03.png",
    fileSize: 508576,
    enhancedSize: 320117,
    enhancedSource: "scan",
    enhancedAt: "2026-09-30T10:05:00+08:00",
    hitCount: 9,
  },
  {
    attachmentId: 1071,
    voucherId: 107,
    voucherNo: "2026-10-08-01",
    fileName: "2026-10-08-01.png",
    fileSize: 433332,
    enhancedSize: 281004,
    enhancedSource: "scan",
    enhancedAt: "2026-10-08T17:44:00+08:00",
    hitCount: 2,
  },
];

const HIT_POINTS = [
  [5, 3, 8, 2],
  [6, 5, 9, 3],
  [7, 4, 6, 1],
  [8, 6, 11, 4],
  [9, 3, 7, 2],
  [10, 7, 12, 5],
  [11, 5, 10, 3],
  [12, 8, 14, 4],
  [13, 6, 9, 2],
  [14, 9, 15, 5],
  [15, 4, 8, 3],
  [16, 10, 16, 6],
  [17, 7, 11, 4],
  [18, 8, 13, 5],
].map(([d, ocr, exp, viewer]) => ({
  day: `2026-10-${String(d).padStart(2, "0")}`,
  ocr,
  export: exp,
  viewer,
  total: ocr + exp + viewer,
}));
const HIT_TREND = {
  days: 14,
  points: HIT_POINTS,
  maxTotal: Math.max(...HIT_POINTS.map((p) => p.total)),
};

const LOCAL_OCR = {
  installed: true,
  modelsDir: `${DATA_DIR}/ocr-models`,
  files: [
    { name: "ch_PP-OCRv4_det_infer.onnx", size: 4823449 },
    { name: "ch_PP-OCRv4_rec_infer.onnx", size: 10538226 },
    { name: "ppocr_keys_v1.txt", size: 26812 },
  ],
  totalSize: 15388487,
};

const SECRETS = [
  { alias: "deepseek_api_key", label: "DeepSeek API Key", exists: true },
  { alias: "webdav_password", label: "WebDAV 密码", exists: false },
  { alias: "ocr_cloud_token", label: "云端 OCR Token", exists: true },
];

const ENHANCE_V2 = {
  total: 12,
  totalSize: 8734208,
  screenshotCount: 3,
  scanCount: 9,
  hitCount: 128,
  hitCount30d: 64,
  hitCountOcr: 71,
  hitCountExport: 33,
  hitCountViewer: 24,
  oldestEnhancedAt: "2026-09-02T09:10:00+08:00",
  newestEnhancedAt: "2026-10-18T09:12:00+08:00",
};

const PERIOD_LOCKS = [{ period: "2026-09", lockedAt: "2026-10-02T09:12:00+08:00" }];

const TODOS = {
  ocrPending: 2,
  amountMissing: 1,
  staleDrafts: 1,
  staleDraftDays: 7,
  unlockedMonths: 2,
  latestBackup: {
    fileName: "backup-2026-10-17.zip",
    path: `${DATA_DIR}/backups/backup-2026-10-17.zip`,
    size: 7340032,
    createdAt: "2026-10-17T23:10:00+08:00",
    attachmentCount: 12,
    warnings: [],
  },
  backupStale: false,
  dupPairs: 3,
  dupHighPairs: 1,
  dupGroups: 2,
};

const RECOVERY = {
  abnormalExit: false,
  lastSession: null,
  dbOk: true,
  integrityErrors: [],
  attachmentTotal: 12,
  attachmentMissingCount: 0,
  attachmentMissing: [],
  attachmentMissingEnhanced: 0,
  lastBackup: TODOS.latestBackup,
};

const GROUPS = [
  {
    id: 1,
    title: "Q4 市场活动",
    note: "10 月市场相关支出与回款",
    createdAt: "2026-10-10T09:00:00+08:00",
    members: [VOUCHERS[0], VOUCHERS[1]],
  },
  {
    id: 2,
    title: "上海出差（10 月）",
    note: "住宿 + 交通",
    createdAt: "2026-10-15T09:00:00+08:00",
    members: [VOUCHERS[2]],
  },
];

/** 注入页面的完整夹具（addInitScript 的入参，必须可结构化克隆）。 */
export const MOCK = {
  appInfo: APP_INFO,
  config: APP_CONFIG,
  vouchers: VOUCHERS,
  attachments: ATTACHMENTS,
  thumbs: THUMBS,
  images: IMAGES,
  extracted: EXTRACTED,
  monthly: MONTHLY,
  yearly: YEARLY,
  breakdown: BREAKDOWN,
  coverage: COVERAGE,
  storage: STORAGE,
  counterparties: COUNTERPARTIES,
  tags: TAGS,
  recentEnhanced: RECENT_ENHANCED,
  hitTrend: HIT_TREND,
  localOcr: LOCAL_OCR,
  secrets: SECRETS,
  enhanceV2: ENHANCE_V2,
  periodLocks: PERIOD_LOCKS,
  todos: TODOS,
  recovery: RECOVERY,
  groups: GROUPS,
  dupPending: { groups: 2, pairs: 3 },
  dupCount: { groups: 2, pairs: 3, highPairs: 1 },
  health: { total: 12, noImage: 1, ocrPending: 2, unlinked: 4 },
};

/**
 * 页面加载前注入 `__TAURI_INTERNALS__`：invoke 分发 + 回调注册 + 资源地址转换。
 * 该函数会被 Playwright 序列化后执行，不能引用外部变量（夹具通过参数传入）。
 */
export function installTauriMock(mock) {
  const callbacks = new Map();
  let nextCallbackId = 1;
  let nextEventId = 1;

  const listVouchers = (args = {}) => {
    const q = args.query ?? args;
    let rows = mock.vouchers.slice();
    const kw = (q.keyword ?? "").trim().toLowerCase();
    if (kw) {
      rows = rows.filter((v) =>
        [v.voucherNo, v.voucherDate, v.description, v.counterparty, v.note]
          .join("\n")
          .toLowerCase()
          .includes(kw),
      );
    }
    const wanted = (q.statuses?.length ? q.statuses : q.status ? [q.status] : []).filter(Boolean);
    if (wanted.length) rows = rows.filter((v) => wanted.includes(v.status));
    if (q.dateFrom) rows = rows.filter((v) => v.voucherDate >= q.dateFrom);
    if (q.dateTo) rows = rows.filter((v) => v.voucherDate <= q.dateTo);
    if (q.directionNone)
      rows = rows.filter((v) => !v.tags.includes("收入") && !v.tags.includes("支出"));
    if (q.exported === "never")
      rows = rows.filter((v) => v.status !== "exported" && v.status !== "archived");
    rows.sort((a, b) =>
      a.voucherDate < b.voucherDate ? 1 : a.voucherDate > b.voucherDate ? -1 : b.id - a.id,
    );
    const page = Math.max(1, Number(q.page) || 1);
    const pageSize = Math.max(1, Number(q.pageSize) || 10);
    const items = rows.slice((page - 1) * pageSize, page * pageSize);
    const thumbs = {};
    for (const v of items) if (mock.thumbs[v.id]) thumbs[v.id] = mock.thumbs[v.id];
    return { items, total: rows.length, thumbs };
  };

  const handlers = {
    get_app_info: () => mock.appInfo,
    get_config: () => mock.config,
    save_config: (args) => args?.config ?? mock.config,
    get_recovery_report: () => mock.recovery,
    dashboard_todos: () => mock.todos,
    data_health: () => mock.health,
    duplicate_pending_summary: () => mock.dupPending,
    duplicate_pending_count: () => mock.dupCount,
    list_vouchers: (args) => listVouchers(args),
    list_voucher_ids: () => mock.vouchers.map((v) => v.id),
    get_voucher: (args) => {
      const id = Number(args?.id);
      const voucher = mock.vouchers.find((v) => v.id === id);
      if (!voucher) return null;
      return {
        voucher,
        attachments: mock.attachments.filter((a) => a.voucherId === id),
        snapshots: [],
      };
    },
    get_voucher_neighbor: (args) => {
      const id = Number(args?.id);
      const ordered = mock.vouchers
        .slice()
        .sort((a, b) => (a.voucherDate < b.voucherDate ? 1 : -1));
      const idx = ordered.findIndex((v) => v.id === id);
      if (idx < 0) return null;
      return {
        prevId: idx > 0 ? ordered[idx - 1].id : null,
        nextId: idx < ordered.length - 1 ? ordered[idx + 1].id : null,
        position: idx + 1,
        total: ordered.length,
      };
    },
    list_voucher_links: () => [],
    list_voucher_link_events: () => [],
    list_voucher_groups: () => mock.groups,
    voucher_groups_of: () => [],
    list_period_locks: () => mock.periodLocks,
    ocr_preview_fields: (args) =>
      mock.extracted[Number(args?.attachmentId)] ?? mock.extracted[Number(args?.id)] ?? null,
    check_voucher_duplicates: () => [],
    monthly_summary: () => mock.monthly,
    yearly_summary: () => mock.yearly,
    monthly_breakdown: () => mock.breakdown,
    export_coverage: () => mock.coverage,
    storage_usage: () => mock.storage,
    top_counterparties: () => mock.counterparties,
    tag_stats: () => mock.tags,
    enhance_status_v2: () => mock.enhanceV2,
    recent_enhanced: () => mock.recentEnhanced,
    enhance_hit_trend: () => mock.hitTrend,
    local_ocr_status: () => mock.localOcr,
    list_secrets: () => mock.secrets,
    list_export_batches: () => [],
    frequent_descriptions: () => [],
    validate_naming_template: (args) =>
      `模板可用：示例 ${args?.template === "{date-}-{n}" ? "2026-10-18-01" : "2026-10-18-001"}`,
    preview_voucher_no: () => "2026-10-18-01",
    test_webdav: () => 42,
    open_path: () => null,
    record_viewer_hit: () => null,
    "plugin:event|listen": () => nextEventId++,
    "plugin:event|unlisten": () => null,
  };

  window.__TAURI_INTERNALS__ = {
    transformCallback(cb, once = false) {
      const id = nextCallbackId++;
      callbacks.set(id, { cb, once });
      return id;
    },
    unregisterCallback(id) {
      callbacks.delete(id);
    },
    convertFileSrc(filePath) {
      return mock.images[filePath] ?? mock.images["/mock/attachments/placeholder.png"] ?? "";
    },
    async invoke(cmd, args) {
      const h = handlers[cmd];
      if (!h) return null;
      return h(args ?? {});
    },
    async metadata() {
      return null;
    },
  };
  // event 插件在 unlisten 时会用到；不定义会抛 TypeError 污染控制台
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener() {},
  };
  // 便于排查：看到这个标记说明 mock 注入成功
  window.__VS_VISUAL_MOCK__ = true;
}
