//! 备份格式：快照/附件包的构造（write_cloud_parts）、合成（compose/materialize）、内容寻址指纹。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::*;
use crate::{CoreContext, CoreError, Result};
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
        return Ok(path.to_path_buf()); // 全量（含旧备份）：原样（age 密封由 with_materialized 兜底）
    }
    // v0.14-A：快照可能是 age 信封（v4）→ 先解密到 ctx temp，后续 zip 读取针对
    // 明文 zip；v0.13 加密 zip（条目级 AES）不是信封，由 open_entry 兼容解密。
    let sealed = super::crypto::is_age_envelope(path);
    let snap_plain = if sealed {
        super::crypto::unseal_to_temp_dir(path, &ctx.paths.temp)?
    } else {
        path.to_path_buf()
    };
    let manifest = match read_manifest(&snap_plain) {
        Some(m) => m,
        None => {
            if sealed {
                let _ = std::fs::remove_file(&snap_plain);
            }
            return Err(CoreError::Invalid("快照损坏：缺 manifest".into()));
        }
    };
    let bundle_name = match manifest.attachments_bundle {
        Some(b) => b,
        None => return Ok(snap_plain), // 无附件快照：自身即全量（age 已解密到 temp，调用方清理）
    };
    let bundle_path = path.parent().unwrap_or(Path::new(".")).join(&bundle_name);
    if !bundle_path.is_file() {
        if sealed {
            let _ = std::fs::remove_file(&snap_plain);
        }
        return Err(CoreError::Invalid(format!(
            "快照引用的附件包 {bundle_name} 缺失（与快照放同一目录），恢复中止"
        )));
    }
    let dest = ctx.paths.temp.join(format!(
        "materialized-{}",
        name.replacen(SNAPSHOT_PREFIX, "voucher-backup-", 1)
    ));
    let res = compose_full_zip(&snap_plain, &bundle_path, &dest);
    if sealed {
        let _ = std::fs::remove_file(&snap_plain);
    }
    res?;
    Ok(dest)
}

/// materialize 后执行 `f`，用完删除合成产物（全量原样路径不删）。
/// v0.14-A 兜底：非快照名的备份文件若本身是 age 信封（v4 全量），先解密到 temp
/// 再执行 `f`，用后连同解密临时文件一起清理。
pub(super) fn with_materialized<T>(
    ctx: &CoreContext,
    path: &Path,
    f: impl FnOnce(&Path) -> Result<T>,
) -> Result<T> {
    let full = materialize_full_zip(ctx, path)?;
    let result = (|| {
        let plain = if super::crypto::is_age_envelope(&full) {
            Some(super::crypto::unseal_to_temp_dir(&full, &ctx.paths.temp)?)
        } else {
            None
        };
        let r = match &plain {
            Some(p) => f(p),
            None => f(&full),
        };
        if let Some(p) = &plain {
            let _ = std::fs::remove_file(p);
        }
        r
    })();
    if full != path {
        let _ = std::fs::remove_file(&full);
    }
    result
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
pub(super) fn attachments_set_hash(checksums: &HashMap<String, String>) -> String {
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
pub(super) fn write_cloud_parts(
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
/// v0.14-A：入参可能是 age 信封（v4 整文件加密）或旧 v0.13 加密 zip（条目级 AES）——
/// age 信封先解密到临时明文 zip 再合成（用后清理）；条目级 AES 由 crypto::open_entry
/// 解密；产物保持明文（临时中间物）。
pub(super) fn compose_full_zip(snapshot: &Path, bundle: &Path, dest: &Path) -> Result<()> {
    super::crypto::with_readable_zip(snapshot, |snap_plain| {
        super::crypto::with_readable_zip(bundle, |bundle_plain| {
            compose_from_plain(snap_plain, bundle_plain, dest)
        })
    })
}

/// 在「已是明文 zip（或旧条目级 AES zip）」的 snapshot/bundle 路径上执行合成。
/// snapshot/bundle 的 manifest 读取与条目解密统一走 crypto::open_entry。
fn compose_from_plain(snapshot: &Path, bundle: &Path, dest: &Path) -> Result<()> {
    let snap_manifest: Manifest = read_manifest(snapshot)
        .ok_or_else(|| CoreError::Invalid("快照损坏：缺 manifest".into()))?;
    let bundle_manifest: Manifest = read_manifest(bundle)
        .ok_or_else(|| CoreError::Invalid("附件包损坏：缺 manifest".into()))?;

    let out = std::fs::File::create(dest)?;
    let mut w = zip::ZipWriter::new(out);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut merged = HashMap::new();
    // db + config 来自 snapshot（加密则经 open_entry 解密）
    {
        let file = std::fs::File::open(snapshot)?;
        let mut zip = zip::ZipArchive::new(file)?;
        for entry in [DB_ENTRY, CONFIG_ENTRY] {
            if let Ok(mut e) = super::crypto::open_entry(&mut zip, entry) {
                let mut buf = Vec::new();
                e.read_to_end(&mut buf)?;
                merged.insert(entry.to_string(), sha256_hex(&buf));
                w.start_file(entry, opts)?;
                w.write_all(&buf)?;
            }
        }
    }
    // attachments 来自 bundle（同上解密读取；名字枚举用 file_names——加密条目 by_index 会报「需要密码」）
    {
        let file = std::fs::File::open(bundle)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let names: Vec<String> = zip.file_names().map(|n| n.to_string()).collect();
        for name in names {
            if !name.starts_with(ATTACHMENTS_PREFIX) || name.ends_with('/') {
                continue;
            }
            let mut buf = Vec::new();
            let mut e = super::crypto::open_entry(&mut zip, &name)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::backup::restore::{inspect_backup_impl, restore_backup_impl};

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
