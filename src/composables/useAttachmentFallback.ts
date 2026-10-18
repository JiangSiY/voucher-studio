/**
 * 附件图片的候选路径与回退阶梯。
 *
 * 背景一：增强版是「可重建缓存」、不入备份（`backup/local.rs`），恢复快照后它的文件可能不在磁盘上。
 * 后端已在下发界面时剥掉这类路径（`services::attachment_view`），这里提供**第二道保险**：
 * 万一仍拿到坏路径（原图缺失、缓存陈旧、后端口径回归），界面按阶梯回退——
 * 增强版 → 原图 → 占位（`null`），而不是留一块无法解释的灰白。
 *
 * 背景二（2026-09-23 缺陷）：候选路径一律用**后端下发的绝对路径**
 * （`fileAbs` / `enhancedAbs`，Rust 侧按平台原生分隔符拼好）。此前前端是拿相对路径
 * 自行拼附件目录、并把 `/` 替换成 Windows 的反斜杠——在 macOS 上拼出的是并不存在的
 * 路径 → asset protocol 逐条拒绝 → 缩略图/预览全站空白。
 * **前端不要再把相对路径拼成绝对路径**，缺字段时按占位处理。
 */

export interface AttachmentLike {
  /** 原图绝对路径（Rust 侧拼好） */
  fileAbs?: string | null;
  /** 增强版绝对路径（可缺失，缺失时应回退原图） */
  enhancedAbs?: string | null;
}

/** 该附件的候选**绝对路径**（按偏好排序、去重、剔除空值） */
export function candidatePaths(a: AttachmentLike, forceOriginal = false): string[] {
  const out: string[] = [];
  const enhanced = (a.enhancedAbs ?? "").trim();
  const original = (a.fileAbs ?? "").trim();
  if (!forceOriginal && enhanced !== "") {
    out.push(enhanced);
  }
  if (original !== "" && !out.includes(original)) {
    out.push(original);
  }
  return out;
}

/** 取第 step 个候选；用尽返回 null（调用方显示占位） */
export function pickCandidate(
  a: AttachmentLike,
  step: number,
  forceOriginal = false,
): string | null {
  const list = candidatePaths(a, forceOriginal);
  if (step < 0 || step >= list.length) return null;
  return list[step];
}

/** 加载失败后的下一步；返回 `candidates.length` 表示已用尽 */
export function nextFallbackStep(current: number, total: number): number {
  return current < total ? current + 1 : total;
}
