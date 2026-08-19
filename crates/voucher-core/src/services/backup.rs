//! 备份与恢复（对应 v4.3 的 `webdav_backup.py`；备份为新版自有 zip 格式）
//!
//! 数据是生命线：本地 zip + 可选 WebDAV 上传；zip 内含版本化清单，
//! 恢复时校验格式与版本。旧版（v4.3）备份不在兼容范围内。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::db::repos::config_repo;
use crate::utils::config::SECRET_WEBDAV_PASSWORD;
use crate::{CoreContext, CoreError, Result};

const BACKUP_FORMAT: &str = "voucher-studio-backup";
const BACKUP_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "manifest.json";
pub const DB_ENTRY: &str = "voucher.db";
pub const CONFIG_ENTRY: &str = "app.json";

pub const KEY_LAST_BACKUP: &str = "last_backup_at";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub file_name: String,
    pub path: String,
    pub size: u64,
    /// zip 内的创建时间
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    created_at: String,
    files: Vec<String>,
}

/// 立即执行一次备份：SQLite VACUUM INTO 快照 + 配置文件 → zip → backups/
pub fn run_backup(ctx: &CoreContext) -> Result<BackupInfo> {
    // 1. 数据库一致性快照
    let snap = ctx.paths.temp.join("backup-snapshot.db");
    if snap.exists() {
        std::fs::remove_file(&snap)?;
    }
    ctx.db.with_conn(|conn| {
        conn.execute("VACUUM INTO ?1", params![snap.to_string_lossy()])?;
        Ok(())
    })?;

    // 2. 打包
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let file_name = format!("voucher-backup-{stamp}.zip");
    let zip_path = ctx.paths.backups.join(&file_name);
    let tmp_zip = ctx.paths.temp.join(&file_name);

    let created_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let manifest = Manifest {
        format: BACKUP_FORMAT.into(),
        version: BACKUP_VERSION,
        created_at: created_at.clone(),
        files: vec![DB_ENTRY.into(), CONFIG_ENTRY.into()],
    };

    {
        let file = std::fs::File::create(&tmp_zip)?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        zip.start_file(MANIFEST_FILE, opts)?;
        zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;

        zip.start_file(DB_ENTRY, opts)?;
        std::io::copy(&mut std::fs::File::open(&snap)?, &mut zip)?;

        if ctx.paths.config_file().exists() {
            zip.start_file(CONFIG_ENTRY, opts)?;
            std::io::copy(&mut std::fs::File::open(ctx.paths.config_file())?, &mut zip)?;
        }
        zip.finish()?;
    }
    std::fs::copy(&tmp_zip, &zip_path)?;
    let _ = std::fs::remove_file(&tmp_zip);
    let _ = std::fs::remove_file(&snap);

    // 3. 保留策略：超出份数删最旧
    enforce_retention(ctx)?;

    // 4. 记录状态
    let info = BackupInfo {
        file_name,
        path: zip_path.to_string_lossy().to_string(),
        size: std::fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0),
        created_at,
    };
    ctx.db.with_conn(|conn| {
        config_repo::set(conn, KEY_LAST_BACKUP, &chrono::Utc::now().to_rfc3339())
    })?;
    Ok(info)
}

fn enforce_retention(ctx: &CoreContext) -> Result<()> {
    let retention = ctx.config().backup.retention as usize;
    if retention == 0 {
        return Ok(());
    }
    let mut backups = list_backups(ctx)?;
    while backups.len() > retention {
        let oldest = backups.pop(); // list_backups 已按新→旧排序
        if let Some(b) = oldest {
            let p = PathBuf::from(&b.path);
            if ctx.paths.is_inside_backups(&p) {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    Ok(())
}

pub fn list_backups(ctx: &CoreContext) -> Result<Vec<BackupInfo>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&ctx.paths.backups)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("zip") {
            continue;
        }
        let created_at = read_backup_created_at(&path).unwrap_or_default();
        out.push(BackupInfo {
            file_name: path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
            path: path.to_string_lossy().to_string(),
            size: entry.metadata().map(|m| m.len()).unwrap_or(0),
            created_at,
        });
    }
    out.sort_by(|a, b| b.file_name.cmp(&a.file_name));
    Ok(out)
}

fn read_backup_created_at(zip_path: &Path) -> Option<String> {
    let file = std::fs::File::open(zip_path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut entry = zip.by_name(MANIFEST_FILE).ok()?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf).ok()?;
    let m: Manifest = serde_json::from_str(&buf).ok()?;
    Some(m.created_at)
}

/// 恢复摘要（预览 + 执行共用）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSummary {
    pub batch: String,
    pub created_at: String,
    pub restored_files: Vec<String>,
}

/// 校验备份 zip（格式/版本），返回清单信息
pub fn inspect_backup(zip_path: &Path) -> Result<RestoreSummary> {
    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file)?;
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
    Ok(RestoreSummary {
        batch: zip_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string(),
        created_at: m.created_at,
        restored_files: m.files.clone(),
    })
}

/// 把备份内容释放到 `target_dir`（data/ 与 config/ 由调用方安排重启后接管）。
/// 释放后会校验数据库可打开。
pub fn restore_backup(zip_path: &Path, target_dir: &Path) -> Result<RestoreSummary> {
    let summary = inspect_backup(zip_path)?;

    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file)?;

    let db_target = target_dir.join("data").join("voucher.db");
    std::fs::create_dir_all(db_target.parent().unwrap())?;
    extract_entry(&mut zip, DB_ENTRY, &db_target)?;

    let cfg_target = target_dir.join("config").join("app.json");
    if zip.by_name(CONFIG_ENTRY).is_ok() {
        std::fs::create_dir_all(cfg_target.parent().unwrap())?;
        extract_entry(&mut zip, CONFIG_ENTRY, &cfg_target)?;
    }

    // 校验恢复出的库可用
    let conn = crate::db::connection::open(&db_target)?;
    conn.query_row("SELECT COUNT(*) FROM vouchers", [], |r| r.get::<_, i64>(0))?;
    Ok(summary)
}

fn extract_entry<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    target: &Path,
) -> Result<()> {
    let mut entry = zip.by_name(name)?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;
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

/// 上传备份到 WebDAV（设置 → 备份与同步）
pub fn upload_to_webdav(ctx: &CoreContext, zip_path: &Path) -> Result<()> {
    let cfg = ctx.config();
    let webdav = &cfg.backup.webdav;
    if !webdav.enabled || webdav.url.trim().is_empty() {
        return Err(CoreError::Invalid("WebDAV 未启用或地址为空".into()));
    }
    let password = ctx
        .secrets
        .get(SECRET_WEBDAV_PASSWORD)?
        .ok_or_else(|| CoreError::Invalid("WebDAV 密码未设置（密钥管理）".into()))?;

    let file_name = zip_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| CoreError::Invalid("非法的备份文件名".into()))?;
    let url = format!("{}/{}", webdav.url.trim().trim_end_matches('/'), file_name);

    let bytes = std::fs::read(zip_path)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    client
        .put(&url)
        .basic_auth(&webdav.username, Some(&password))
        .body(bytes)
        .send()?
        .error_for_status()?;
    Ok(())
}
