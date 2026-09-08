//! 备份与恢复（对应 v4.3 的 `webdav_backup.py`；备份为新版自有 zip 格式）
//!
//! 3-2-1 备份策略：
//! - 3 份数据：当前库 + 本地备份（GFS 分层保留）+ 异盘/云端副本
//! - 2 种介质：本地盘（backups/）+ 备用份目录（secondary_dir，建议异物理盘）
//! - 1 份异地：WebDAV 上传（带重试与远端保留清理）
//!
//! 备份内容：voucher.db 快照 + app.json + attachments/ 全部附件原图。
//! zip 内含版本化 manifest（含每文件 sha256），恢复与演练时逐文件校验。
//!
//! 子模块划分（v0.9 拆分，避免单文件多职责）：
//! - local：本地备份产出（快照+附件包）、列表、异盘双写、启动补传
//! - retention：GFS 分层保留 + 孤儿附件包回收（本地/异盘共用纯逻辑）
//! - format：快照/附件包格式构造、内容寻址指纹、materialize 合成
//! - restore：校验/释放/演练（快照经 materialize 透明合成）
//! - webdav：云端上传/清理/下载协议层

mod crypto;
mod format;
mod local;
mod restore;
mod retention;
mod webdav;

pub use format::export_full_backup;
pub use local::{list_backups, run_backup, run_local_backup};
pub use restore::{drill_backup, inspect_backup, restore_backup, DrillReport, RestoreSummary};
pub use webdav::{
    download_webdav_backup, list_webdav_backups, test_webdav, upload_backup_to_webdav,
    upload_missing_backups, WebdavBackupRow,
};

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const BACKUP_FORMAT: &str = "voucher-studio-backup";
/// v3：云端增量结构（snapshot + 内容寻址附件包；v2 及以下为全量单文件，均可恢复）
pub(super) const BACKUP_VERSION: u32 = 3;
pub const MANIFEST_FILE: &str = "manifest.json";
pub const DB_ENTRY: &str = "voucher.db";
pub const CONFIG_ENTRY: &str = "app.json";
pub const ATTACHMENTS_PREFIX: &str = "attachments/";
/// v0.8 云端快照文件名前缀（db+config+manifest，KB 级）
pub const SNAPSHOT_PREFIX: &str = "voucher-snapshot-";
/// v0.8 云端附件包文件名前缀（内容寻址：voucher-attachments-<hash8>.zip）
pub const BUNDLE_PREFIX: &str = "voucher-attachments-";

pub const KEY_LAST_BACKUP: &str = "last_backup_at";

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
impl BackupInfo {
    /// 排序键：文件名中的时间戳（提不出的排最后，保持稳定）
    pub(super) fn stamp_key(&self) -> String {
        stamp_of_file_name(&self.file_name).unwrap_or_else(|| String::from("9999"))
    }
}

/// 从备份文件名提取时间戳段（voucher-{backup,snapshot}-YYYYMMDD-HHMMSS.zip → YYYYMMDD-HHMMSS）
pub(super) fn stamp_of_file_name(name: &str) -> Option<String> {
    let digits: String = name.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 14 {
        Some(format!("{}-{}", &digits[..8], &digits[8..14]))
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Manifest {
    pub(super) format: String,
    pub(super) version: u32,
    pub(super) created_at: String,
    pub(super) files: Vec<String>,
    /// entry 名 → sha256 hex（v2 起有；v1 备份为空 = 恢复时跳过校验）
    #[serde(default)]
    pub(super) checksums: HashMap<String, String>,
    #[serde(default)]
    pub(super) attachment_count: u32,
    /// v3 快照指向的附件包文件名（仅云端增量结构的 snapshot 有；
    /// 全量备份 / 附件包自身为 None）
    #[serde(default)]
    pub(super) attachments_bundle: Option<String>,
}

pub(super) fn read_manifest(zip_path: &Path) -> Option<Manifest> {
    let file = std::fs::File::open(zip_path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    // v0.13：加密备份的 manifest 也是加密条目 → open_entry 走凭据管理器解密
    let mut entry = crypto::open_entry(&mut zip, MANIFEST_FILE).ok()?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    let mut s = String::with_capacity(d.len() * 2);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_is_stable() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
