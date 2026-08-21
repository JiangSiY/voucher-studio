//! 备份与恢复（对应 v4.3 的 `webdav_backup.py`；备份为新版自有 zip 格式）
//!
//! 3-2-1 备份策略：
//! - 3 份数据：当前库 + 本地备份（GFS 分层保留）+ 异盘/云端副本
//! - 2 种介质：本地盘（backups/）+ 备用份目录（secondary_dir，建议异物理盘）
//! - 1 份异地：WebDAV 上传（带重试与远端保留清理）
//!
//! 备份内容：voucher.db 快照 + app.json + attachments/ 全部附件原图。
//! zip 内含版本化 manifest（含每文件 sha256），恢复与演练时逐文件校验。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::repos::config_repo;
use crate::utils::config::SECRET_WEBDAV_PASSWORD;
use crate::{CoreContext, CoreError, Result};

const BACKUP_FORMAT: &str = "voucher-studio-backup";
/// v2：附件入包 + sha256 清单（v1 备份仍可恢复，附件缺失时保留现有附件）
const BACKUP_VERSION: u32 = 2;
pub const MANIFEST_FILE: &str = "manifest.json";
pub const DB_ENTRY: &str = "voucher.db";
pub const CONFIG_ENTRY: &str = "app.json";
pub const ATTACHMENTS_PREFIX: &str = "attachments/";

pub const KEY_LAST_BACKUP: &str = "last_backup_at";

/// WebDAV 上传总尝试次数（失败重试 2 次，指数退避）
const UPLOAD_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub file_name: String,
    pub path: String,
    pub size: u64,
    /// zip 内的创建时间
    pub created_at: String,
    /// 附件文件数（v1 备份为 0）
    pub attachment_count: u32,
    /// 非阻断性告警（异盘/WebDAV 失败等；本地主备份不受影响，但 3-2-1 已不完整，需提醒用户）
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    created_at: String,
    files: Vec<String>,
    /// entry 名 → sha256 hex（v2 起有；v1 备份为空 = 恢复时跳过校验）
    #[serde(default)]
    checksums: HashMap<String, String>,
    #[serde(default)]
    attachment_count: u32,
}

/// 立即执行一次备份：DB 快照 + 配置 + 附件 → zip → 本地 + 异盘 + WebDAV。
/// 手动与自动备份统一走这里，保证行为一致。
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

    // 2. 收集附件（相对 attachments/ 的路径，稳定排序保证 zip 可复现）
    let attachments = collect_attachment_files(&ctx.paths.attachments)?;

    // 3. 打包：manifest 校验和随写入逐文件计算
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let file_name = format!("voucher-backup-{stamp}.zip");
    let zip_path = ctx.paths.backups.join(&file_name);
    let tmp_zip = ctx.paths.temp.join(&file_name);

    let created_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut checksums = HashMap::new();

    {
        let file = std::fs::File::create(&tmp_zip)?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        // DB 快照
        let db_bytes = std::fs::read(&snap)?;
        checksums.insert(DB_ENTRY.into(), sha256_hex(&db_bytes));
        zip.start_file(DB_ENTRY, opts)?;
        zip.write_all(&db_bytes)?;

        // 配置
        if ctx.paths.config_file().exists() {
            let cfg_bytes = std::fs::read(ctx.paths.config_file())?;
            checksums.insert(CONFIG_ENTRY.into(), sha256_hex(&cfg_bytes));
            zip.start_file(CONFIG_ENTRY, opts)?;
            zip.write_all(&cfg_bytes)?;
        }

        // 附件原图（凭证核心资产，不可再生）
        for rel in &attachments {
            let bytes = std::fs::read(ctx.paths.attachments.join(rel))?;
            let entry = format!("{ATTACHMENTS_PREFIX}{rel}");
            checksums.insert(entry.clone(), sha256_hex(&bytes));
            zip.start_file(entry.as_str(), opts)?;
            zip.write_all(&bytes)?;
        }

        let manifest = Manifest {
            format: BACKUP_FORMAT.into(),
            version: BACKUP_VERSION,
            created_at: created_at.clone(),
            files: checksums.keys().cloned().collect(),
            checksums,
            attachment_count: attachments.len() as u32,
        };
        zip.start_file(MANIFEST_FILE, zip::write::SimpleFileOptions::default())?;
        zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        zip.finish()?;
    }
    std::fs::copy(&tmp_zip, &zip_path)?;
    let _ = std::fs::remove_file(&tmp_zip);
    let _ = std::fs::remove_file(&snap);

    // 4. 异盘双写（secondary_dir 配置了才启用；失败不阻断主流程，记入告警）
    let mut warnings = Vec::new();
    if let Err(e) = copy_to_secondary(ctx, &zip_path) {
        log::warn!("备用份目录写入失败: {e}");
        warnings.push(format!("异盘副本写入失败：{e}"));
    }

    // 5. WebDAV 上传（带重试）+ 远端保留清理；失败不阻断主流程，记入告警
    if let Err(e) = upload_backup_to_webdav(ctx, &zip_path) {
        log::warn!("WebDAV 上传失败（本地备份不受影响）: {e}");
        warnings.push(format!("WebDAV 异地副本上传失败：{e}"));
    }

    let info = BackupInfo {
        file_name: file_name.clone(),
        path: zip_path.to_string_lossy().to_string(),
        size: std::fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0),
        created_at: created_at.clone(),
        attachment_count: attachments.len() as u32,
        warnings,
    };

    // 6. 本地 GFS 分层保留
    enforce_retention(ctx)?;

    // 7. 记录状态
    ctx.db.with_conn(|conn| {
        config_repo::set(conn, KEY_LAST_BACKUP, &chrono::Utc::now().to_rfc3339())
    })?;
    Ok(info)
}

/// 递归收集 attachments/ 下全部文件的相对路径（正斜杠分隔，按路径排序）
fn collect_attachment_files(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return Ok(out);
    }
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let dir = root.join(&rel);
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                let mut sub = rel.clone();
                sub.push(name.as_ref());
                stack.push(sub);
            } else {
                let mut file = rel.clone();
                file.push(name.as_ref());
                out.push(file.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    Ok(out)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    let mut s = String::with_capacity(d.len() * 2);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

// ---------- GFS 分层保留（日/周/月） ----------

/// 从备份文件名解析日期（voucher-backup-YYYYMMDD-HHMMSS.zip）
fn date_of_file_name(name: &str) -> Option<NaiveDate> {
    let digits: String = name
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take(8)
        .collect();
    NaiveDate::parse_from_str(&digits, "%Y%m%d").ok()
}

/// GFS 选择（纯函数，便于测试）：输入按新→旧排序的备份日期，
/// 返回逐项「是否保留」标记。基准取「最新备份」而非今天——停机多日仍能正确分层。
fn gfs_keep(
    dates: &[NaiveDate],
    keep_daily: u32,
    keep_weekly: u32,
    keep_monthly: u32,
) -> Vec<bool> {
    if dates.is_empty() {
        return Vec::new();
    }
    let mut keep = vec![true; dates.len()];
    let newest = dates[0];

    // 周序号 / 月序号（ISO 周；月用 (年, 月)）
    let week_of = |d: NaiveDate| d.iso_week().week() as i32 + d.iso_week().year() * 100;
    let month_of = |d: NaiveDate| d.month() as i32 + d.year() * 12;

    let mut weeks_seen: Vec<i32> = Vec::new();
    let mut months_seen: Vec<i32> = Vec::new();
    let mut weekly_left = keep_weekly;
    let mut monthly_left = keep_monthly;

    for (i, d) in dates.iter().enumerate() {
        let age_days = (newest - *d).num_days();
        if keep_daily > 0 && age_days < keep_daily as i64 {
            continue; // 日备窗口内：默认保留
        }
        let w = week_of(*d);
        let m = month_of(*d);
        let in_weekly = weeks_seen.contains(&w);
        let in_monthly = months_seen.contains(&m);
        if weekly_left > 0 && !in_weekly {
            // 该周第一份（最新）→ 周备
            weeks_seen.push(w);
            months_seen.push(m);
            weekly_left -= 1;
            continue;
        }
        if monthly_left > 0 && !in_monthly {
            // 该月第一份（最新）→ 月备
            months_seen.push(m);
            monthly_left -= 1;
            continue;
        }
        keep[i] = false;
    }
    keep
}

fn enforce_retention(ctx: &CoreContext) -> Result<()> {
    let cfg = ctx.config().backup;
    let backups = list_backups(ctx)?; // 新→旧
    let dates: Vec<Option<NaiveDate>> = backups
        .iter()
        .map(|b| date_of_file_name(&b.file_name))
        .collect();
    // 解析不出日期的备份（外部拷入等）一律保留，不参与清理
    let dated: Vec<(usize, NaiveDate)> = dates
        .iter()
        .enumerate()
        .filter_map(|(i, d)| d.map(|d| (i, d)))
        .collect();
    if dated.is_empty() {
        return Ok(());
    }
    let keep_flags = gfs_keep(
        &dated.iter().map(|(_, d)| *d).collect::<Vec<_>>(),
        cfg.keep_daily,
        cfg.keep_weekly,
        cfg.keep_monthly,
    );
    for (pos, keep) in keep_flags.iter().enumerate() {
        if !keep {
            let (idx, _) = dated[pos];
            let p = PathBuf::from(&backups[idx].path);
            if ctx.paths.is_inside_backups(&p) {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    Ok(())
}

/// 备份复制到备用份目录（异盘双写）+ 该目录同样应用 GFS 清理
fn copy_to_secondary(ctx: &CoreContext, zip_path: &Path) -> Result<()> {
    let dir = ctx.config().backup.secondary_dir;
    let dir = dir.trim();
    if dir.is_empty() {
        return Ok(());
    }
    let dir = PathBuf::from(dir);
    // 与数据目录相同则无意义（用户配错）
    if dir == ctx.paths.backups {
        return Ok(());
    }
    std::fs::create_dir_all(&dir)?;
    let dst = dir.join(zip_path.file_name().unwrap_or_default());
    std::fs::copy(zip_path, dst)?;

    // 次级目录同样 GFS 清理（仅清 voucher-backup-*.zip）
    let cfg = ctx.config().backup;
    let mut names: Vec<(NaiveDate, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if let Some(d) = date_of_file_name(name) {
            if name.starts_with("voucher-backup-") && name.ends_with(".zip") {
                names.push((d, path));
            }
        }
    }
    names.sort_by_key(|(d, _)| std::cmp::Reverse(*d)); // 新→旧
    let keep = gfs_keep(
        &names.iter().map(|(d, _)| *d).collect::<Vec<_>>(),
        cfg.keep_daily,
        cfg.keep_weekly,
        cfg.keep_monthly,
    );
    for (i, keep_flag) in keep.iter().enumerate() {
        if !keep_flag {
            let _ = std::fs::remove_file(&names[i].1);
        }
    }
    Ok(())
}

// ---------- 备份列表 / 校验 ----------

pub fn list_backups(ctx: &CoreContext) -> Result<Vec<BackupInfo>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&ctx.paths.backups)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("zip") {
            continue;
        }
        let manifest = read_manifest(&path);
        out.push(BackupInfo {
            file_name: path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
            path: path.to_string_lossy().to_string(),
            size: entry.metadata().map(|m| m.len()).unwrap_or(0),
            created_at: manifest
                .as_ref()
                .map(|m| m.created_at.clone())
                .unwrap_or_default(),
            attachment_count: manifest.as_ref().map(|m| m.attachment_count).unwrap_or(0),
            warnings: Vec::new(),
        });
    }
    out.sort_by(|a, b| b.file_name.cmp(&a.file_name));
    Ok(out)
}

fn read_manifest(zip_path: &Path) -> Option<Manifest> {
    let file = std::fs::File::open(zip_path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut entry = zip.by_name(MANIFEST_FILE).ok()?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}

/// 恢复摘要（预览 + 执行共用）
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

/// 校验备份 zip（格式/版本/完整性），返回清单信息
pub fn inspect_backup(zip_path: &Path) -> Result<RestoreSummary> {
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

/// 演练指定备份；path 为 None 时取最新一份
pub fn drill_backup(ctx: &CoreContext, zip_path: Option<&Path>) -> Result<DrillReport> {
    let path = match zip_path {
        Some(p) => p.to_path_buf(),
        None => list_backups(ctx)?
            .into_iter()
            .next()
            .map(|b| PathBuf::from(b.path))
            .ok_or_else(|| CoreError::NotFound("还没有任何备份".into()))?,
    };

    let summary = inspect_backup(&path)?;
    let drill_dir = ctx.paths.temp.join("backup-drill");
    let _ = std::fs::remove_dir_all(&drill_dir);
    let restored = restore_backup(&path, &drill_dir)?;

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
pub fn restore_backup(zip_path: &Path, target_dir: &Path) -> Result<RestoreSummary> {
    let summary = inspect_backup(zip_path)?;

    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let checksums = read_manifest(zip_path)
        .map(|m| m.checksums)
        .unwrap_or_default();

    let db_target = target_dir.join("data").join("voucher.db");
    std::fs::create_dir_all(db_target.parent().unwrap())?;
    extract_verified(&mut zip, DB_ENTRY, &db_target, &checksums)?;

    let cfg_target = target_dir.join("config").join("app.json");
    if zip.by_name(CONFIG_ENTRY).is_ok() {
        std::fs::create_dir_all(cfg_target.parent().unwrap())?;
        extract_verified(&mut zip, CONFIG_ENTRY, &cfg_target, &checksums)?;
    }

    // 附件（v2 起）：释放为 attachments/ 树
    for i in 0..zip.len() {
        let name = {
            let entry = zip.by_index(i)?;
            entry.name().to_string()
        };
        if name.starts_with(ATTACHMENTS_PREFIX) && !name.ends_with('/') {
            // 防路径穿越：entry 名规范化后必须仍在 attachments/ 下
            let rel = name.strip_prefix(ATTACHMENTS_PREFIX).unwrap_or("");
            if rel.contains("..") || rel.is_empty() {
                continue;
            }
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

// ---------- WebDAV（上传 + 重试 + 远端保留） ----------

/// 上传备份到 WebDAV（设置 → 备份与同步）；带指数退避重试，成功后清理远端超额备份。
/// 未启用时静默跳过（Ok）。
pub fn upload_backup_to_webdav(ctx: &CoreContext, zip_path: &Path) -> Result<()> {
    let cfg = ctx.config();
    let webdav = &cfg.backup.webdav;
    if !webdav.enabled || webdav.url.trim().is_empty() {
        return Ok(());
    }
    let password = ctx
        .secrets
        .get(SECRET_WEBDAV_PASSWORD)?
        .ok_or_else(|| CoreError::Invalid("WebDAV 密码未设置（密钥管理）".into()))?;

    let file_name = zip_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| CoreError::Invalid("非法的备份文件名".into()))?;
    let base = webdav.url.trim().trim_end_matches('/');
    let url = format!("{base}/{file_name}");

    let bytes = std::fs::read(zip_path)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    // 重试：2s / 8s 指数退避
    let mut last_err = None;
    for attempt in 0..UPLOAD_ATTEMPTS {
        if attempt > 0 {
            let delay = 2u64.pow(attempt); // 2s, 4s
            std::thread::sleep(std::time::Duration::from_secs(delay));
            log::info!("WebDAV 上传重试 #{attempt}: {file_name}");
        }
        match client
            .put(&url)
            .basic_auth(&webdav.username, Some(&password))
            .body(bytes.clone())
            .send()
        {
            Ok(resp) if resp.status().is_success() => {
                last_err = None;
                break;
            }
            Ok(resp) => {
                last_err = Some(CoreError::Invalid(format!("WebDAV 返回 {}", resp.status())))
            }
            Err(e) => last_err = Some(CoreError::Http(e)),
        }
    }
    if let Some(e) = last_err {
        return Err(e);
    }

    // 远端保留清理
    if webdav.remote_retention > 0 {
        if let Err(e) = cleanup_webdav(
            &client,
            base,
            &webdav.username,
            &password,
            webdav.remote_retention,
        ) {
            log::warn!("WebDAV 远端清理失败（不影响上传结果）: {e}");
        }
    }
    Ok(())
}

/// 列出远端 voucher-backup-*.zip，超出保留数删最旧。
/// PROPFIND 解析采用宽松文本匹配（兼容 Nextcloud / 坚果云 / Alist 等主流实现）。
fn cleanup_webdav(
    client: &reqwest::blocking::Client,
    base: &str,
    user: &str,
    password: &str,
    keep: u32,
) -> Result<()> {
    let resp = client
        .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), base)
        .basic_auth(user, Some(password))
        .header("Depth", "1")
        .send()?
        .error_for_status()?;
    let body = resp.text()?;
    let lower = body.to_lowercase();
    let mut names: Vec<String> = Vec::new();
    let mut pos = 0;
    while let Some(start) = lower[pos..]
        .find("<d:href")
        .or_else(|| lower[pos..].find("<href"))
    {
        let abs = pos + start;
        let Some(gt) = lower[abs..].find('>') else {
            break;
        };
        let content_start = abs + gt + 1;
        let Some(end) = lower[content_start..].find("</") else {
            break;
        };
        let content_end = content_start + end;
        let href = &body[content_start..content_end];
        // 只关心本程序的备份 zip，排除目录自身（href 以 / 结尾）
        if let Some(name) = href.rsplit('/').next() {
            if name.starts_with("voucher-backup-") && name.ends_with(".zip") {
                // URL 解码 %XX（Nextcloud 常见）
                let decoded = percent_decode(name);
                names.push(decoded);
            }
        }
        pos = content_end;
    }
    names.sort();
    names.dedup();
    let excess = names.len().saturating_sub(keep as usize);
    for name in &names[..excess] {
        let del_url = format!("{base}/{}", url_escape(name));
        let status = client
            .delete(&del_url)
            .basic_auth(user, Some(password))
            .send()?
            .status();
        if !status.is_success() {
            log::warn!("WebDAV 删除远端备份 {name} 返回 {status}");
        }
    }
    Ok(())
}

/// 简易 %XX 解码
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 1 && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 只对路径不安全字符做转义（保留 /）
fn url_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn gfs_keeps_daily_window_weekly_and_monthly() {
        // 2026-08-03/10/17 均为周一；同周配对：08-10+08-11、08-03+08-04、07-14+07-15
        let dates = vec![
            d("2026-08-18"), // 最新（日备窗口内）
            d("2026-08-17"), // 日备窗口内
            d("2026-08-11"), // 周备 #1（第 33 周）
            d("2026-08-10"), // 同周更旧 → 删
            d("2026-08-04"), // 周备 #2（第 32 周）
            d("2026-08-03"), // 同周更旧 → 删
            d("2026-07-15"), // 周备 #3（第 29 周）
            d("2026-07-14"), // 同周更旧 → 删
            d("2026-06-10"), // 月备 #1（6 月）
        ];
        let keep = gfs_keep(&dates, 2, 3, 2);
        assert_eq!(
            keep,
            vec![true, true, true, false, true, false, true, false, true]
        );
    }

    #[test]
    fn gfs_zero_daily_zero_weekly() {
        // 只留月备：每月最新一份，共 2 份
        let dates = vec![
            d("2026-08-18"), // 月备 #1（8 月）
            d("2026-07-04"), // 月备 #2（7 月）
            d("2026-06-02"), // 超出 → 删
            d("2026-05-03"), // 超出 → 删
        ];
        let keep = gfs_keep(&dates, 0, 0, 2);
        assert_eq!(keep, vec![true, true, false, false]);
    }

    #[test]
    fn gfs_empty() {
        assert!(gfs_keep(&[], 7, 4, 6).is_empty());
    }

    #[test]
    fn sha256_is_stable() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn date_parsing_from_file_name() {
        assert_eq!(
            date_of_file_name("voucher-backup-20260818-093000.zip"),
            Some(d("2026-08-18"))
        );
        assert_eq!(date_of_file_name("random.zip"), None);
    }
}
