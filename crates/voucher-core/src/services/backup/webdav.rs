//! WebDAV 云备份：上传（快照直连/全量拆分）、远端保留清理、连接测试、列表、下载。

use std::io::Read;
use std::path::Path;

use super::format::{compose_full_zip, write_cloud_parts};
use super::local::list_backups;
use super::*;
use crate::utils::config::SECRET_WEBDAV_PASSWORD;
use crate::{CoreContext, CoreError, Result};

/// WebDAV 上传总尝试次数（失败重试 2 次，指数退避）
const UPLOAD_ATTEMPTS: u32 = 3;

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

    #[test]
    fn remote_name_whitelist_accepts_incremental_files() {
        assert!(is_valid_remote_name("voucher-snapshot-20260818-093000.zip"));
        assert!(is_valid_remote_name("voucher-attachments-a1b2c3d4.zip"));
        assert!(!is_valid_remote_name("voucher-attachments-../evil.zip"));
        assert!(!is_valid_remote_name("voucher-snapshot-x.exe"));
    }
}
