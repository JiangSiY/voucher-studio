//! 增强版命中仓储（v4 / v0.6.5）
//!
//! 记录「增强版被实际使用」的次数：
//! - `ocr`     —— run_for_attachment 走了 enhanced_path
//! - `export`  —— 导出 PDF 时使用 enhanced_path（D-6 起仅扫描件增强版进 PDF）
//! - `viewer`  —— 编辑页大图查看器打开 enhanced_path（D-7 经 record_viewer_hit 接通）

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::utils::paths::now_iso;
use crate::Result;

/// 命中来源（与 schema CHECK 约束一致）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitSource {
    Ocr,
    Export,
    Viewer,
}

impl HitSource {
    pub fn as_str(self) -> &'static str {
        match self {
            HitSource::Ocr => "ocr",
            HitSource::Export => "export",
            HitSource::Viewer => "viewer",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ocr" => Some(HitSource::Ocr),
            "export" => Some(HitSource::Export),
            "viewer" => Some(HitSource::Viewer),
            _ => None,
        }
    }
}

/// D-8：按 source 聚合的命中数（ocr / export / viewer，缺失项为 0）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HitCounts {
    pub ocr: u64,
    pub export: u64,
    pub viewer: u64,
}

impl HitCounts {
    pub fn total(&self) -> u64 {
        self.ocr + self.export + self.viewer
    }
}

/// 记录一次命中
pub fn record(conn: &Connection, attachment_id: i64, source: HitSource) -> Result<()> {
    conn.execute(
        "INSERT INTO enhance_hits(attachment_id, source, hit_at) VALUES(?, ?, ?)",
        params![attachment_id, source.as_str(), now_iso()],
    )?;
    Ok(())
}

/// 单条附件的命中次数（给最近列表 LEFT JOIN 用）
pub fn count_by_attachment(conn: &Connection, attachment_id: i64) -> Result<u32> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM enhance_hits WHERE attachment_id = ?",
        params![attachment_id],
        |r| r.get(0),
    )?;
    Ok(n as u32)
}

/// 批量：按 attachment_id 聚合命中次数（最近列表用）
pub fn count_grouped_by_attachment(conn: &Connection) -> Result<Vec<(i64, u32)>> {
    let mut stmt =
        conn.prepare("SELECT attachment_id, COUNT(*) FROM enhance_hits GROUP BY attachment_id")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? as u32))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 全表命中总数（设置页「总命中」指标）
pub fn count_total(conn: &Connection) -> Result<u64> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM enhance_hits", [], |r| r.get(0))?;
    Ok(n as u64)
}

/// D-8：按 source 聚合命中数（设置页「OCR / 导出 / 查看」分列指标）
pub fn count_by_source(conn: &Connection) -> Result<HitCounts> {
    let mut stmt = conn.prepare("SELECT source, COUNT(*) FROM enhance_hits GROUP BY source")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
    })?;
    let mut counts = HitCounts::default();
    for r in rows {
        let (source, n) = r?;
        match HitSource::parse(&source) {
            Some(HitSource::Ocr) => counts.ocr = n,
            Some(HitSource::Export) => counts.export = n,
            Some(HitSource::Viewer) => counts.viewer = n,
            _ => log::warn!("enhance_hits 出现未知 source: {source}（跳过）"),
        }
    }
    Ok(counts)
}

/// 近 N 天命中数（设置页「近 30 天命中」指标）
pub fn count_since(conn: &Connection, since_iso: &str) -> Result<u64> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM enhance_hits WHERE hit_at >= ?",
        params![since_iso],
        |r| r.get(0),
    )?;
    Ok(n as u64)
}

/// D-10：按日聚合的命中数（趋势看板用）
///
/// `day` 为 `hit_at` 前 10 字符（本地日期 YYYY-MM-DD；now_iso 与本地时区一致）。
#[derive(Debug, Clone, Serialize)]
pub struct DailyHitCount {
    pub day: String,
    pub ocr: u64,
    pub export: u64,
    pub viewer: u64,
}

/// D-10：取 since_iso（含）之后每个自然日的三类命中数（无命中的日期不返回，由服务层补零）
///
/// hit_at 存 UTC（now_iso），这里用 SQLite `localtime` 折算本地日期再聚合，
/// 与用户在 UI 看到的「今天」对齐。
pub fn daily_counts(conn: &Connection, since_iso: &str) -> Result<Vec<DailyHitCount>> {
    let mut stmt = conn.prepare(
        "SELECT substr(datetime(hit_at, 'localtime'), 1, 10) AS day, \
                COALESCE(SUM(source = 'ocr'), 0), \
                COALESCE(SUM(source = 'export'), 0), \
                COALESCE(SUM(source = 'viewer'), 0) \
         FROM enhance_hits WHERE hit_at >= ? GROUP BY day ORDER BY day",
    )?;
    let rows = stmt.query_map(params![since_iso], |r| {
        Ok(DailyHitCount {
            day: r.get(0)?,
            ocr: r.get::<_, i64>(1)? as u64,
            export: r.get::<_, i64>(2)? as u64,
            viewer: r.get::<_, i64>(3)? as u64,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 释放某条附件时级联（ON DELETE CASCADE 已保证，但显式调用便于单测）
pub fn delete_by_attachment(conn: &Connection, attachment_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM enhance_hits WHERE attachment_id = ?",
        params![attachment_id],
    )?;
    Ok(())
}
