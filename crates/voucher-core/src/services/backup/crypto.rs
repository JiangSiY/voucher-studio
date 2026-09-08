//! v0.13 备份加密（AES-256 zip；密码存 Windows 凭据管理器，永不落盘）。
//!
//! 加密点：写入侧（write_snapshot_zip / write_bundle_zip）——本地主备份、
//! 异盘副本（文件复制）、WebDAV 云端（上传同一文件）一次性全覆盖。
//! 解密点：读取侧统一走 `open_entry`——先按明文打开，命中加密条目再用
//! 凭据管理器里的密码重试（旧明文备份完全兼容）。
//! 合成产物（materialize → 全量 zip）与「导出完整备份」保持明文：
//! 前者是临时中间物，后者是用户主动拷走的文件（自行决定去留）。

use std::io::Read;

use crate::utils::config::SECRET_BACKUP_PASSWORD;
use crate::utils::crypto::SecretStore;
use crate::{CoreContext, CoreError, Result};

/// 写入选项：`password` 非 None 时全部条目挂 AES-256。
/// 返回 FileOptions 携带密码引用生命周期（调用方持有密码变量即可）。
pub(super) fn write_options<'k>(password: Option<&'k str>) -> zip::write::FileOptions<'k, ()> {
    let mut o = zip::write::FileOptions::<()>::default()
        .compression_method(zip::CompressionMethod::Deflated);
    if let Some(p) = password {
        o = o.with_aes_encryption(zip::AesMode::Aes256, p);
    }
    o
}

/// 备份写入用的密码：`cfg.backup.encrypted` 开启 → 必须有密码（否则报错，
/// 宁可备份失败也不静默降级成明文）；未开启 → None（明文备份）。
pub(super) fn password_for_write(ctx: &CoreContext) -> Result<Option<String>> {
    if !ctx.config().backup.encrypted {
        return Ok(None);
    }
    match ctx
        .secrets
        .get(SECRET_BACKUP_PASSWORD)?
        .filter(|p| !p.is_empty())
    {
        Some(p) => Ok(Some(p)),
        None => Err(CoreError::Invalid(
            "已启用备份加密但未设置密码：请到「设置 → 备份与同步 → 备份加密」设置密码后再备份"
                .into(),
        )),
    }
}

/// 读取侧密码（凭据管理器；读取失败 / 未设置 → None，由明文兼容路径兜底）。
fn stored_password() -> Option<String> {
    SecretStore::new("VoucherStudio")
        .get(SECRET_BACKUP_PASSWORD)
        .ok()
        .flatten()
        .filter(|p| !p.is_empty())
}

/// 打开 zip 条目：先明文；条目被加密时用凭据管理器密码重试（兼容旧明文备份）。
/// 密码错误在 read 阶段报 AEAD 校验失败，由调用方给出中文错误。
pub(super) fn open_entry<'a, R: Read + std::io::Seek>(
    zip: &'a mut zip::ZipArchive<R>,
    name: &str,
) -> Result<zip::read::ZipFile<'a>> {
    // 探测：打开后立即丢弃（借用即释放），拿「是否需要密码」
    let needs_password = {
        match zip.by_name(name) {
            Ok(f) => f.encrypted(),
            Err(zip::result::ZipError::UnsupportedArchive(msg)) => {
                msg.contains("Password") || msg.contains("password")
            }
            Err(e) => return Err(e.into()),
        }
    };
    if !needs_password {
        return zip.by_name(name).map_err(CoreError::from);
    }
    match stored_password() {
        Some(p) => zip
            .by_name_decrypt(name, p.as_bytes())
            .map_err(|e| CoreError::Invalid(format!("备份解密失败（密码错误或文件损坏）：{e}"))),
        None => Err(CoreError::Invalid(
            "备份已加密但本机未设置解密密码：请到「设置 → 备份与同步 → 备份加密」设置当初的密码"
                .into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(path: &std::path::Path, password: Option<&str>) {
        let file = std::fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(file);
        let opts = write_options(password);
        w.start_file("a.txt", opts).unwrap();
        w.write_all(b"secret-content").unwrap();
        w.finish().unwrap();
    }

    #[test]
    fn plaintext_options_no_encryption() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plain.zip");
        make_zip(&p, None);
        let file = std::fs::File::open(&p).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut e = open_entry(&mut zip, "a.txt").unwrap();
        assert!(!e.encrypted());
        let mut buf = String::new();
        e.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "secret-content");
    }

    #[test]
    fn aes_entry_roundtrip_manual() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("enc.zip");
        make_zip(&p, Some("pwd123"));

        // 1) 明文打开加密条目 → 报「需要密码」
        let file = std::fs::File::open(&p).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        {
            let plain = zip.by_name("a.txt");
            assert!(plain.is_err(), "加密条目明文打开应报错");
        }

        // 2) by_name_decrypt + 正确密码 → 读出内容
        let mut f = zip
            .by_name_decrypt("a.txt", b"pwd123")
            .expect("解密打开成功");
        let mut s = String::new();
        f.read_to_string(&mut s).expect("解密读取成功");
        assert_eq!(s, "secret-content");

        // 3) 错误密码 → 打开即失败（zip 在 open 阶段校验）
        let file2 = std::fs::File::open(&p).unwrap();
        let mut zip2 = zip::ZipArchive::new(file2).unwrap();
        assert!(
            zip2.by_name_decrypt("a.txt", b"wrong").is_err(),
            "错误密码应打开失败"
        );
    }
}
