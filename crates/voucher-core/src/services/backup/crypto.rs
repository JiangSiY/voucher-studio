//! v0.13 备份加密（AES-256 zip）→ v0.14-A 升级为 age passphrase 信封（格式 v4）。
//!
//! 背景（体检 #24-3，架构师定案）：zip crate 无 AEAD，WinZip AES（v0.13 所用）非
//! AEAD，无法防篡改。v0.14-A 起**写侧不再做条目级 zip 加密**：备份文件保持
//! 明文 zip 结构（db/config/attachments/manifest），写盘前用 age passphrase
//! 模式（scrypt KDF + ChaCha20-Poly1305 流式 AEAD）密封**整个文件**，存盘的是
//! 一个 age 信封（文件头 `age-encryption.org/`）。format/restore/webdav/retention
//! 对 zip 的内部逻辑全复用；manifest 的 sha256 校验仍保留（与 AEAD 双保险）。
//!
//! 密码流转（不变点）：写入侧 `password_for_write` 取配置开关 + 凭据管理器；
//! 读取侧 `stored_password` 取凭据管理器。age 信封的 salt 每次密封随机生成，
//! 同密码两次备份密文不同（salt 独立）。
//!
//! 读侧三态兼容（`read_manifest` / `compose_full_zip` / `materialize` 统一入口）：
//! 1. age 信封（`age-encryption.org/` 开头，v4）→ 先解密到临时明文 zip 再走原逻辑；
//! 2. zip 加密条目（旧 v0.13 WinZip AES）→ 保留 `open_entry` 旧分支，存量加密
//!    备份仍可恢复（密码同样取凭据管理器）；
//! 3. 明文 zip → 原逻辑，零改动。
//!
//! 合成产物（materialize → 全量 zip）与「导出完整备份」保持明文：前者是临时中间物，
//! 后者是用户主动拷走的文件（自行决定去留）。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::utils::config::SECRET_BACKUP_PASSWORD;
use crate::utils::crypto::SecretStore;
use crate::{CoreContext, CoreError, Result};

/// age 信封文件头魔数（v4 备份；`age-encryption.org/v1\n-> scrypt …`）
const AGE_MAGIC: &[u8] = b"age-encryption.org/";

/// 临时文件序号（同进程内并发 unseal/密封也不撞名）
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// v0.13 旧写侧选项（仅测试构造「存量 AES 加密包」样例用）。
/// v0.14-A 生产写入不再调用：见 `seal_file`。
#[cfg(test)]
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

/// 读取侧强制要求有密码（age 信封 / 加密 zip 条目解密用）。
/// 未设置 / 读取失败 → 明确中文错误（绝不静默按明文处理）。
fn require_stored_password() -> Result<String> {
    stored_password().ok_or_else(|| {
        CoreError::Invalid(
            "备份已加密但本机未设置解密密码：请到「设置 → 备份与同步 → 备份加密」设置当初的密码"
                .into(),
        )
    })
}

fn secret(passphrase: &str) -> age::secrecy::SecretString {
    age::secrecy::SecretString::from(passphrase.to_string())
}

/// 判断 `path` 是否为 age 信封（读文件头 `AGE_MAGIC`）。文件不存在/过短 → false。
pub(super) fn is_age_envelope(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; AGE_MAGIC.len()];
    if f.read(&mut buf).unwrap_or(0) != AGE_MAGIC.len() {
        return false;
    }
    buf.as_slice() == AGE_MAGIC
}

/// 判断内存字节流是否为 age 信封（WebDAV manifest 读取等场景）。
pub(super) fn is_age_envelope_bytes(bytes: &[u8]) -> bool {
    bytes.starts_with(AGE_MAGIC)
}

/// 密封：把明文文件（备份 zip）用 age passphrase 密封写入 `dest`。
/// 先在 dest 同目录写临时再 rename——中断不留半截文件；salt 每次随机（同密码密文不同）。
pub(super) fn seal_file(plain_src: &Path, dest: &Path, password: &str) -> Result<()> {
    let mut src = std::fs::File::open(plain_src)?;
    let dir = dest.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(
        ".age-seal-{}-{}.tmp",
        std::process::id(),
        TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let out = std::fs::File::create(&tmp)?;
    let encryptor = age::Encryptor::with_user_passphrase(secret(password));
    let mut writer = encryptor
        .wrap_output(out)
        .map_err(|e| CoreError::Invalid(format!("备份加密初始化失败：{e}")))?;
    std::io::copy(&mut src, &mut writer)
        .map_err(|e| CoreError::Invalid(format!("备份加密写入失败：{e}")))?;
    writer
        .finish()
        .map_err(|e| CoreError::Invalid(format!("备份加密收尾失败：{e}")))?;
    // fsync + rename：这是**加密备份真正的落盘点**（local.rs 的收口不覆盖加密分支）
    crate::utils::jsonio::commit_file(&tmp, dest)?;
    Ok(())
}

/// 用显式密码解密 age 信封 → 写明文到 `dst`（不经过凭据管理器，便于单测）。
fn unseal_with<R: Read, W: Write>(src: R, mut dst: W, password: &str) -> Result<()> {
    let decryptor = age::Decryptor::new(src)
        .map_err(|e| CoreError::Invalid(format!("备份不是有效的加密备份或文件头已损坏：{e}")))?;
    let identity = age::scrypt::Identity::new(secret(password));
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|e| CoreError::Invalid(format!("备份解密失败（密码错误或文件被篡改）：{e}")))?;
    std::io::copy(&mut reader, &mut dst)?;
    Ok(())
}

/// 解密 age 信封 → 临时明文 zip（写进 `dir`），返回临时路径（调用方负责删除）。
pub(super) fn unseal_to_temp_dir(sealed: &Path, dir: &Path) -> Result<PathBuf> {
    let password = require_stored_password()?;
    let dest = dir.join(format!(
        ".age-unseal-{}-{}.zip",
        std::process::id(),
        TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    unseal_with(
        std::fs::File::open(sealed)?,
        std::fs::File::create(&dest)?,
        &password,
    )?;
    Ok(dest)
}

/// 解密 age 信封内存字节 → 明文字节（WebDAV 远端 manifest 解析用）。
pub(super) fn unseal_bytes(sealed: &[u8]) -> Result<Vec<u8>> {
    let password = require_stored_password()?;
    let mut out = Vec::new();
    unseal_with(sealed, &mut out, &password)?;
    Ok(out)
}

/// 以「可读取的明文 zip 路径」执行 `f`（所有备份读取入口的统一分派点）：
/// - age 信封（v4）→ 先解密到系统临时目录，回调后自动删除；
/// - 其它（明文 zip / v0.13 加密 zip）→ 原路径直接回调。
///
/// v0.13 加密 zip 的条目级解密仍由回调内的 `open_entry` 负责。
pub(super) fn with_readable_zip<T>(path: &Path, f: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    if !is_age_envelope(path) {
        return f(path);
    }
    let plain = unseal_to_temp_dir(path, &std::env::temp_dir())?;
    let r = f(&plain);
    let _ = std::fs::remove_file(&plain);
    r
}

/// 探测 zip 条目是否加密（含 zip crate 把「需密码」表达为 UnsupportedArchive 的旧路径）。
fn entry_needs_password<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<bool> {
    match zip.by_name(name) {
        Ok(f) => Ok(f.encrypted()),
        Err(zip::result::ZipError::UnsupportedArchive(msg)) => {
            Ok(msg.contains("Password") || msg.contains("password"))
        }
        Err(e) => Err(e.into()),
    }
}

/// 打开 zip 条目（含解密核心）：先明文；条目被加密时用给定密码解密。
/// 密码错误在 read/decrypt 阶段报 AEAD/校验失败，由调用方给出中文错误。
pub(super) fn open_entry_with_password<'a, R: Read + std::io::Seek>(
    zip: &'a mut zip::ZipArchive<R>,
    name: &str,
    password: &str,
) -> Result<zip::read::ZipFile<'a>> {
    if !entry_needs_password(zip, name)? {
        return zip.by_name(name).map_err(CoreError::from);
    }
    zip.by_name_decrypt(name, password.as_bytes())
        .map_err(|e| CoreError::Invalid(format!("备份解密失败（密码错误或文件损坏）：{e}")))
}

/// 打开 zip 条目：先明文；条目被加密时用凭据管理器密码重试（兼容旧 v0.13 加密备份）。
/// 密码错误在 read 阶段报 AEAD 校验失败，由调用方给出中文错误。
pub(super) fn open_entry<'a, R: Read + std::io::Seek>(
    zip: &'a mut zip::ZipArchive<R>,
    name: &str,
) -> Result<zip::read::ZipFile<'a>> {
    match stored_password() {
        Some(p) => open_entry_with_password(zip, name, &p),
        None => {
            // 明文条目仍可直接打开（兼容只读明文备份）；仅加密条目需要密码
            if !entry_needs_password(zip, name)? {
                zip.by_name(name).map_err(CoreError::from)
            } else {
                Err(CoreError::Invalid(
                    "备份已加密但本机未设置解密密码：请到「设置 → 备份与同步 → 备份加密」设置当初的密码"
                        .into(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_zip(path: &std::path::Path, password: Option<&str>) {
        let file = std::fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(file);
        let opts = write_options(password);
        w.start_file("a.txt", opts).unwrap();
        w.write_all(b"secret-content").unwrap();
        w.finish().unwrap();
    }

    /// 生成一个小的明文测试文件
    fn temp_plain(dir: &std::path::Path, content: &[u8]) -> std::path::PathBuf {
        let p = dir.join("plain.bin");
        std::fs::write(&p, content).unwrap();
        p
    }

    fn read_all(p: &std::path::Path) -> Vec<u8> {
        std::fs::read(p).unwrap()
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

    /// 存量 v0.13 WinZip AES 加密条目仍可用显式密码解密（读侧兼容分支的单元级验证）。
    #[test]
    fn aes_entry_decrypt_with_explicit_password() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("enc.zip");
        make_zip(&p, Some("pwd123"));

        // 明文打开加密条目 → 报「需要密码」
        let file = std::fs::File::open(&p).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        assert!(entry_needs_password(&mut zip, "a.txt").unwrap());

        // 正确密码 → 读出内容
        let mut f = open_entry_with_password(&mut zip, "a.txt", "pwd123").expect("解密打开成功");
        let mut s = String::new();
        f.read_to_string(&mut s).expect("解密读取成功");
        assert_eq!(s, "secret-content");

        // 错误密码 → 打开即失败（zip 在 open 阶段校验）
        let file2 = std::fs::File::open(&p).unwrap();
        let mut zip2 = zip::ZipArchive::new(file2).unwrap();
        assert!(
            open_entry_with_password(&mut zip2, "a.txt", "wrong").is_err(),
            "错误密码应打开失败"
        );
    }

    // ---------- v0.14-A：age 信封密封原语 ----------

    #[test]
    fn age_seal_roundtrip_same_password() {
        let dir = tempfile::tempdir().unwrap();
        let plain = temp_plain(dir.path(), b"voucher backup bytes");
        let sealed = dir.path().join("sealed.age");
        seal_file(&plain, &sealed, "pw-123").unwrap();

        // 产物是 age 信封（不是 zip）：文件头为 age-encryption.org/
        assert!(is_age_envelope(&sealed));
        assert!(is_age_envelope_bytes(&read_all(&sealed)));
        let head = read_all(&sealed);
        assert!(head.starts_with(b"age-encryption.org/v1"), "age v1 信封头");

        // 回读（明文文件应与原文一致）
        let unsealed = dir.path().join("unsealed.bin");
        unseal_with(
            std::fs::File::open(&sealed).unwrap(),
            std::fs::File::create(&unsealed).unwrap(),
            "pw-123",
        )
        .unwrap();
        assert_eq!(read_all(&unsealed), b"voucher backup bytes");

        // 明文 zip 不是 age 信封
        assert!(!is_age_envelope(&plain));
        assert!(!is_age_envelope_bytes(b"PK\x03\x04 not age"));
    }

    #[test]
    fn age_wrong_password_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let plain = temp_plain(dir.path(), b"payload");
        let sealed = dir.path().join("sealed.age");
        seal_file(&plain, &sealed, "right-pw").unwrap();
        let out = dir.path().join("out.bin");
        let err = unseal_with(
            std::fs::File::open(&sealed).unwrap(),
            std::fs::File::create(&out).unwrap(),
            "wrong-pw",
        );
        assert!(err.is_err(), "错误密码必须解密失败");
    }

    #[test]
    fn age_tampered_header_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let plain = temp_plain(dir.path(), b"payload");
        let sealed = dir.path().join("sealed.age");
        seal_file(&plain, &sealed, "pw").unwrap();
        let mut bytes = read_all(&sealed);
        // 篡改文件头（magic 后第 6 字节翻位：破坏版本行）
        let pos = AGE_MAGIC.len() + 6;
        assert!(bytes.len() > pos, "age 文件头应远长于篡改位置");
        bytes[pos] ^= 0xFF;
        let tampered = dir.path().join("tampered.age");
        std::fs::write(&tampered, &bytes).unwrap();
        let out = dir.path().join("out.bin");
        let err = unseal_with(
            std::fs::File::open(&tampered).unwrap(),
            std::fs::File::create(&out).unwrap(),
            "pw",
        );
        assert!(err.is_err(), "文件头被篡改必须解密失败");
        assert!(
            !out.exists() || read_all(&out).is_empty(),
            "不得产出被篡改的明文"
        );
    }

    #[test]
    fn age_tampered_payload_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let plain = temp_plain(dir.path(), b"payload-payload-payload");
        let sealed = dir.path().join("sealed.age");
        seal_file(&plain, &sealed, "pw").unwrap();
        let mut bytes = read_all(&sealed);
        // 篡改密文主体（末尾附近一字节）——AEAD 标签校验必须失败
        let n = bytes.len();
        assert!(n >= 4, "age 密文长度需足以承载主体篡改点");
        bytes[n - 3] ^= 0xFF;
        let tampered = dir.path().join("tampered.age");
        std::fs::write(&tampered, &bytes).unwrap();
        let out = dir.path().join("out.bin");
        let err = unseal_with(
            std::fs::File::open(&tampered).unwrap(),
            std::fs::File::create(&out).unwrap(),
            "pw",
        );
        assert!(err.is_err(), "密文被篡改必须被 AEAD 拒绝");
    }

    #[test]
    fn age_same_plaintext_salt_independent() {
        // 同密码两次密封同一明文 → 密文不同（salt 独立）+ 都能正确解密
        let dir = tempfile::tempdir().unwrap();
        let plain = temp_plain(dir.path(), b"same plaintext content");
        let s1 = dir.path().join("s1.age");
        let s2 = dir.path().join("s2.age");
        seal_file(&plain, &s1, "pw").unwrap();
        seal_file(&plain, &s2, "pw").unwrap();
        assert_ne!(
            read_all(&s1),
            read_all(&s2),
            "两次密封 salt 应独立、密文不同"
        );
        for s in [&s1, &s2] {
            let out = dir.path().join("out.bin");
            unseal_with(
                std::fs::File::open(s).unwrap(),
                std::fs::File::create(&out).unwrap(),
                "pw",
            )
            .unwrap();
            assert_eq!(read_all(&out), b"same plaintext content");
        }
    }
}
