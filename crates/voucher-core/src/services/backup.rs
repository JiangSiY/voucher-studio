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
/// v3：云端增量结构（snapshot + 内容寻址附件包；v2 及以下为全量单文件，均可恢复）
const BACKUP_VERSION: u32 = 3;
pub const MANIFEST_FILE: &str = "manifest.json";
pub const DB_ENTRY: &str = "voucher.db";
pub const CONFIG_ENTRY: &str = "app.json";
pub const ATTACHMENTS_PREFIX: &str = "attachments/";
/// v0.8 云端快照文件名前缀（db+config+manifest，KB 级）
pub const SNAPSHOT_PREFIX: &str = "voucher-snapshot-";
/// v0.8 云端附件包文件名前缀（内容寻址：voucher-attachments-<hash8>.zip）
pub const BUNDLE_PREFIX: &str = "voucher-attachments-";

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
    /// v0.9：snapshot（快照，附件在共享附件包里）| full（全量单文件，含旧备份）
    #[serde(default)]
    pub kind: String,
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
    /// v3 快照指向的附件包文件名（仅云端增量结构的 snapshot 有；
    /// 全量备份 / 附件包自身为 None）
    #[serde(default)]
    attachments_bundle: Option<String>,
}

/// 立即执行一次备份：DB 快照 + 配置 + 附件 → zip → 本地 + 异盘 + WebDAV。
/// 手动与自动备份统一走这里，保证行为一致。
pub fn run_backup(ctx: &CoreContext) -> Result<BackupInfo> {
    run_backup_impl(ctx, false)
}

/// 关闭应用前的本地快速备份：只做 DB 快照 + 配置 + 附件 → zip → 本地 + GFS。
/// 跳过异盘双写和 WebDAV（两者可能慢且失败不影响本地主备份）。
/// 已在最近 60 分钟内备份过则跳过（幂等）。
pub fn run_local_backup(ctx: &CoreContext) -> Result<Option<BackupInfo>> {
    // 1. 最近备份检查：60 分钟内已有备份则跳过
    let recent = last_backup_within(ctx, 60 * 60);
    if recent {
        log::info!("本地备份跳过：60 分钟内已备份");
        return Ok(None);
    }
    // 2. 执行本地快速备份
    let info = run_backup_impl(ctx, true)?;
    Ok(Some(info))
}

/// 最近一次备份是否在 `seconds` 秒内
fn last_backup_within(ctx: &CoreContext, seconds: i64) -> bool {
    let last: Option<String> = ctx
        .db
        .with_conn(|conn| config_repo::get(conn, KEY_LAST_BACKUP))
        .ok()
        .flatten();
    if let Some(rfc) = last {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&rfc) {
            let diff = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc));
            return diff.num_seconds() >= 0 && diff.num_seconds() < seconds;
        }
    }
    false
}

/// 备份实现：`local_only` = 跳过异盘双写和 WebDAV 上传。
/// v0.8 起全程持 `backup_lock` 互斥——手动/定时/关闭前备份可能并发触发，
/// VACUUM INTO 写同一固定 temp 路径 + zip 文件名秒级时间戳可碰撞，串行化避免互相破坏。
///
/// v0.9 本地增量（与云端同构）：产物为「快照 + 内容寻址附件包」两件套：
/// - 快照（KB~MB）：db + config + manifest，每次新增
/// - 附件包：附件集合未变 → 文件名相同 → 已存在即跳过，本次 0 附件 I/O
///   （附件 hash 从快照库读，不重算、不读附件文件；只打包 DB 记录的原图，
///   增强版是可重建缓存不入备份）
///
/// 旧全量 voucher-backup-*.zip 兼容并存；恢复/演练经 materialize 合成，流程零改动。
fn run_backup_impl(ctx: &CoreContext, local_only: bool) -> Result<BackupInfo> {
    let _guard = ctx.backup_guard();
    // 1. 数据库一致性快照
    let snap = ctx.paths.temp.join("backup-snapshot.db");
    if snap.exists() {
        std::fs::remove_file(&snap)?;
    }
    ctx.db.with_conn(|conn| {
        conn.execute("VACUUM INTO ?1", params![snap.to_string_lossy()])?;
        Ok(())
    })?;

    // 2. 附件清单 + hash 从快照库读（= 备份时点 DB 状态，与 VACUUM 完全一致）
    let attachments = attachments_from_snapshot_db(&snap)?;
    // 附件是核心资产：文件缺失即中止（与旧全量行为一致，宁可不备份不备错的）
    for (rel, _) in &attachments {
        if !ctx.paths.attachments.join(rel).is_file() {
            return Err(CoreError::NotFound(format!(
                "附件文件缺失，备份中止：{rel}"
            )));
        }
    }
    let mut att_checksums: HashMap<String, String> = HashMap::new();
    for (rel, h) in &attachments {
        att_checksums.insert(format!("{ATTACHMENTS_PREFIX}{rel}"), h.clone());
    }

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let created_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // 3. 附件包：内容寻址，已存在即命中（0 附件 I/O——至此未读过任何附件文件）
    let bundle_name = if attachments.is_empty() {
        String::new()
    } else {
        format!(
            "{BUNDLE_PREFIX}{}.zip",
            &attachments_set_hash(&att_checksums)[..8]
        )
    };
    let bundle_path = ctx.paths.backups.join(&bundle_name);
    let has_attachments = !attachments.is_empty();
    let bundle_created = has_attachments && !bundle_path.exists();
    if bundle_created {
        write_bundle_zip(ctx, &attachments, &bundle_path, &created_at)?;
    }

    // 4. 快照：db + config + manifest（指向附件包名）。
    // 同秒再备份时加序号后缀（-2、-3…），避免覆盖既有快照——
    // 覆盖会让被顶掉的快照失去附件包引用，触发 GC 误回收。
    let mut snap_name = format!("{SNAPSHOT_PREFIX}{stamp}.zip");
    let mut seq = 1u32;
    while ctx.paths.backups.join(&snap_name).exists() {
        seq += 1;
        snap_name = format!("{SNAPSHOT_PREFIX}{stamp}-{seq}.zip");
    }
    let snap_path = ctx.paths.backups.join(&snap_name);
    write_snapshot_zip(
        ctx,
        &snap,
        &bundle_name,
        &att_checksums,
        &snap_path,
        &created_at,
    )?;
    let _ = std::fs::remove_file(&snap);

    // 5. 异盘双写 + WebDAV（关闭快速备份跳过这两步）
    let mut warnings = Vec::new();
    if !local_only {
        if let Err(e) =
            sync_backup_set_to_secondary(ctx, &snap_path, has_attachments.then_some(&bundle_path))
        {
            log::warn!("备用份目录写入失败: {e}");
            warnings.push(format!("异盘副本写入失败：{e}"));
        }
        if let Err(e) = upload_backup_to_webdav(ctx, &snap_path) {
            log::warn!("WebDAV 上传失败（本地备份不受影响）: {e}");
            warnings.push(format!("WebDAV 异地副本上传失败：{e}"));
        }
    }

    let info = BackupInfo {
        file_name: snap_name.clone(),
        path: snap_path.to_string_lossy().to_string(),
        size: std::fs::metadata(&snap_path).map(|m| m.len()).unwrap_or(0),
        created_at,
        attachment_count: attachments.len() as u32,
        warnings,
        kind: "snapshot".into(),
    };

    // 6. 本地 GFS 分层保留 + 孤儿附件包回收
    enforce_retention(ctx)?;

    // 7. 记录状态
    ctx.db.with_conn(|conn| {
        config_repo::set(conn, KEY_LAST_BACKUP, &chrono::Utc::now().to_rfc3339())
    })?;
    Ok(info)
}

/// 从 VACUUM 快照库读附件清单：(相对路径, sha256)，按路径排序（保证集合 hash 稳定）
fn attachments_from_snapshot_db(db: &Path) -> Result<Vec<(String, String)>> {
    let conn = crate::db::connection::open(db)?;
    let mut stmt =
        conn.prepare("SELECT file_path, file_hash FROM attachments ORDER BY file_path")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 写附件包（temp + rename 原子落位）：全部附件原图 + 自带 manifest。
/// 打包时逐文件读内容并算实际 hash（读了就算，manifest 与内容自洽可校验）。
fn write_bundle_zip(
    ctx: &CoreContext,
    attachments: &[(String, String)],
    dest: &Path,
    created_at: &str,
) -> Result<()> {
    let tmp = ctx.paths.temp.join(format!(
        "backup-bundle-{}",
        dest.file_name().and_then(|s| s.to_str()).unwrap_or("tmp")
    ));
    {
        let file = std::fs::File::create(&tmp)?;
        let mut w = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut checksums = HashMap::new();
        for (rel, _) in attachments {
            let bytes = std::fs::read(ctx.paths.attachments.join(rel))?;
            let entry = format!("{ATTACHMENTS_PREFIX}{rel}");
            checksums.insert(entry.clone(), sha256_hex(&bytes));
            w.start_file(entry.as_str(), opts)?;
            w.write_all(&bytes)?;
        }
        let manifest = Manifest {
            format: BACKUP_FORMAT.into(),
            version: BACKUP_VERSION,
            created_at: created_at.into(),
            files: checksums.keys().cloned().collect(),
            checksums,
            attachment_count: attachments.len() as u32,
            attachments_bundle: None,
        };
        w.start_file(MANIFEST_FILE, opts)?;
        w.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        w.finish()?;
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// 写快照（temp + rename 原子落位）：db + config + manifest（指向附件包名）。
/// checksums 只含自身条目（db/config），附件校验和在附件包 manifest 里。
fn write_snapshot_zip(
    ctx: &CoreContext,
    snap_db: &Path,
    bundle_name: &str,
    att_checksums: &HashMap<String, String>,
    dest: &Path,
    created_at: &str,
) -> Result<()> {
    let tmp = ctx.paths.temp.join(format!(
        "backup-snap-{}",
        dest.file_name().and_then(|s| s.to_str()).unwrap_or("tmp")
    ));
    {
        let file = std::fs::File::create(&tmp)?;
        let mut w = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut checksums = HashMap::new();

        let db_bytes = std::fs::read(snap_db)?;
        checksums.insert(DB_ENTRY.into(), sha256_hex(&db_bytes));
        w.start_file(DB_ENTRY, opts)?;
        w.write_all(&db_bytes)?;

        if ctx.paths.config_file().exists() {
            let cfg_bytes = std::fs::read(ctx.paths.config_file())?;
            checksums.insert(CONFIG_ENTRY.into(), sha256_hex(&cfg_bytes));
            w.start_file(CONFIG_ENTRY, opts)?;
            w.write_all(&cfg_bytes)?;
        }

        let manifest = Manifest {
            format: BACKUP_FORMAT.into(),
            version: BACKUP_VERSION,
            created_at: created_at.into(),
            files: checksums.keys().cloned().collect(),
            checksums,
            attachment_count: att_checksums.len() as u32,
            attachments_bundle: (!bundle_name.is_empty()).then(|| bundle_name.to_string()),
        };
        w.start_file(MANIFEST_FILE, opts)?;
        w.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        w.finish()?;
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
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
    let backups = list_backups(ctx)?; // 新→旧（快照+全量合并，bundle 已隐藏）
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

    // v0.9：孤儿附件包回收——被任一保留快照引用的留下，其余删除
    // （GFS 删过期快照后引用集合缩小，正好在这里收割）
    let kept: Vec<PathBuf> = keep_flags
        .iter()
        .enumerate()
        .filter(|(_, keep)| **keep)
        .map(|(pos, _)| {
            let (idx, _) = dated[pos];
            PathBuf::from(&backups[idx].path)
        })
        .collect();
    gc_orphan_bundles(&ctx.paths.backups, &kept)?;
    Ok(())
}

/// 回收孤儿附件包：`kept` 为保留的备份文件路径（快照读 manifest 引用，全量自含附件）。
/// 目录内未被引用的 voucher-attachments-*.zip 一律删除。
fn gc_orphan_bundles(dir: &Path, kept: &[PathBuf]) -> Result<()> {
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
    for p in kept {
        if let Some(m) = read_manifest(p) {
            if let Some(n) = m.attachments_bundle {
                referenced.insert(n);
            }
        }
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with(BUNDLE_PREFIX) && name.ends_with(".zip") && !referenced.contains(name) {
            let _ = std::fs::remove_file(&path);
            log::info!("回收孤儿附件包：{name}");
        }
    }
    Ok(())
}

/// v0.9 异盘双写：快照总是同步；附件包异盘缺失才同步（本盘命中但异盘被清空等场景）。
/// 异盘目录同样应用 GFS（快照+全量合并序列）与孤儿附件包回收，保持可独立恢复的完整集合。
fn sync_backup_set_to_secondary(
    ctx: &CoreContext,
    snap_path: &Path,
    bundle_path: Option<&Path>,
) -> Result<()> {
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
    copy_if_missing(snap_path, &dir)?;
    if let Some(b) = bundle_path {
        copy_if_missing(b, &dir)?;
    }

    // 次级目录同样 GFS 清理（快照 + 全量合并；引用保持的附件包不清理）
    let cfg = ctx.config().backup;
    let mut names: Vec<(NaiveDate, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if let Some(d) = date_of_file_name(name) {
            let is_backup = (name.starts_with("voucher-backup-")
                || name.starts_with(SNAPSHOT_PREFIX))
                && name.ends_with(".zip");
            if is_backup {
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
    let mut kept_paths: Vec<PathBuf> = Vec::new();
    for (i, keep_flag) in keep.iter().enumerate() {
        if *keep_flag {
            kept_paths.push(names[i].1.clone());
        } else {
            let _ = std::fs::remove_file(&names[i].1);
        }
    }
    // 异盘孤儿附件包回收（引用集合 = 异盘保留的快照）
    gc_orphan_bundles(&dir, &kept_paths)?;
    Ok(())
}

/// 复制到目标目录（同名已存在则跳过——快照名唯一，附件包内容寻址幂等）
fn copy_if_missing(src: &Path, dir: &Path) -> Result<()> {
    let dst = dir.join(src.file_name().unwrap_or_default());
    if dst.exists() {
        return Ok(());
    }
    // 先写临时再改名：中断不留半截文件污染集合
    let tmp = dir.join(format!(
        ".writing-{}",
        src.file_name().and_then(|s| s.to_str()).unwrap_or("tmp")
    ));
    std::fs::copy(src, &tmp)?;
    std::fs::rename(&tmp, &dst)?;
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
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        // v0.9：附件包是内部对象（由快照引用），不进备份列表
        if file_name.starts_with(BUNDLE_PREFIX) {
            continue;
        }
        let kind = if file_name.starts_with(SNAPSHOT_PREFIX) {
            "snapshot"
        } else {
            "full"
        }
        .to_string();
        let manifest = read_manifest(&path);
        out.push(BackupInfo {
            file_name,
            path: path.to_string_lossy().to_string(),
            size: entry.metadata().map(|m| m.len()).unwrap_or(0),
            created_at: manifest
                .as_ref()
                .map(|m| m.created_at.clone())
                .unwrap_or_default(),
            attachment_count: manifest.as_ref().map(|m| m.attachment_count).unwrap_or(0),
            warnings: Vec::new(),
            kind,
        });
    }
    // 新→旧：按文件名时间戳排序（快照与全量前缀不同，不能直接按文件名字典序）
    out.sort_by_key(|b| std::cmp::Reverse(b.stamp_key()));
    Ok(out)
}

impl BackupInfo {
    /// 排序键：文件名中的时间戳（提不出的排最后，保持稳定）
    fn stamp_key(&self) -> String {
        stamp_of_file_name(&self.file_name).unwrap_or_else(|| String::from("9999"))
    }
}

/// 从备份文件名提取时间戳段（voucher-{backup,snapshot}-YYYYMMDD-HHMMSS.zip → YYYYMMDD-HHMMSS）
fn stamp_of_file_name(name: &str) -> Option<String> {
    let digits: String = name.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 14 {
        Some(format!("{}-{}", &digits[..8], &digits[8..14]))
    } else {
        None
    }
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

// ---------- v0.9 materialize：快照 + 附件包 → 全量 zip（恢复/演练/校验透明复用） ----------

/// 把备份物化为全量 zip 路径：全量原样返回；快照则与同目录附件包合成到 temp（调用方用完删除）。
/// 附件包在 `path` 同目录查找（本地 backups/、异盘目录、外部拷入目录均成立）。
fn materialize_full_zip(ctx: &CoreContext, path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    if !name.starts_with(SNAPSHOT_PREFIX) {
        return Ok(path.to_path_buf()); // 全量（含旧备份）：原样
    }
    let manifest =
        read_manifest(path).ok_or_else(|| CoreError::Invalid("快照损坏：缺 manifest".into()))?;
    let bundle_name = match manifest.attachments_bundle {
        Some(b) => b,
        None => return Ok(path.to_path_buf()), // 无附件快照：自身即全量
    };
    let bundle_path = path.parent().unwrap_or(Path::new(".")).join(&bundle_name);
    if !bundle_path.is_file() {
        return Err(CoreError::Invalid(format!(
            "快照引用的附件包 {bundle_name} 缺失（与快照放同一目录），恢复中止"
        )));
    }
    let dest = ctx.paths.temp.join(format!(
        "materialized-{}",
        name.replacen(SNAPSHOT_PREFIX, "voucher-backup-", 1)
    ));
    compose_full_zip(path, &bundle_path, &dest)?;
    Ok(dest)
}

/// materialize 后执行 `f`，用完删除合成产物（全量原样路径不删）
fn with_materialized<T>(
    ctx: &CoreContext,
    path: &Path,
    f: impl FnOnce(&Path) -> Result<T>,
) -> Result<T> {
    let full = materialize_full_zip(ctx, path)?;
    let result = f(&full);
    if full != path {
        let _ = std::fs::remove_file(&full);
    }
    result
}

/// 校验备份（格式/版本/完整性），返回清单信息。
/// 快照会自动与附件包合成全量再校验（对调用方透明）。
pub fn inspect_backup(ctx: &CoreContext, zip_path: &Path) -> Result<RestoreSummary> {
    with_materialized(ctx, zip_path, inspect_backup_impl)
}

/// 校验全量 zip（格式/版本/完整性），返回清单信息（内部实现，入参必须是全量）
fn inspect_backup_impl(zip_path: &Path) -> Result<RestoreSummary> {
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

/// v0.9 导出完整备份：把快照 + 附件包合成单个全量 zip 落入 backups/
/// （供用户拷走单文件；全量入参原样拷贝重命名）。返回落盘路径。
pub fn export_full_backup(ctx: &CoreContext, source: &Path) -> Result<PathBuf> {
    let name = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let stamp = stamp_of_file_name(&name)
        .unwrap_or_else(|| chrono::Local::now().format("%Y%m%d-%H%M%S").to_string());
    let dest = ctx
        .paths
        .backups
        .join(format!("voucher-backup-export-{stamp}.zip"));
    with_materialized(ctx, source, |full| {
        std::fs::copy(full, &dest)?;
        Ok(())
    })?;
    log::info!("导出完整备份：{} → {}", source.display(), dest.display());
    Ok(dest)
}

/// 释放全量 zip（内部实现，入参必须是全量）
fn restore_backup_impl(zip_path: &Path, target_dir: &Path) -> Result<RestoreSummary> {
    let summary = inspect_backup_impl(zip_path)?;

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

// ---------- v0.8 云端增量结构：snapshot + 内容寻址附件包 ----------
//
// 本地全量 zip（v2 格式）保持不变（恢复链路零改动）；云端上传拆两个文件：
//   voucher-snapshot-<ts>.zip      db + config + manifest（KB 级，每次都传）
//   voucher-attachments-<hash8>.zip 全部附件（内容寻址：附件集合未变文件名相同，
//                                  远端已有即跳过 → 附件未变时每次 0 流量）
// 恢复时下载两文件在本地合成 v2 全量 zip，复用现有恢复流程。
// 云端旧的全量 voucher-backup-*.zip 仍可直接下载恢复（兼容并存）。

/// 附件集合指纹：对全部附件条目的 (entry, sha256) 排序后整体 sha256。
/// 附件未增未删未换 → 指纹不变 → 附件包文件名不变 → 云端命中跳过。
fn attachments_set_hash(checksums: &HashMap<String, String>) -> String {
    let mut pairs: Vec<(&String, &String)> = checksums
        .iter()
        .filter(|(k, _)| k.starts_with(ATTACHMENTS_PREFIX))
        .collect();
    pairs.sort();
    let mut input = String::new();
    for (k, v) in pairs {
        input.push_str(k);
        input.push('\n');
        input.push_str(v);
        input.push('\n');
    }
    sha256_hex(input.as_bytes())
}

/// 从本地全量 zip 拆出云端增量两件套。
/// 返回 (snapshot 路径, bundle 路径, bundle 文件名)——bundle 仅在附件非空时产生。
/// 拆出的文件写入 temp 目录，由调用方清理。
fn write_cloud_parts(
    full_zip: &Path,
    temp_dir: &Path,
) -> Result<(PathBuf, Option<PathBuf>, String)> {
    let file = std::fs::File::open(full_zip)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let manifest: Manifest = {
        let mut e = zip.by_name(MANIFEST_FILE)?;
        let mut buf = String::new();
        e.read_to_string(&mut buf)?;
        serde_json::from_str(&buf)?
    };

    let set_hash = attachments_set_hash(&manifest.checksums);
    let bundle_name = format!("{BUNDLE_PREFIX}{}.zip", &set_hash[..8]);
    let has_attachments = manifest.attachment_count > 0;

    // 1. 附件包：attachments/* 条目 + 自带 manifest（校验和仅附件条目）
    let bundle_path = if has_attachments {
        let p = temp_dir.join(&bundle_name);
        let out = std::fs::File::create(&p)?;
        let mut w = zip::ZipWriter::new(out);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut bundle_checksums = HashMap::new();
        for i in 0..zip.len() {
            let name = {
                let e = zip.by_index(i)?;
                e.name().to_string()
            };
            if !name.starts_with(ATTACHMENTS_PREFIX) || name.ends_with('/') {
                continue;
            }
            let mut buf = Vec::new();
            let mut e = zip.by_name(&name)?;
            e.read_to_end(&mut buf)?;
            bundle_checksums.insert(name.clone(), sha256_hex(&buf));
            w.start_file(name.as_str(), opts)?;
            w.write_all(&buf)?;
        }
        let bundle_manifest = Manifest {
            format: BACKUP_FORMAT.into(),
            version: BACKUP_VERSION,
            created_at: manifest.created_at.clone(),
            files: bundle_checksums.keys().cloned().collect(),
            checksums: bundle_checksums,
            attachment_count: manifest.attachment_count,
            attachments_bundle: None,
        };
        w.start_file(MANIFEST_FILE, opts)?;
        w.write_all(serde_json::to_string_pretty(&bundle_manifest)?.as_bytes())?;
        w.finish()?;
        Some(p)
    } else {
        None
    };

    // 2. 快照：db + config + v3 manifest（指向附件包名；校验和仅自身条目）
    let snap_path = temp_dir.join(format!(
        "{SNAPSHOT_PREFIX}{}.zip",
        date_time_of_name(
            full_zip
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
        )
    ));
    let out = std::fs::File::create(&snap_path)?;
    let mut w = zip::ZipWriter::new(out);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut snap_checksums = HashMap::new();
    for entry in [DB_ENTRY, CONFIG_ENTRY] {
        if let Ok(mut e) = zip.by_name(entry) {
            let mut buf = Vec::new();
            e.read_to_end(&mut buf)?;
            snap_checksums.insert(entry.to_string(), sha256_hex(&buf));
            w.start_file(entry, opts)?;
            w.write_all(&buf)?;
        }
    }
    let snap_manifest = Manifest {
        format: BACKUP_FORMAT.into(),
        version: BACKUP_VERSION,
        created_at: manifest.created_at.clone(),
        files: snap_checksums.keys().cloned().collect(),
        checksums: snap_checksums,
        attachment_count: manifest.attachment_count,
        attachments_bundle: if has_attachments {
            Some(bundle_name.clone())
        } else {
            None
        },
    };
    w.start_file(MANIFEST_FILE, opts)?;
    w.write_all(serde_json::to_string_pretty(&snap_manifest)?.as_bytes())?;
    w.finish()?;

    Ok((snap_path, bundle_path, bundle_name))
}

/// 从备份文件名提取「日期时间」段（voucher-backup-YYYYMMDD-HHMMSS.zip → YYYYMMDD-HHMMSS）。
/// 提不出时退化为当前时间（snapshot 名只需可排序唯一）。
fn date_time_of_name(name: &str) -> String {
    let digits: String = name
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(14)
        .collect();
    if digits.len() == 14 {
        format!("{}-{}", &digits[..8], &digits[8..])
    } else {
        chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
    }
}

/// 合成 v2 全量 zip（下载恢复链路）：snapshot 的 db/config + 附件包的 attachments。
/// 产物走现有 inspect/restore 流程，恢复逻辑零改动。
fn compose_full_zip(snapshot: &Path, bundle: &Path, dest: &Path) -> Result<()> {
    let snap_manifest: Manifest = read_manifest(snapshot)
        .ok_or_else(|| CoreError::Invalid("快照损坏：缺 manifest".into()))?;
    let bundle_manifest: Manifest = read_manifest(bundle)
        .ok_or_else(|| CoreError::Invalid("附件包损坏：缺 manifest".into()))?;

    let out = std::fs::File::create(dest)?;
    let mut w = zip::ZipWriter::new(out);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut merged = HashMap::new();
    // db + config 来自 snapshot
    {
        let file = std::fs::File::open(snapshot)?;
        let mut zip = zip::ZipArchive::new(file)?;
        for entry in [DB_ENTRY, CONFIG_ENTRY] {
            if let Ok(mut e) = zip.by_name(entry) {
                let mut buf = Vec::new();
                e.read_to_end(&mut buf)?;
                merged.insert(entry.to_string(), sha256_hex(&buf));
                w.start_file(entry, opts)?;
                w.write_all(&buf)?;
            }
        }
    }
    // attachments 来自 bundle
    {
        let file = std::fs::File::open(bundle)?;
        let mut zip = zip::ZipArchive::new(file)?;
        for i in 0..zip.len() {
            let name = {
                let e = zip.by_index(i)?;
                e.name().to_string()
            };
            if !name.starts_with(ATTACHMENTS_PREFIX) || name.ends_with('/') {
                continue;
            }
            let mut buf = Vec::new();
            let mut e = zip.by_name(&name)?;
            e.read_to_end(&mut buf)?;
            merged.insert(name.clone(), sha256_hex(&buf));
            w.start_file(name.as_str(), opts)?;
            w.write_all(&buf)?;
        }
    }

    let full_manifest = Manifest {
        format: BACKUP_FORMAT.into(),
        version: BACKUP_VERSION,
        created_at: snap_manifest.created_at.clone(),
        files: merged.keys().cloned().collect(),
        checksums: merged,
        attachment_count: bundle_manifest.attachment_count,
        attachments_bundle: None,
    };
    w.start_file(MANIFEST_FILE, opts)?;
    w.write_all(serde_json::to_string_pretty(&full_manifest)?.as_bytes())?;
    w.finish()?;
    Ok(())
}

/// 上传备份到 WebDAV（设置 → 备份与同步）；带指数退避重试，成功后清理远端超额备份。
/// 未启用时静默跳过（Ok）。
///
/// v0.8 增量：含附件的备份拆成 snapshot（KB 级，每次传）+ 附件包（内容寻址，
/// 附件未变则远端命中跳过，0 流量）；无附件 / 旧格式备份原样整文件上传。
/// v0.9 本地增量直连：入参已是快照时直接上传两件套，
/// 不再「先合成全量再拆分」（run_backup 产物零中转）。
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
    let base = webdav.url.trim().trim_end_matches('/').to_string();

    // v0.8 流量保护：当日云端已有备份则跳过（坚果云等免费服务上传配额有限；
    // snapshot 与旧全量都算「今日已有」）
    if webdav.once_daily {
        let today = chrono::Local::now().format("%Y%m%d").to_string();
        let uploaded_today = list_webdav_backups(ctx)
            .map(|rows| {
                rows.first().is_some_and(|r| {
                    r.file_name.starts_with(&format!("voucher-backup-{today}"))
                        || r.file_name
                            .starts_with(&format!("{SNAPSHOT_PREFIX}{today}"))
                })
            })
            .unwrap_or(false);
        if uploaded_today {
            log::info!("WebDAV 当日已有备份，跳过上传（流量保护）：{file_name}");
            return Ok(());
        }
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    // v0.9 分流：快照 → 直连上传（本地两件套已是云端结构）；
    // 全量 → 拆分两件套上传（旧备份 / 手动补传路径）
    if file_name.starts_with(SNAPSHOT_PREFIX) {
        upload_snapshot_direct(&client, &base, &webdav.username, &password, zip_path)?;
    } else {
        // 增量分流：有附件 → snapshot + 附件包；否则整文件上传（无附件备份本身极小）
        let incremental = read_manifest(zip_path)
            .map(|m| m.attachment_count > 0)
            .unwrap_or(false);
        if incremental {
            upload_incremental(ctx, &client, &base, &webdav.username, &password, zip_path)?;
        } else {
            let bytes = std::fs::read(zip_path)?;
            put_with_retry(
                &client,
                &format!("{base}/{file_name}"),
                &base,
                &webdav.username,
                &password,
                bytes,
                file_name,
            )?;
        }
    }

    // 远端保留清理
    if webdav.remote_retention > 0 {
        if let Err(e) = cleanup_webdav(
            &client,
            &base,
            &webdav.username,
            &password,
            webdav.remote_retention,
        ) {
            log::warn!("WebDAV 远端清理失败（不影响上传结果）: {e}");
        }
    }
    Ok(())
}

/// v0.9 快照直连上传：本地快照 + 同目录附件包（远端命中跳过）→ PUT。
/// 附件包本地缺失时报错跳过（避免上传出云端不可恢复的孤儿快照）。
fn upload_snapshot_direct(
    client: &reqwest::blocking::Client,
    base: &str,
    user: &str,
    password: &str,
    snap_path: &Path,
) -> Result<()> {
    let snap_name = snap_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let manifest = read_manifest(snap_path)
        .ok_or_else(|| CoreError::Invalid("快照损坏：缺 manifest".into()))?;

    if let Some(bundle_name) = manifest.attachments_bundle {
        let bundle_path = snap_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(&bundle_name);
        let existing = propfind_all_names(client, base, user, password)?;
        if existing.contains(&bundle_name) {
            log::info!("附件集合未变，云端命中附件包（0 附件流量）：{bundle_name}");
        } else if bundle_path.is_file() {
            let bytes = std::fs::read(&bundle_path)?;
            let size_mb = bytes.len() as f64 / 1024.0 / 1024.0;
            log::info!("上传附件包 {bundle_name}（{size_mb:.1} MB，附件集合已变化）");
            put_with_retry(
                client,
                &format!("{base}/{}", url_escape(&bundle_name)),
                base,
                user,
                password,
                bytes,
                &bundle_name,
            )?;
        } else {
            return Err(CoreError::Invalid(format!(
                "本地附件包 {bundle_name} 缺失，快照 {snap_name} 不完整，已跳过上传"
            )));
        }
    }

    // 快照：KB 级，每次都传
    let bytes = std::fs::read(snap_path)?;
    put_with_retry(
        client,
        &format!("{base}/{}", url_escape(&snap_name)),
        base,
        user,
        password,
        bytes,
        &snap_name,
    )?;
    log::info!("云端快照直连上传完成：{snap_name}");
    Ok(())
}

/// v0.8 增量上传：全量 zip → 拆 snapshot + 附件包 → 附件包远端命中则跳过 → PUT snapshot。
fn upload_incremental(
    ctx: &CoreContext,
    client: &reqwest::blocking::Client,
    base: &str,
    user: &str,
    password: &str,
    full_zip: &Path,
) -> Result<()> {
    let temp_dir = ctx.paths.temp.join("cloud-parts");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir)?;
    let result = (|| {
        let (snap_path, bundle_path, bundle_name) = write_cloud_parts(full_zip, &temp_dir)?;
        let snap_name = snap_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        // 附件包：内容寻址命中（远端已有同名）→ 跳过，本次上传附件 0 流量
        let existing = propfind_all_names(client, base, user, password)?;
        if existing.contains(&bundle_name) {
            log::info!("附件集合未变，云端命中附件包（0 附件流量）：{bundle_name}");
        } else if let Some(bp) = &bundle_path {
            let bytes = std::fs::read(bp)?;
            let size_mb = bytes.len() as f64 / 1024.0 / 1024.0;
            log::info!("上传附件包 {bundle_name}（{size_mb:.1} MB，附件集合已变化）");
            put_with_retry(
                client,
                &format!("{base}/{}", url_escape(&bundle_name)),
                base,
                user,
                password,
                bytes,
                &bundle_name,
            )?;
        }

        // snapshot：KB 级，每次都传
        let bytes = std::fs::read(&snap_path)?;
        put_with_retry(
            client,
            &format!("{base}/{}", url_escape(&snap_name)),
            base,
            user,
            password,
            bytes,
            &snap_name,
        )?;
        log::info!("云端增量上传完成：{snap_name}");
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&temp_dir);
    result
}

/// PUT + 重试（v0.8 修正的重试策略）：
/// - 401/403/507：认证 / 权限 / 存储配额——确定性失败，立即报错
/// - 404/409：目标目录不存在 → 逐级 MKCOL 创建，成功后跳过退避立即重试
/// - 网络错误 / 5xx：2s / 4s 指数退避重试
#[allow(clippy::too_many_arguments)]
fn put_with_retry(
    client: &reqwest::blocking::Client,
    url: &str,
    base: &str,
    user: &str,
    password: &str,
    bytes: Vec<u8>,
    file_name: &str,
) -> Result<()> {
    let mut last_err = None;
    let mut skip_next_delay = false;
    for attempt in 0..UPLOAD_ATTEMPTS {
        if attempt > 0 {
            if skip_next_delay {
                skip_next_delay = false;
                log::info!("WebDAV 目录已建，立即重试 PUT: {file_name}");
            } else {
                let delay = 2u64.pow(attempt); // 2s, 4s
                std::thread::sleep(std::time::Duration::from_secs(delay));
                log::info!("WebDAV 上传重试 #{attempt}: {file_name}");
            }
        }
        match client
            .put(url)
            .basic_auth(user, Some(password))
            .body(bytes.clone())
            .send()
        {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) => {
                let status = resp.status();
                // 确定性失败：不重试（重试只会浪费时间并掩盖真实原因）
                if matches!(status.as_u16(), 401 | 403 | 507) {
                    return Err(CoreError::Invalid(format!(
                        "WebDAV 返回 {status}（认证/权限/配额问题，请检查账号与服务器设置）"
                    )));
                }
                // 目录不存在：逐级创建后立即重试
                if (status == reqwest::StatusCode::NOT_FOUND
                    || status == reqwest::StatusCode::CONFLICT)
                    && mkcol_all(client, base, user, password)
                {
                    skip_next_delay = true;
                }
                last_err = Some(CoreError::Invalid(format!("WebDAV 返回 {status}")));
            }
            Err(e) => last_err = Some(CoreError::Http(e)),
        }
    }
    Err(last_err.unwrap_or_else(|| CoreError::Invalid("WebDAV 上传失败".into())))
}

/// 逐级 MKCOL 创建目录路径（如 `https://host/dav/a/b/c` → 依次 MKCOL
/// `/dav/a`、`/dav/a/b`、`/dav/a/b/c`）。已存在（405）视为成功；
/// 单级失败返回 false（中间目录缺失时单级 MKCOL 必然 409，逐级才能真正建出深路径）。
fn mkcol_all(client: &reqwest::blocking::Client, base: &str, user: &str, password: &str) -> bool {
    let Some(scheme_at) = base.find("://") else {
        return false;
    };
    let Some(path_at) = base[scheme_at + 3..].find('/') else {
        return true; // base 无路径（主机根），无需创建
    };
    let root = &base[..scheme_at + 3 + path_at]; // scheme://host
    let mut cur = root.to_string();
    for seg in base[scheme_at + 3 + path_at..]
        .split('/')
        .filter(|s| !s.is_empty())
    {
        cur = format!("{cur}/{seg}");
        let r = client
            .request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), &cur)
            .basic_auth(user, Some(password))
            .send();
        match r {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 405 => continue,
            Ok(resp) => {
                log::warn!("WebDAV MKCOL {cur} 返回 {}", resp.status());
                return false;
            }
            Err(e) => {
                log::warn!("WebDAV MKCOL 请求失败: {e}");
                return false;
            }
        }
    }
    log::info!("WebDAV 目标目录已就绪: {base}");
    true
}

/// v0.8：补传「本地有、云端无」的备份（应用启动时后台执行）。
///
/// 修复场景：某次备份时 WebDAV 上传失败（仅记 warning），本地 zip 一直在，
/// 但此后没有机制再传——远端永远缺这一份。补传在启动时跑（趁本地 GFS
/// 清理把旧备份删掉之前）。WebDAV 未启用 / 网络不通时静默跳过。
/// 返回补传成功数。
pub fn upload_missing_backups(ctx: &CoreContext) -> Result<usize> {
    let cfg = ctx.config();
    if !cfg.backup.webdav.enabled || cfg.backup.webdav.url.trim().is_empty() {
        return Ok(0);
    }
    let remote = match list_webdav_backups(ctx) {
        Ok(rows) => rows,
        Err(e) => {
            // 启动时网络未就绪很常见——静默跳过，不制造启动告警噪音
            log::info!("启动补传检查跳过（远端列表获取失败：{e}）");
            return Ok(0);
        }
    };
    let remote_names: std::collections::HashSet<String> =
        remote.into_iter().map(|r| r.file_name).collect();
    let local = list_backups(ctx)?;
    let _guard = ctx.backup_guard();
    let mut ok = 0usize;
    for b in &local {
        if remote_names.contains(&b.file_name) {
            continue;
        }
        // 只补传本程序命名的备份（外部拷入的 zip 不上云）
        if !is_valid_remote_name(&b.file_name) {
            continue;
        }
        match upload_backup_to_webdav(ctx, Path::new(&b.path)) {
            Ok(()) => {
                ok += 1;
                log::info!("补传备份到云端：{}", b.file_name);
            }
            Err(e) => log::warn!("补传统份失败（{}）: {e}", b.file_name),
        }
    }
    if ok > 0 {
        log::info!("启动补传完成：{ok} 份");
    }
    Ok(ok)
}

/// 远端保留清理（v0.8 增量适配）：
/// - 可恢复备份（旧全量 voucher-backup-* + 快照 voucher-snapshot-*）合并按新→旧保留 `keep` 份
/// - 附件包（voucher-attachments-*）：被任一保留快照引用的留下，其余回收
///   （引用关系读快照 manifest——KB 级 GET，成本可忽略）
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
    let mut names = parse_backup_hrefs(&body);
    names.sort();
    names.dedup();

    // 1. 可恢复备份（全量 + 快照）合并保留
    let restorable: Vec<String> = names
        .iter()
        .filter(|n| !n.starts_with(BUNDLE_PREFIX))
        .cloned()
        .collect();
    let excess = restorable.len().saturating_sub(keep as usize);
    for name in &restorable[..excess] {
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
    let kept: Vec<&String> = restorable.iter().skip(excess).collect();

    // 2. 附件包回收：收集保留快照引用的 bundle 名，未引用的删
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
    for name in &kept {
        if !name.starts_with(SNAPSHOT_PREFIX) {
            continue; // 旧全量自含附件，不引用 bundle
        }
        if let Ok(bytes) = webdav_get(client, base, user, password, name) {
            if let Some(m) = parse_manifest_bytes(&bytes) {
                if let Some(b) = m.attachments_bundle {
                    referenced.insert(b);
                }
            }
        }
    }
    for name in names.iter().filter(|n| n.starts_with(BUNDLE_PREFIX)) {
        if referenced.contains(name) {
            continue;
        }
        let del_url = format!("{base}/{}", url_escape(name));
        let status = client
            .delete(&del_url)
            .basic_auth(user, Some(password))
            .send()?
            .status();
        if !status.is_success() {
            log::warn!("WebDAV 回收附件包 {name} 返回 {status}");
        }
    }
    Ok(())
}

/// GET 远端文件到内存（快照 manifest 读取用，KB 级）
fn webdav_get(
    client: &reqwest::blocking::Client,
    base: &str,
    user: &str,
    password: &str,
    name: &str,
) -> Result<Vec<u8>> {
    let url = format!("{base}/{}", url_escape(name));
    let resp = client
        .get(&url)
        .basic_auth(user, Some(password))
        .send()?
        .error_for_status()?;
    Ok(resp.bytes()?.to_vec())
}

/// PROPFIND base 并返回全部本程序远端文件名（三类：全量/快照/附件包；
/// 增量上传判附件包是否已存在用）。
fn propfind_all_names(
    client: &reqwest::blocking::Client,
    base: &str,
    user: &str,
    password: &str,
) -> Result<Vec<String>> {
    let resp = client
        .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), base)
        .basic_auth(user, Some(password))
        .header("Depth", "1")
        .send()?
        .error_for_status()?;
    let body = resp.text()?;
    Ok(parse_backup_hrefs(&body))
}

/// 从 zip 字节流解析 manifest（远端快照引用读取用）
fn parse_manifest_bytes(bytes: &[u8]) -> Option<Manifest> {
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;
    let mut entry = zip.by_name(MANIFEST_FILE).ok()?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}

/// 从 PROPFIND 响应体解析备份文件名：提取所有 href，过滤 voucher-backup-*.zip
/// （排除目录自身），URL 解码后去重返回（不排序，由调用方决定序）。
fn parse_backup_hrefs(body: &str) -> Vec<String> {
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
            if is_valid_remote_name(name) {
                // URL 解码 %XX（Nextcloud 常见）
                let decoded = percent_decode(name);
                names.push(decoded);
            }
        }
        pos = content_end;
    }
    names
}

/// 远端备份文件名白名单（v0.8 增量适配）：三类 zip 且不含路径分隔符/..
/// （下载入参校验用，防路径穿越；名字含时间戳或内容哈希，天然可排序）
fn is_valid_remote_name(name: &str) -> bool {
    (name.starts_with("voucher-backup-")
        || name.starts_with(SNAPSHOT_PREFIX)
        || name.starts_with(BUNDLE_PREFIX))
        && name.ends_with(".zip")
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
}

// ---------- WebDAV 云备份闭环：连接测试 / 远端列表 / 下载 ----------

/// 远端备份行（PROPFIND 解析结果）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebdavBackupRow {
    pub file_name: String,
}

/// WebDAV 连接参数聚合（启用 + url + 凭据齐备才可用）
struct WebdavTarget {
    client: reqwest::blocking::Client,
    base: String,
    username: String,
    password: String,
}

/// 校验配置并构造带认证的客户端；未启用/缺配置时返回 Invalid
fn webdav_target(ctx: &CoreContext) -> Result<WebdavTarget> {
    let cfg = ctx.config();
    let webdav = &cfg.backup.webdav;
    if !webdav.enabled || webdav.url.trim().is_empty() {
        return Err(CoreError::Invalid(
            "WebDAV 未启用或地址为空（设置 → 备份与同步）".into(),
        ));
    }
    let password = ctx
        .secrets
        .get(SECRET_WEBDAV_PASSWORD)?
        .ok_or_else(|| CoreError::Invalid("WebDAV 密码未设置（密钥管理）".into()))?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    Ok(WebdavTarget {
        client,
        base: webdav.url.trim().trim_end_matches('/').to_string(),
        username: webdav.username.clone(),
        password,
    })
}

/// PROPFIND base 目录并解析备份文件名（连接测试/远端列表共用）。
/// 目录不存在时给出明确提示（认证已通过；首次备份上传时会自动 MKCOL 创建）。
fn propfind_backup_names(t: &WebdavTarget) -> Result<Vec<String>> {
    let resp = t
        .client
        .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &t.base)
        .basic_auth(&t.username, Some(&t.password))
        .header("Depth", "1")
        .send()?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(CoreError::Invalid(format!(
            "认证成功但目录不存在：{}（首次备份上传时会自动创建）",
            t.base
        )));
    }
    let body = resp.error_for_status()?.text()?;
    Ok(parse_backup_hrefs(&body))
}

/// 连接测试：PROPFIND 根目录，返回远端已有备份数（配置错误/凭据错误/网络不通都会报错）
pub fn test_webdav(ctx: &CoreContext) -> Result<u32> {
    let t = webdav_target(ctx)?;
    Ok(propfind_backup_names(&t)?.len() as u32)
}

/// 远端备份列表（新 → 旧）。v0.8 增量适配：只列「可恢复」备份
/// （旧全量 + 快照）；附件包是内部件（由快照引用），不进列表。
pub fn list_webdav_backups(ctx: &CoreContext) -> Result<Vec<WebdavBackupRow>> {
    let t = webdav_target(ctx)?;
    let mut names = propfind_backup_names(&t)?;
    names.retain(|n| !n.starts_with(BUNDLE_PREFIX));
    names.sort();
    names.dedup();
    names.reverse(); // 新 → 旧
    Ok(names
        .into_iter()
        .map(|file_name| WebdavBackupRow { file_name })
        .collect())
}

/// 下载远端备份到本地 backups/ 目录（换机恢复链路：云端列表 → 下载 → 本地恢复）。
/// 返回落地的本地路径。同名本地备份直接跳过下载（幂等）。
/// v0.8 增量适配：下载快照时自动拉取其引用的附件包并合成 v2 全量 zip（复用现有恢复流程）。
pub fn download_webdav_backup(ctx: &CoreContext, file_name: &str) -> Result<String> {
    if !is_valid_remote_name(file_name) {
        return Err(CoreError::Invalid(format!("非法的远端备份名：{file_name}")));
    }
    if file_name.starts_with(BUNDLE_PREFIX) {
        return Err(CoreError::Invalid(
            "附件包是内部文件（由快照引用），请下载对应的快照".into(),
        ));
    }
    let t = webdav_target(ctx)?;

    // 增量快照：下载 snapshot + 引用的 bundle → 合成全量（落地名为 voucher-backup-*，与本地列表口径一致）
    if file_name.starts_with(SNAPSHOT_PREFIX) {
        let snap_bytes = webdav_get(&t.client, &t.base, &t.username, &t.password, file_name)?;
        if snap_bytes.is_empty() {
            return Err(CoreError::Invalid("远端返回空文件（备份可能损坏）".into()));
        }
        let bundle_name = parse_manifest_bytes(&snap_bytes)
            .and_then(|m| m.attachments_bundle)
            .ok_or_else(|| CoreError::Invalid("快照缺少附件包引用（格式异常）".into()))?;
        let bundle_bytes = webdav_get(&t.client, &t.base, &t.username, &t.password, &bundle_name)?;
        if bundle_bytes.is_empty() {
            return Err(CoreError::Invalid(format!(
                "附件包 {bundle_name} 下载为空（可能已被清理，请换一份备份）"
            )));
        }
        let full_name = format!(
            "voucher-backup-{}",
            file_name.trim_start_matches(SNAPSHOT_PREFIX)
        );
        let dest = ctx.paths.backups.join(&full_name);
        if dest.exists() {
            return Ok(dest.to_string_lossy().into_owned()); // 幂等
        }
        let tmp_snap = ctx.paths.temp.join("webdav-dl-snapshot.zip");
        let tmp_bundle = ctx.paths.temp.join("webdav-dl-bundle.zip");
        let tmp_full = ctx.paths.temp.join(format!("webdav-compose-{full_name}"));
        std::fs::write(&tmp_snap, &snap_bytes)?;
        std::fs::write(&tmp_bundle, &bundle_bytes)?;
        compose_full_zip(&tmp_snap, &tmp_bundle, &tmp_full)?;
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::rename(&tmp_full, &dest)?;
        let _ = std::fs::remove_file(&tmp_snap);
        let _ = std::fs::remove_file(&tmp_bundle);
        log::info!("WebDAV 快照合成下载完成：{file_name} + {bundle_name} → {full_name}");
        return Ok(dest.to_string_lossy().into_owned());
    }

    // 旧全量：直接下载
    let url = format!("{}/{}", t.base, url_escape(file_name));
    let dest = ctx.paths.backups.join(file_name);
    if dest.exists() {
        return Ok(dest.to_string_lossy().into_owned()); // 已下载过：幂等返回
    }

    let resp = t
        .client
        .get(&url)
        .basic_auth(&t.username, Some(&t.password))
        .send()?
        .error_for_status()?;
    let bytes = resp.bytes()?;
    if bytes.is_empty() {
        return Err(CoreError::Invalid("远端返回空文件（备份可能损坏）".into()));
    }

    // 先写临时文件再改名：下载中断不产生半截备份污染本地列表
    let tmp = ctx.paths.temp.join(format!("webdav-dl-{file_name}"));
    std::fs::write(&tmp, &bytes)?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::rename(&tmp, &dest)?;
    log::info!("WebDAV 下载完成：{file_name} → {}", dest.display());
    Ok(dest.to_string_lossy().into_owned())
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

    // ---------- WebDAV 云备份闭环 ----------

    #[test]
    fn parse_backup_hrefs_filters_dirs_and_foreign_files() {
        // Nextcloud 风格 PROPFIND：目录自身（尾 /）+ 两个备份 + 混入文件
        let body = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
  <d:response><d:href>/remote.php/dav/files/user/voucher/</d:href></d:response>
  <d:response><d:href>/remote.php/dav/files/user/voucher/voucher-backup-20260818-093000.zip</d:href></d:response>
  <d:response><d:href>/remote.php/dav/files/user/voucher/voucher-backup-20260817-120000.zip</d:href></d:response>
  <d:response><d:href>/remote.php/dav/files/user/voucher/readme.txt</d:href></d:response>
</d:multistatus>"#;
        let mut names = parse_backup_hrefs(body);
        names.sort();
        assert_eq!(
            names,
            vec![
                "voucher-backup-20260817-120000.zip".to_string(),
                "voucher-backup-20260818-093000.zip".to_string(),
            ]
        );
    }

    #[test]
    fn parse_backup_hrefs_decodes_percent_names() {
        // 坚果云/Alist 常见 %XX 编码文件名：解码后再返回
        let body = "<d:href>/dav/voucher-backup-2026%2E08%2E18.zip</d:href>";
        let names = parse_backup_hrefs(body);
        assert_eq!(names, vec!["voucher-backup-2026.08.18.zip".to_string()]);
    }

    #[test]
    fn remote_name_whitelist_blocks_traversal() {
        assert!(is_valid_remote_name("voucher-backup-20260818-093000.zip"));
        assert!(!is_valid_remote_name("../voucher-backup-x.zip")); // 路径穿越
        assert!(!is_valid_remote_name("sub/dir/voucher-backup-x.zip"));
        assert!(!is_valid_remote_name("voucher-backup-x.exe")); // 非备份扩展名
        assert!(!is_valid_remote_name("notes.txt"));
        assert!(!is_valid_remote_name(""));
    }

    #[test]
    fn url_escape_and_percent_decode_roundtrip() {
        assert_eq!(
            url_escape("voucher-backup-2026.zip"),
            "voucher-backup-2026.zip"
        );
        assert_eq!(url_escape("a b&c"), "a%20b%26c");
        assert_eq!(percent_decode("2026%2E08%2E18"), "2026.08.18");
        // 尾部不完整 %XX：原样保留
        assert_eq!(percent_decode("100%"), "100%");
    }

    // ---------- v0.8 云端增量结构 ----------

    #[test]
    fn remote_name_whitelist_accepts_incremental_files() {
        assert!(is_valid_remote_name("voucher-snapshot-20260818-093000.zip"));
        assert!(is_valid_remote_name("voucher-attachments-a1b2c3d4.zip"));
        assert!(!is_valid_remote_name("voucher-attachments-../evil.zip"));
        assert!(!is_valid_remote_name("voucher-snapshot-x.exe"));
    }

    #[test]
    fn attachments_set_hash_order_insensitive() {
        // 集合指纹与条目顺序无关（HashMap 无序输入 → 同一指纹）
        let a: HashMap<String, String> = [
            ("attachments/2026/08/a.jpg", "hash-a"),
            ("attachments/2026/08/b.jpg", "hash-b"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let b: HashMap<String, String> = [
            ("attachments/2026/08/b.jpg", "hash-b"),
            ("attachments/2026/08/a.jpg", "hash-a"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(attachments_set_hash(&a), attachments_set_hash(&b));
        // 任一附件 hash 变化 → 指纹变化（附件包文件名变化 → 重新上传）
        let c: HashMap<String, String> = [(
            "attachments/2026/08/a.jpg",
            "hash-a2", // 内容变了
        )]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_ne!(attachments_set_hash(&a), attachments_set_hash(&c));
    }

    /// 增量拆包 + 合成 roundtrip：全量 zip → snapshot + bundle → 合成 →
    /// inspect 校验通过 + 附件数与条目集合一致（恢复链路语义等价）
    #[test]
    fn cloud_parts_split_and_compose_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let full = dir.path().join("voucher-backup-20260818-093000.zip");

        // 真实 SQLite 快照（restore_backup 校验库可打开）
        let db_file = dir.path().join("voucher.db");
        {
            let conn = rusqlite::Connection::open(&db_file).unwrap();
            conn.execute_batch("CREATE TABLE vouchers (id INTEGER PRIMARY KEY);")
                .unwrap();
        }
        let db_bytes = std::fs::read(&db_file).unwrap();

        // 构造小型 v2 全量 zip：db + config + 2 附件
        {
            let out = std::fs::File::create(&full).unwrap();
            let mut w = zip::ZipWriter::new(out);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            let mut checksums = HashMap::new();
            let entries: Vec<(&str, Vec<u8>)> = vec![
                (DB_ENTRY, db_bytes),
                (CONFIG_ENTRY, b"config-bytes".to_vec()),
                ("attachments/2026/08/a.jpg", b"image-a".to_vec()),
                ("attachments/2026/08/b.jpg", b"image-b".to_vec()),
            ];
            for (name, bytes) in &entries {
                checksums.insert(name.to_string(), sha256_hex(bytes));
                w.start_file(*name, opts).unwrap();
                w.write_all(bytes).unwrap();
            }
            let manifest = Manifest {
                format: BACKUP_FORMAT.into(),
                version: BACKUP_VERSION,
                created_at: "2026-08-18 09:30:00".into(),
                files: checksums.keys().cloned().collect(),
                checksums,
                attachment_count: 2,
                attachments_bundle: None,
            };
            w.start_file(MANIFEST_FILE, opts).unwrap();
            w.write_all(serde_json::to_string_pretty(&manifest).unwrap().as_bytes())
                .unwrap();
            w.finish().unwrap();
        }

        // 拆包：snapshot + bundle
        let parts_dir = dir.path().join("parts");
        std::fs::create_dir_all(&parts_dir).unwrap();
        let (snap, bundle, bundle_name) = write_cloud_parts(&full, &parts_dir).unwrap();
        // 调试：snapshot 内 db 条目长度
        {
            let f = std::fs::File::open(&snap).unwrap();
            let mut z = zip::ZipArchive::new(f).unwrap();
            let mut e = z.by_name(DB_ENTRY).unwrap();
            use std::io::Read;
            let mut buf = Vec::new();
            e.read_to_end(&mut buf).unwrap();
            println!("snapshot db entry len: {}", buf.len());
        }
        assert!(bundle.is_some(), "含附件备份应产生附件包");
        assert!(bundle_name.starts_with(BUNDLE_PREFIX));
        assert_eq!(
            bundle_name.len(),
            BUNDLE_PREFIX.len() + 8 + 4,
            "附件包名 = 前缀 + hash8 + .zip"
        );
        assert!(
            snap.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(SNAPSHOT_PREFIX),
            "快照名前缀"
        );

        // 合成：v2 全量等价物
        let composed = dir.path().join("composed.zip");
        compose_full_zip(&snap, bundle.as_ref().unwrap(), &composed).unwrap();

        // inspect 通过（校验和全对）+ 附件数 2
        let summary = inspect_backup_impl(&composed).unwrap();
        assert_eq!(summary.attachment_count, 2);
        assert!(summary.checksums_verified);
        // 条目集合 = db + config + 2 附件
        assert_eq!(summary.restored_files.len(), 4);
        assert!(summary
            .restored_files
            .contains(&"attachments/2026/08/b.jpg".into()));

        // 合成产物可恢复（restore_backup 走通 = 恢复链路等价）
        let restore_dir = dir.path().join("restore");
        let restored = restore_backup_impl(&composed, &restore_dir).unwrap();
        assert_eq!(restored.attachment_count, 2);
        assert!(restore_dir
            .join("attachments")
            .join("2026/08/b.jpg")
            .exists());
    }

    #[test]
    fn snapshot_bundle_names_parse() {
        assert_eq!(
            date_time_of_name("voucher-backup-20260818-093000.zip"),
            "20260818-093000"
        );
        // 提不出时间戳 → fallback 当前时间（格式 YYYYMMDD-HHMMSS，不跨断言取两次时间）
        let fallback = date_time_of_name("weird");
        assert_eq!(fallback.len(), 15);
        assert!(fallback.contains('-'));
    }
}
