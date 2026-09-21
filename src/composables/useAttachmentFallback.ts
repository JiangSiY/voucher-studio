/**
 * 附件图片的候选路径与回退阶梯。
 *
 * 背景：增强版是「可重建缓存」、不入备份（`backup/local.rs`），恢复快照后它的文件可能不在磁盘上。
 * 后端已在下发界面时剥掉这类路径（`services::attachment_view`），这里提供**第二道保险**：
 * 万一仍拿到坏路径（原图缺失、缓存陈旧、后端口径回归），界面按阶梯回退——
 * 增强版 → 原图 → 占位（`null`），而不是留一块无法解释的灰白。
 */

export interface AttachmentLike {
  /** 原图相对路径（相对附件根目录，正斜杠） */
  filePath: string;
  /** 增强版相对路径（可缺失，缺失时应回退原图） */
  enhancedPath?: string | null;
}

/** 该附件的候选相对路径（按偏好排序、去重、剔除空值） */
export function candidatePaths(a: AttachmentLike, forceOriginal = false): string[] {
  const out: string[] = [];
  if (!forceOriginal && a.enhancedPath && a.enhancedPath.trim() !== "") {
    out.push(a.enhancedPath);
  }
  if (a.filePath && a.filePath.trim() !== "" && !out.includes(a.filePath)) {
    out.push(a.filePath);
  }
  return out;
}

/** 取第 step 个候选；用尽返回 null（调用方显示占位） */
export function pickCandidate(a: AttachmentLike, step: number, forceOriginal = false): string | null {
  const list = candidatePaths(a, forceOriginal);
  if (step < 0 || step >= list.length) return null;
  return list[step];
}

/** 加载失败后的下一步；返回 `candidates.length` 表示已用尽 */
export function nextFallbackStep(current: number, total: number): number {
  return current < total ? current + 1 : total;
}
