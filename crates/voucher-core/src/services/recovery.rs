//! 崩溃恢复（对应 v4.3 的 recovery_window）
//!
//! 会话标记：启动时写 marker 文件（`.last-session`），正常关闭（窗口关闭事件）时删除。
//! 下次启动若 marker 仍在 → 上次异常退出（崩溃 / 强杀 / 断电）→ 触发数据健康检查。
//!
//! 健康检查（只读，绝不自动改动数据）：
//! - SQLite `PRAGMA integrity_check`（WAL 崩溃恢复后的完整性验证）
//! - 附件核对：DB 每行附件的物理文件存在性（全量 stat，千级附件毫秒完成）
//! - 最近备份信息（供「从备份恢复」决策）
//!
//! 设计原则「数据是生命线」的直接落点：宁可误报（退出钩子偶发未执行），
//! 不可漏报（崩溃后静默无提示）——检查是只读的，误报代价仅为一次弹窗。

use serde::Serialize;

use crate::services::backup;
use crate::{CoreContext, Result};

/// 上次会话信息（marker 文件内容）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSession {
    /// 上次会话开始时间（ISO）
    pub started_at: String,
    /// 上次会话进程 PID
    pub pid: u32,
}

/// 恢复检查报告（前端恢复对话框的数据源）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryReport {
    /// 是否检测到上次异常退出
    pub abnormal_exit: bool,
    /// 上次会话信息（异常退出时存在）
    pub last_session: Option<LastSession>,
    /// 数据库完整性是否通过
    pub db_ok: bool,
    /// integrity_check 的错误明细（db_ok=false 时非空）
    pub integrity_errors: Vec<String>,
    /// DB 中的附件总数
    pub attachment_total: i64,
    /// 物理文件缺失的附件总数（真值，供前端准确展示）
    pub attachment_missing_count: usize,
    /// 缺失附件的文件名样本（最多 10 个，供前端展示举例）
    pub attachment_missing: Vec<String>,
    /// 最近一次备份（供一键前往恢复）
    pub last_backup: Option<backup::BackupInfo>,
}

/// 会话 marker 文件路径（与 .lock 同级的隐藏文件）
fn marker_path(ctx: &CoreContext) -> std::path::PathBuf {
    ctx.paths.base.join(".last-session")
}

/// 启动时标记会话运行中。
/// 返回 `Some(last_session)` 表示检测到上次异常退出（marker 残留）。
pub fn mark_running(ctx: &CoreContext) -> Result<Option<LastSession>> {
    let path = marker_path(ctx);
    let last = read_marker(&path);
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    std::fs::write(&path, format!("{now}\n{}", std::process::id()))?;
    Ok(last)
}

/// 正常退出时清除标记（窗口关闭事件时调用）
pub fn mark_clean_exit(ctx: &CoreContext) -> Result<()> {
    let _ = std::fs::remove_file(marker_path(ctx));
    Ok(())
}

fn read_marker(path: &std::path::Path) -> Option<LastSession> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines = content.lines();
    let started_at = lines.next()?.trim().to_string();
    if started_at.is_empty() {
        return None;
    }
    let pid = lines
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    Some(LastSession { started_at, pid })
}

/// 数据健康检查（只读）
pub fn check_health(ctx: &CoreContext) -> Result<RecoveryReport> {
    let (db_ok, integrity_errors) = check_integrity(ctx)?;
    let (attachment_total, missing_count, missing_sample) = check_attachments(ctx)?;
    let last_backup = backup::list_backups(ctx)?.into_iter().next();

    Ok(RecoveryReport {
        abnormal_exit: false,
        last_session: None,
        db_ok,
        integrity_errors,
        attachment_total,
        attachment_missing_count: missing_count,
        attachment_missing: missing_sample,
        last_backup,
    })
}

/// 组合入口：启动时调用——标记会话 + 若异常退出则附带上次会话信息
pub fn check_recovery(ctx: &CoreContext) -> Result<RecoveryReport> {
    let last_session = mark_running(ctx)?;
    let mut report = check_health(ctx)?;
    report.abnormal_exit = last_session.is_some();
    report.last_session = last_session;
    Ok(report)
}

/// SQLite 完整性检查（单次查询；失败时返回多行错误说明）
fn check_integrity(ctx: &CoreContext) -> Result<(bool, Vec<String>)> {
    ctx.db.with_conn(|conn| {
        let mut stmt = conn.prepare("PRAGMA integrity_check")?;
        let mut rows = stmt.query([])?;
        let mut lines = Vec::new();
        while let Some(r) = rows.next()? {
            lines.push(r.get::<_, String>(0)?);
        }
        if lines.len() == 1 && lines[0] == "ok" {
            Ok((true, Vec::new()))
        } else {
            Ok((false, lines))
        }
    })
}

/// 附件物理文件核对：返回（总数, 缺失总数, 缺失 file_name 样本前 10 个）
fn check_attachments(ctx: &CoreContext) -> Result<(i64, usize, Vec<String>)> {
    let rows: Vec<(String, String)> = ctx.db.with_conn(|conn| {
        let mut stmt = conn.prepare("SELECT file_path, file_name FROM attachments")?;
        let mut rows = stmt.query([])?;
        let mut v = Vec::new();
        while let Some(r) = rows.next()? {
            v.push((r.get(0)?, r.get(1)?));
        }
        Ok(v)
    })?;

    let total = rows.len() as i64;
    let missing_rows: Vec<&String> = rows
        .iter()
        .filter(|(rel, _)| !ctx.paths.attachment_abs(rel).is_file())
        .map(|(_, name)| name)
        .collect();
    let missing_count = missing_rows.len();
    let sample = missing_rows.into_iter().take(10).cloned().collect();
    Ok((total, missing_count, sample))
}
