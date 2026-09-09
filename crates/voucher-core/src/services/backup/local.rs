//! 本地备份链：run_backup 产出「快照 + 内容寻址附件包」两件套、备份列表、异盘双写、启动补传。

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::params;

use super::format::attachments_set_hash;
use super::retention::fs_retention;
use super::webdav::upload_backup_to_webdav;
use super::*;
use crate::db::repos::config_repo;
use crate::{CoreContext, CoreError, Result};

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
    // v0.14-A：加密口令前置检查（开启加密但无密码 → 立即报错，不产生明文备份）
    let enc_password = super::crypto::password_for_write(ctx)?;
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
        write_bundle_zip(
            ctx,
            &attachments,
            &bundle_path,
            &created_at,
            enc_password.as_deref(),
        )?;
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
        enc_password.as_deref(),
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
    fs_retention(&ctx.paths.backups, &ctx.config().backup)?;

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
/// v0.14-A：zip 内始终明文结构；`password` 非 None 时整文件再用 age 密封落盘
///（本地/异盘/云端共用此文件）。
fn write_bundle_zip(
    ctx: &CoreContext,
    attachments: &[(String, String)],
    dest: &Path,
    created_at: &str,
    password: Option<&str>,
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
    match password {
        // 加密：整文件 age 密封到 dest（salt 随机，每次密文不同）
        Some(p) => {
            super::crypto::seal_file(&tmp, dest, p)?;
            let _ = std::fs::remove_file(&tmp);
        }
        None => std::fs::rename(&tmp, dest)?,
    }
    Ok(())
}

/// 写快照（temp + rename 原子落位）：db + config + manifest（指向附件包名）。
/// checksums 只含自身条目（db/config），附件校验和在附件包 manifest 里。
/// v0.14-A：zip 内始终明文结构；`password` 非 None 时整文件再用 age 密封落盘。
fn write_snapshot_zip(
    ctx: &CoreContext,
    snap_db: &Path,
    bundle_name: &str,
    att_checksums: &HashMap<String, String>,
    dest: &Path,
    created_at: &str,
    password: Option<&str>,
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
    match password {
        Some(p) => {
            super::crypto::seal_file(&tmp, dest, p)?;
            let _ = std::fs::remove_file(&tmp);
        }
        None => std::fs::rename(&tmp, dest)?,
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

    // 次级目录同样 GFS 清理 + 孤儿附件包回收（与本地同一语义，见 retention::fs_retention）
    fs_retention(&dir, &ctx.config().backup)?;
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
