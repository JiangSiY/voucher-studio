/**
 * WPS 时间归一（S0-08 / 签审决策表 §7）。
 *
 * 冷却截止时间在 Rust 侧存在两种等价写法：库内与行内错误用
 * `to_rfc3339_opts(SecondsFormat::Secs, true)` 输出 `…Z`，而 preflight /
 * initialize 拒绝文案用 `to_rfc3339()` 输出 `…+00:00`（可能带纳秒）。
 * 前端若原样插值，同一时刻会显示成两种字符串，C4 的人工比对会误判为不一致。
 * 本模块只做显示归一（不改 Rust 文案、不推算 30 分钟冷却时长）：
 * 带显式时区的时间串一律折算成 `YYYY-MM-DDTHH:mm:ssZ`；解析失败或缺少时区时原样返回。
 */

const OFFSET_TS_SRC = String.raw`\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})`;

function toUtcSecs(match: string): string {
  const t = Date.parse(match);
  if (!Number.isFinite(t)) return match;
  return new Date(t).toISOString().replace(/\.\d{3}Z$/, "Z");
}

/** 单个时间串归一；空值显示「—」，无时区或解析失败时原样返回。 */
export function fmtUtcSecs(raw: string | null | undefined): string {
  if (!raw) return "—";
  const trimmed = raw.trim();
  if (!new RegExp(`^${OFFSET_TS_SRC}$`).test(trimmed)) return raw;
  return toUtcSecs(trimmed);
}

/** 文案内嵌的每个时间串逐个归一（用于行内错误与提示条）。 */
export function fmtWpsMessage(raw: string | null | undefined): string {
  if (!raw) return "";
  return raw.replace(new RegExp(OFFSET_TS_SRC, "g"), (m) => toUtcSecs(m));
}
