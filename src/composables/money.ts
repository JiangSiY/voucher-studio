/**
 * 金额输入 ↔ 分 的唯一换算口径。
 *
 * 为什么抽出来：金额是本应用最关键的字段（统计、重复判定、导出金额都依赖它），
 * 而"元字符串 → 分"的严格校验原先只写在编辑页里；检视器就地编辑需要同一套规则，
 * 复制一份必然分叉（本项目已有过"两处手写除法把 -50 分写成 0.-50"的教训）。
 *
 * 规则（与编辑页历史行为一致，勿放宽）：
 * - 空串 → `cents = null`（并配合 `clearAmount: true` 提交，表示显式清空）；
 * - 允许可选负号 + 最多两位小数（`123`、`123.4`、`-123.45`）；
 * - **拒绝** `12.34.56`、`1e3`、`1,234`、`１２３`（全角）等一切宽松解析。
 */

export const AMOUNT_FORMAT_HINT = "金额格式不正确：应为数字，最多两位小数（如 123.45）";

export type ParseAmountResult =
  | { ok: true; cents: number | null }
  | { ok: false; reason: string };

/** 元字符串 → 分（空串是合法输入，表示"不填金额"） */
export function parseYuanToCents(input: string): ParseAmountResult {
  const s = input.trim();
  if (s === "") return { ok: true, cents: null };
  if (!/^-?\d+(\.\d{1,2})?$/.test(s)) return { ok: false, reason: AMOUNT_FORMAT_HINT };
  // 用字符串拆分避免浮点误差（parseFloat("0.07") * 100 在某些值上会得到 6.999…）
  const neg = s.startsWith("-");
  const body = neg ? s.slice(1) : s;
  const [intPart, fracPart = ""] = body.split(".");
  const cents = Number(intPart) * 100 + Number(fracPart.padEnd(2, "0"));
  return { ok: true, cents: neg ? -cents : cents };
}

/** 分 → 元字符串（用于回填输入框；null → 空串） */
export function centsToYuanInput(cents: number | null | undefined): string {
  if (cents == null) return "";
  const neg = cents < 0;
  const abs = Math.abs(cents);
  const yuan = Math.floor(abs / 100);
  const fen = abs % 100;
  return `${neg ? "-" : ""}${yuan}${fen === 0 ? "" : "." + String(fen).padStart(2, "0")}`;
}

/** 标签输入 → 数组（中英文逗号、顿号都当分隔符；去空去重保序） */
export function parseTags(input: string): string[] {
  const out: string[] = [];
  for (const raw of input.split(/[,，、]/)) {
    const t = raw.trim();
    if (t && !out.includes(t)) out.push(t);
  }
  return out;
}
