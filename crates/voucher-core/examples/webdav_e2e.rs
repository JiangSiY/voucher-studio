//! WebDAV 真机联调（v0.8 P0）：对真实服务器（坚果云）跑完整协议链路。
//!
//! 用法：cargo run -p voucher-core --example webdav_e2e [-- --keep]
//!
//! 链路：PROPFIND 测试连接 → PUT 上传最新本地备份 → PROPFIND 列表确认
//!       → 移走本地同名文件 → GET 下载回来 → sha256 比对 → （默认）DELETE 清理远端测试文件
//!
//! 安全性：全程只操作 voucher-backup-*.zip 白名单文件；上传的是真实备份内容。

use std::path::PathBuf;

use voucher_core::services::backup;
use voucher_core::CoreContext;

fn main() {
    let keep = std::env::args().any(|a| a == "--keep");
    let target = PathBuf::from(std::env::var("APPDATA").expect("APPDATA"))
        .join("VoucherStudio")
        .join("data");
    let ctx = CoreContext::init(Some(target.clone())).expect("init");

    println!("=== WebDAV 真机联调（数据目录 {}）===", target.display());
    let cfg = ctx.config();
    println!(
        "目标服务器: {} (user: {})",
        cfg.backup.webdav.url, cfg.backup.webdav.username
    );

    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut uploaded_name: Option<String> = None;

    // ---- 步骤 1：连接测试（PROPFIND Depth:1）----
    match backup::test_webdav(&ctx) {
        Ok(n) => {
            println!("[1] PROPFIND 连接测试: PASS（远端已有 {n} 份备份）");
            pass += 1;
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("自动创建") {
                // 目录不存在是预期场景：认证通过 + 明确指引文案 = 连接测试尽责
                println!("[1] PROPFIND 连接测试: PASS（目录不存在，文案指引自动创建）");
                pass += 1;
            } else {
                println!("[1] PROPFIND 连接测试: FAIL（{e}）");
                fail += 1;
            }
        }
    }

    // ---- 步骤 2：远端列表（解析 + 倒序）----
    let before = match backup::list_webdav_backups(&ctx) {
        Ok(rows) => {
            println!("[2] 远端列表解析: PASS（{} 份，新→旧前 5：）", rows.len());
            for r in rows.iter().take(5) {
                println!("      {}", r.file_name);
            }
            pass += 1;
            rows.into_iter().map(|r| r.file_name).collect::<Vec<_>>()
        }
        Err(e) => {
            if e.to_string().contains("自动创建") {
                // 目录尚未创建（本次联调会触发 MKCOL）：列表报错属预期
                println!("[2] 远端列表解析: PASS（目录尚未创建，上传时自动建）");
                pass += 1;
            } else {
                println!("[2] 远端列表解析: FAIL（{e}）");
                fail += 1;
            }
            Vec::new()
        }
    };

    // ---- 步骤 3：上传最新本地备份（PUT + 重试）----
    let local = std::fs::read_dir(ctx.paths.backups.clone())
        .expect("read backups dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("voucher-backup-")
        })
        .max_by_key(|e| e.file_name().to_string_lossy().to_string())
        .expect("至少要有一个本地备份");
    let name = local.file_name().to_string_lossy().into_owned();
    println!("[3] 上传本地备份 {name}（2.84 MB 级，PUT）…");
    match backup::upload_backup_to_webdav(&ctx, &local.path()) {
        Ok(()) => {
            println!("[3] PUT 上传: PASS");
            pass += 1;
            uploaded_name = Some(name.clone());
        }
        Err(e) => {
            println!("[3] PUT 上传: FAIL（{e}）");
            fail += 1;
        }
    }

    // ---- 步骤 4：列表确认新文件出现 ----
    match backup::list_webdav_backups(&ctx) {
        Ok(rows) => {
            let names: Vec<String> = rows.into_iter().map(|r| r.file_name).collect();
            if let Some(n) = &uploaded_name {
                if names.contains(n) {
                    println!("[4] 上传后列表包含新文件: PASS");
                    pass += 1;
                } else {
                    println!("[4] 上传后列表包含新文件: FAIL（{n} 未出现在远端列表）");
                    fail += 1;
                }
            }
            // 顺带验证排序：新→旧（文件名时间戳倒序）
            let mut sorted = names.clone();
            sorted.sort();
            sorted.reverse();
            if sorted == names {
                println!("[4] 列表新→旧排序: PASS");
                pass += 1;
            } else {
                println!("[4] 列表新→旧排序: FAIL");
                fail += 1;
            }
        }
        Err(e) => {
            println!("[4] 上传后列表复查: FAIL（{e}）");
            fail += 1;
        }
    }

    // ---- 步骤 5：下载回来（GET + 临时文件落盘）----
    if let Some(n) = uploaded_name.clone() {
        // 先移走本地同名文件，否则幂等早退测不到真实下载
        let dest = ctx.paths.backups.join(&n);
        let moved = ctx.paths.temp.join(format!("webdav-e2e-park-{n}"));
        let parked = if dest.exists() {
            std::fs::rename(&dest, &moved).is_ok()
        } else {
            false
        };
        match backup::download_webdav_backup(&ctx, &n) {
            Ok(path) => {
                // sha256 比对：上传源（被移走的原件）vs 下载件
                let src = if parked { moved.clone() } else { local.path() };
                let src_hash = sha256_file(&src);
                let dl_hash = sha256_file(std::path::Path::new(&path));
                if src_hash == dl_hash && !src_hash.is_empty() {
                    println!("[5] GET 下载 + sha256 一致: PASS（{path}）");
                    pass += 1;
                } else {
                    println!(
                        "[5] GET 下载 + sha256 一致: FAIL（src={src_hash:.16}… dl={dl_hash:.16}…）"
                    );
                    fail += 1;
                }
            }
            Err(e) => {
                println!("[5] GET 下载: FAIL（{e}）");
                fail += 1;
            }
        }
        // 恢复现场：下载件留在 backups/（本来就是有效备份）；移走的原件若与下载件同内容则删除，否则还原
        if parked {
            let dl = ctx.paths.backups.join(&n);
            if sha256_file(&dl) == sha256_file(&moved) {
                let _ = std::fs::remove_file(&moved); // 内容一致，保留一份即可
            } else {
                let _ = std::fs::rename(&moved, &dl); // 内容不一致，还原原件防数据丢失
            }
        }
    }

    // ---- 步骤 6：非法文件名防护（不应发起网络请求即被拒绝）----
    match backup::download_webdav_backup(&ctx, "../voucher-backup-evil.zip") {
        Err(e) if e.to_string().contains("非法") => {
            println!("[6] 路径穿越入参拒绝: PASS");
            pass += 1;
        }
        other => {
            println!("[6] 路径穿越入参拒绝: FAIL（{other:?}）");
            fail += 1;
        }
    }

    // ---- 步骤 7（默认）：清理远端测试上传，恢复联调前状态 ----
    if !keep {
        if let Some(n) = uploaded_name {
            if !before.contains(&n) {
                // 仅当该文件是本次联调新上传的才删；之前就在云端的绝不动
                match delete_remote(&ctx, &n) {
                    Ok(true) => {
                        println!("[7] 清理远端测试文件: PASS（{n} 已删除，远端恢复联调前状态）");
                        pass += 1;
                    }
                    Ok(false) => {
                        println!("[7] 清理远端测试文件: FAIL（未找到）");
                        fail += 1;
                    }
                    Err(e) => {
                        println!("[7] 清理远端测试文件: FAIL（{e}）");
                        fail += 1;
                    }
                }
            } else {
                println!("[7] 清理: SKIP（{n} 联调前已存在于云端，保留不动）");
            }
        }
    } else {
        println!("[7] 清理: SKIP（--keep 保留云端文件）");
    }

    println!("\n=== 结果：{pass} PASS / {fail} FAIL ===");
    if fail > 0 {
        std::process::exit(1);
    }
}

fn sha256_file(p: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let Ok(bytes) = std::fs::read(p) else {
        return String::new();
    };
    let mut h = Sha256::new();
    h.update(&bytes);
    format!("{:x}", h.finalize())
}

/// 删除远端单个备份（仅用于联调清理；复用 backup 模块的认证与 URL 构造约定）
fn delete_remote(ctx: &CoreContext, file_name: &str) -> voucher_core::Result<bool> {
    // backup 模块没有公开单文件删除，这里用同构逻辑：读配置 + 凭据库
    let cfg = ctx.config();
    let webdav = &cfg.backup.webdav;
    let password = ctx
        .secrets
        .get(voucher_core::utils::config::SECRET_WEBDAV_PASSWORD)?
        .ok_or_else(|| voucher_core::CoreError::Invalid("WebDAV 密码未设置".into()))?;
    let base = webdav.url.trim().trim_end_matches('/');
    let url = format!("{base}/{file_name}");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let resp = client
        .delete(&url)
        .basic_auth(&webdav.username, Some(&password))
        .send()?;
    Ok(resp.status().is_success())
}
