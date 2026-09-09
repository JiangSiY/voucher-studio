//! 恢复链：校验（inspect）/ 释放（restore）/ 演练（drill），快照经 materialize 透明合成。

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::format::with_materialized;
use super::local::list_backups;
use super::*;
use crate::{CoreContext, CoreError, Result};
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSummary {
    pub batch: String,
    pub created_at: String,
    pub restored_files: Vec<String>,
    /// 附件数（v1 备份为 0）
    pub attachment_count: u32,
    /// 是否完成 sha256 全量校验（v1 备份无清单 → false）
    pub checksums_verified: bool,
}

/// 逐文件校验 zip 内 sha256（v2 备份）。返回 (文件数, 是否全部通过)。
/// v1 备份（无 checksums）返回 (0, false)。
fn verify_checksums(zip: &mut zip::ZipArchive<std::fs::File>) -> Result<(usize, bool)> {
    let manifest = {
        let mut entry = zip.by_name(MANIFEST_FILE)?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        serde_json::from_str::<Manifest>(&buf)?
    };
    if manifest.checksums.is_empty() {
        return Ok((0, false));
    }
    let mut ok = true;
    let mut count = 0usize;
    for (name, expect) in &manifest.checksums {
        let Ok(mut entry) = zip.by_name(name) else {
            ok = false;
            continue;
        };
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        count += 1;
        if sha256_hex(&buf) != *expect {
            ok = false;
        }
    }
    Ok((count, ok))
}

/// 校验备份（格式/版本/完整性），返回清单信息。
/// 快照会自动与附件包合成全量再校验（对调用方透明）。
pub fn inspect_backup(ctx: &CoreContext, zip_path: &Path) -> Result<RestoreSummary> {
    with_materialized(ctx, zip_path, inspect_backup_impl)
}

/// 校验全量 zip（格式/版本/完整性），返回清单信息（内部实现，入参必须是全量）
pub(super) fn inspect_backup_impl(zip_path: &Path) -> Result<RestoreSummary> {
    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let manifest = {
        let mut entry = zip.by_name(MANIFEST_FILE).map_err(|_| {
            CoreError::Invalid("不是有效的 Voucher Studio 备份（缺 manifest.json）".into())
        })?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        let m: Manifest = serde_json::from_str(&buf)?;
        if m.format != BACKUP_FORMAT {
            return Err(CoreError::Invalid(
                "备份格式不识别（可能来自其他软件）".into(),
            ));
        }
        if m.version > BACKUP_VERSION {
            return Err(CoreError::Invalid(format!(
                "备份版本 v{} 高于当前程序支持的 v{}，请先升级程序",
                m.version, BACKUP_VERSION
            )));
        }
        m
    };

    let (_, checksums_ok) = verify_checksums(&mut zip)?;
    if manifest.version >= 2 && !checksums_ok {
        return Err(CoreError::Invalid(
            "备份完整性校验失败（文件损坏或被篡改），请换一份备份".into(),
        ));
    }

    Ok(RestoreSummary {
        batch: zip_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string(),
        created_at: manifest.created_at.clone(),
        restored_files: manifest.files.clone(),
        attachment_count: manifest.attachment_count,
        checksums_verified: manifest.version >= 2,
    })
}

/// 恢复演练：完整校验 + 解包到临时目录 + 打开数据库验证，不触碰真实数据。
/// 返回报告（供 3-2-1 定期演练用：验证整条逃生通道可用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrillReport {
    pub batch: String,
    pub created_at: String,
    pub db_ok: bool,
    pub files_checked: usize,
    pub attachment_count: u32,
    pub checksums_verified: bool,
}

/// 演练指定备份；path 为 None 时取最新一份。快照会自动合成全量（透明）。
pub fn drill_backup(ctx: &CoreContext, zip_path: Option<&Path>) -> Result<DrillReport> {
    let path = match zip_path {
        Some(p) => p.to_path_buf(),
        None => list_backups(ctx)?
            .into_iter()
            .next()
            .map(|b| PathBuf::from(b.path))
            .ok_or_else(|| CoreError::NotFound("还没有任何备份".into()))?,
    };

    let summary = with_materialized(ctx, &path, inspect_backup_impl)?;
    let drill_dir = ctx.paths.temp.join("backup-drill");
    let _ = std::fs::remove_dir_all(&drill_dir);
    let restored = with_materialized(ctx, &path, |full| restore_backup_impl(full, &drill_dir))?;

    // 演练目录里数据库必须可打开
    let db_ok = crate::db::connection::open(&drill_dir.join("data").join("voucher.db"))
        .and_then(|conn| {
            conn.query_row("SELECT COUNT(*) FROM vouchers", [], |r| r.get::<_, i64>(0))
                .map_err(CoreError::from)
        })
        .is_ok();
    let report = DrillReport {
        batch: summary.batch,
        created_at: summary.created_at,
        db_ok,
        files_checked: restored.restored_files.len(),
        attachment_count: restored.attachment_count,
        checksums_verified: summary.checksums_verified,
    };
    let _ = std::fs::remove_dir_all(&drill_dir);
    if !db_ok {
        return Err(CoreError::Invalid(format!(
            "演练失败：备份 {} 中的数据库无法打开",
            report.batch
        )));
    }
    Ok(report)
}

/// 把备份内容释放到 `target_dir`（data/ config/ attachments/ 由调用方安排重启后接管）。
/// 释放过程逐文件 sha256 校验；恢复后校验数据库可打开。
/// 快照会自动与附件包合成全量（透明）。
pub fn restore_backup(
    ctx: &CoreContext,
    zip_path: &Path,
    target_dir: &Path,
) -> Result<RestoreSummary> {
    with_materialized(ctx, zip_path, |full| restore_backup_impl(full, target_dir))
}

/// 释放全量 zip（内部实现，入参必须是全量）
pub(super) fn restore_backup_impl(zip_path: &Path, target_dir: &Path) -> Result<RestoreSummary> {
    let summary = inspect_backup_impl(zip_path)?;

    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let checksums = read_manifest(zip_path)
        .map(|m| m.checksums)
        .unwrap_or_default();

    let db_target = target_dir.join("data").join("voucher.db");
    // parent() 对带文件名的 join 路径必然存在；防御式 ok_or 避免重构时引入 panic
    let db_parent = db_target
        .parent()
        .ok_or_else(|| CoreError::Invalid("恢复目标路径无效".into()))?;
    std::fs::create_dir_all(db_parent)?;
    extract_verified(&mut zip, DB_ENTRY, &db_target, &checksums)?;

    let cfg_target = target_dir.join("config").join("app.json");
    if zip.by_name(CONFIG_ENTRY).is_ok() {
        let cfg_parent = cfg_target
            .parent()
            .ok_or_else(|| CoreError::Invalid("恢复目标路径无效".into()))?;
        std::fs::create_dir_all(cfg_parent)?;
        extract_verified(&mut zip, CONFIG_ENTRY, &cfg_target, &checksums)?;
    }

    // 附件（v2 起）：释放为 attachments/ 树
    for i in 0..zip.len() {
        let name = {
            let entry = zip.by_index(i)?;
            entry.name().to_string()
        };
        if name.starts_with(ATTACHMENTS_PREFIX) && !name.ends_with('/') {
            // 防路径穿越 + 防绝对路径外写：条目名可能用 `\` 分隔，规范化后必须仍是
            // attachments/ 下的相对路径。绝对路径（/、\ 开头、Windows 盘符、UNC）
            // 或含 `..` 路径段的条目一律拒绝恢复，而不是静默跳过。
            let rel = name.strip_prefix(ATTACHMENTS_PREFIX).unwrap_or("");
            ensure_safe_rel_entry(&name, rel)?;
            let target = target_dir.join("attachments").join(rel);
            extract_verified(&mut zip, &name, &target, &checksums)?;
        }
    }

    // 校验恢复出的库可用
    let conn = crate::db::connection::open(&db_target)?;
    conn.query_row("SELECT COUNT(*) FROM vouchers", [], |r| r.get::<_, i64>(0))?;
    Ok(summary)
}

/// 解包单个 entry 并做 sha256 校验（manifest 有该 entry 的校验和时）
fn extract_verified<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    target: &Path,
    checksums: &HashMap<String, String>,
) -> Result<()> {
    let mut entry = zip.by_name(name)?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;

    if let Some(expect) = checksums.get(name) {
        if sha256_hex(&buf) != *expect {
            return Err(CoreError::Invalid(format!(
                "文件 {name} 校验失败（损坏或被篡改）"
            )));
        }
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // 先写临时再替换，避免覆盖一半
    let tmp = target.with_extension("restore-tmp");
    std::fs::write(&tmp, &buf)?;
    if target.exists() {
        std::fs::remove_file(target)?;
    }
    std::fs::rename(&tmp, target)?;
    Ok(())
}

/// 校验 zip 附件条目剥离 `attachments/` 前缀后的相对路径可安全写盘。
///
/// zip 条目可能用 `\` 分隔路径，需先按两种分隔符统一判定：
/// - 空路径
/// - 绝对路径：以 `/` 或 `\` 开头、UNC（`\\`、`//`）、Windows 盘符（`C:` 开头）
/// - 任一路径段为 `..`（路径穿越）
///
/// 命中任一即拒绝整份恢复（返回错误），不静默跳过——损坏/恶意备份应显式暴露。
fn ensure_safe_rel_entry(full_name: &str, rel: &str) -> Result<()> {
    let is_absolute = rel.starts_with('/')
        || rel.starts_with('\\')
        || rel
            .as_bytes()
            .get(..2)
            .is_some_and(|b| b[0].is_ascii_alphabetic() && b[1] == b':');
    let has_dotdot = rel.split(['/', '\\']).any(|seg| seg == "..");
    if rel.is_empty() || is_absolute || has_dotdot {
        return Err(CoreError::Invalid(format!(
            "备份包含不安全的附件路径，已拒绝恢复：{full_name}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_rel_allows_normal_relative_paths() {
        // 正常相对路径（含中文/空格文件名）放行
        assert!(ensure_safe_rel_entry("attachments/2026/08/a.jpg", "2026/08/a.jpg").is_ok());
        // 反斜杠分隔的相对路径（zip 条目非标准写法）也应放行
        assert!(ensure_safe_rel_entry("attachments\\2026\\08\\a.jpg", "2026\\08\\a.jpg").is_ok());
        assert!(ensure_safe_rel_entry(
            "attachments/2026/08/凭证 收据.png",
            "2026/08/凭证 收据.png"
        )
        .is_ok());
    }

    #[test]
    fn safe_rel_rejects_absolute_paths() {
        // 绝对路径（POSIX / Windows / UNC）一律拒绝
        assert!(ensure_safe_rel_entry("attachments//etc/passwd", "/etc/passwd").is_err());
        assert!(ensure_safe_rel_entry("attachments//etc/passwd", "//etc/passwd").is_err());
        assert!(ensure_safe_rel_entry("attachments\\evil", "\\evil").is_err());
        assert!(
            ensure_safe_rel_entry("attachments/\\\\server\\share\\x", "\\\\server\\share\\x")
                .is_err()
        );
    }

    #[test]
    fn safe_rel_rejects_windows_drive_and_traversal() {
        // Windows 盘符
        assert!(ensure_safe_rel_entry("attachments/C:\\evil", "C:\\evil").is_err());
        assert!(ensure_safe_rel_entry("attachments/C:/evil", "C:/evil").is_err());
        assert!(ensure_safe_rel_entry("attachments/c:/evil", "c:/evil").is_err());
        // 路径穿越（跨分隔符）
        assert!(ensure_safe_rel_entry("attachments/../x", "../x").is_err());
        assert!(ensure_safe_rel_entry("attachments/..\\..\\x", "..\\..\\x").is_err());
        assert!(ensure_safe_rel_entry("attachments/2026/08/../../x", "2026/08/../../x").is_err());
        assert!(ensure_safe_rel_entry("attachments/..\\x", "..\\x").is_err());
        // 空路径
        assert!(ensure_safe_rel_entry("attachments/", "").is_err());
    }
}
