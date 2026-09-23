//! WebDAV 联网链路 mock 测试（体检第三批第 14 项）：
//! 手写极简 HTTP mock（零新依赖，Connection: close 逐请求回放），
//! 覆盖 401 确定性失败不重试、5xx 指数退避重试至成功、PROPFIND 列表解析与过滤。
//!
//! 前提：`validated_webdav_base` 对回环地址（127.0.0.1）放行 http，
//! mock server 恰好落在豁免内——不绕过任何生产校验。

mod common;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use voucher_core::services::backup;
use voucher_core::utils::config::SECRET_WEBDAV_PASSWORD;
use voucher_core::{CoreContext, Result};

/// 一次被记录的请求摘要（method + path + 是否带 Basic Auth）
#[derive(Debug, Clone, PartialEq)]
struct RecordedReq {
    method: String,
    path: String,
    has_auth: bool,
}

/// 极简同步 HTTP mock：按到达顺序回放预设响应（耗尽后重复最后一个），记录全部请求。
/// 每个请求独立连接（响应带 Connection: close），与 reqwest blocking 兼容。
struct MockWebdav {
    url: String,
    requests: Arc<Mutex<Vec<RecordedReq>>>,
    _handle: std::thread::JoinHandle<()>,
}

impl MockWebdav {
    fn start(responses: Vec<(u16, String)>) -> Self {
        assert!(!responses.is_empty(), "至少一个预设响应");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let requests: Arc<Mutex<Vec<RecordedReq>>> = Arc::new(Mutex::new(Vec::new()));
        let reqs = Arc::clone(&requests);
        let handle = std::thread::spawn(move || {
            let mut idx = 0usize;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let Some(req) = read_one_request(&mut stream) else {
                    continue;
                };
                reqs.lock().unwrap().push(req);
                let (status, body) = &responses[idx.min(responses.len() - 1)];
                idx += 1;
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        Self {
            url: format!("http://{addr}"),
            requests,
            _handle: handle,
        }
    }

    fn recorded(&self) -> Vec<RecordedReq> {
        self.requests.lock().unwrap().clone()
    }
}

/// 读取一个完整 HTTP 请求（headers + Content-Length 定长的 body），返回摘要。
fn read_one_request(stream: &mut TcpStream) -> Option<RecordedReq> {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    // 读到 header 结束
    loop {
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            return None; // 连接关闭且无完整请求
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let header_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("上方循环保证存在");
    let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let head = headers.lines().next().unwrap_or_default().to_string();
    let mut parts = head.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    // 读 body（PUT 带 Content-Length）
    let content_length: usize = headers
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())
                .flatten()
        })
        .unwrap_or(0);
    while buf.len() < header_end + 4 + content_length {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    Some(RecordedReq {
        method,
        path,
        has_auth: headers
            .to_ascii_lowercase()
            .contains("authorization: basic"),
    })
}

/// 构造 WebDAV 已启用的测试上下文（url 指向 mock；密码写入凭据库）
fn webdav_ctx(ctx: &CoreContext, url: &str) -> Result<()> {
    webdav_ctx_with_retention(ctx, url, 0)
}

/// 同上，但可指定远端保留数——`0` = 跳过远端清理（既有用例的单请求闭环），
/// `>=1` 才会走 `cleanup_webdav`（2026-09-23 残余清理新增，供 fail-closed 用例使用）。
fn webdav_ctx_with_retention(ctx: &CoreContext, url: &str, remote_retention: u32) -> Result<()> {
    ctx.secrets.set(SECRET_WEBDAV_PASSWORD, "mock-pw")?;
    let mut cfg = ctx.config();
    cfg.backup.webdav.enabled = true;
    cfg.backup.webdav.url = url.to_string();
    cfg.backup.webdav.username = "tester".into();
    cfg.backup.webdav.once_daily = false; // 跳过「当日已传」检查，聚焦 PUT 本身
    cfg.backup.webdav.remote_retention = remote_retention;
    ctx.set_config(cfg)
}

/// 造一个「无附件旧全量」备份文件（非 zip 结构 → read_manifest 失败 → 整文件 PUT 路径）
fn plain_backup_file(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("voucher-backup-20260909-000000.zip");
    std::fs::write(&p, b"mock full backup bytes").unwrap();
    p
}

/// 401 是确定性失败（认证/权限），重试无意义：单次 PUT 即报错，
/// 且错误文案带状态码供用户排查。
#[test]
fn webdav_upload_401_fails_without_retry() -> Result<()> {
    let (dir, ctx) = common::test_ctx();
    let mock = MockWebdav::start(vec![(401, String::new())]);
    webdav_ctx(&ctx, &mock.url)?;
    let zip = plain_backup_file(dir.path());

    let err = backup::upload_backup_to_webdav(&ctx, &zip).expect_err("401 应报错");
    assert!(
        err.to_string().contains("401"),
        "错误应带状态码 401，实际：{err}"
    );
    let reqs = mock.recorded();
    assert_eq!(reqs.len(), 1, "401 不应重试，实际请求 {} 次", reqs.len());
    assert_eq!(reqs[0].method, "PUT");
    assert!(reqs[0].has_auth, "PUT 应携带 Basic Auth");
    Ok(())
}

/// 5xx 为暂时性失败：指数退避重试（2s/4s）后第三次成功。
/// 断言恰好 3 次请求（UPLOAD_ATTEMPTS 上限）。
#[test]
fn webdav_upload_5xx_retries_until_success() -> Result<()> {
    let (dir, ctx) = common::test_ctx();
    let mock = MockWebdav::start(vec![
        (500, String::new()),
        (502, String::new()),
        (201, String::new()),
    ]);
    webdav_ctx(&ctx, &mock.url)?;
    let zip = plain_backup_file(dir.path());

    backup::upload_backup_to_webdav(&ctx, &zip).expect("第三次重试成功后应整体 Ok");
    let reqs = mock.recorded();
    assert_eq!(
        reqs.len(),
        3,
        "500→502→201 应恰好 3 次 PUT，实际 {:?}",
        reqs
    );
    assert!(reqs.iter().all(|r| r.method == "PUT"));
    Ok(())
}

/// PROPFIND 列表解析：远端列表只含「可恢复」备份（全量 + 快照），
/// 附件包（bundle）是内部件被滤除；目录自身与白名单外的 zip 一并排除。
#[test]
fn webdav_list_filters_bundles_and_unknown_names() -> Result<()> {
    let (_dir, ctx) = common::test_ctx();
    let xml = "<?xml version=\"1.0\"?>\
<d:multistatus xmlns:d=\"DAV:\">\
<d:response><d:href>/dav/</d:href></d:response>\
<d:response><d:href>/dav/voucher-backup-20260901-000000.zip</d:href></d:response>\
<d:response><d:href>/dav/voucher-backup-20260908-000000.zip</d:href></d:response>\
<d:response><d:href>/dav/voucher-snapshot-20260909-120000.zip</d:href></d:response>\
<d:response><d:href>/dav/voucher-attachments-1c81db15.zip</d:href></d:response>\
<d:response><d:href>/dav/other-file.zip</d:href></d:response>\
</d:multistatus>";
    let mock = MockWebdav::start(vec![(207, xml.to_string())]);
    webdav_ctx(&ctx, &mock.url)?;

    let rows = backup::list_webdav_backups(&ctx)?;
    let names: Vec<&str> = rows.iter().map(|r| r.file_name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "voucher-snapshot-20260909-120000.zip",
            "voucher-backup-20260908-000000.zip",
            "voucher-backup-20260901-000000.zip",
        ],
        "列表应只含可恢复备份且新→旧排序"
    );

    let reqs = mock.recorded();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "PROPFIND");
    assert!(reqs[0].has_auth);
    Ok(())
}

/// 连接测试（test_webdav）返回远端全部合法备份名计数（含附件包——
/// 与 list_webdav_backups 的「可恢复」口径不同，二者语义均已在各自文档注明）。
#[test]
fn webdav_test_connection_counts_all_valid_names() -> Result<()> {
    let (_dir, ctx) = common::test_ctx();
    let xml = "<?xml version=\"1.0\"?>\
<d:multistatus xmlns:d=\"DAV:\">\
<d:response><d:href>/dav/voucher-backup-20260901-000000.zip</d:href></d:response>\
<d:response><d:href>/dav/voucher-snapshot-20260909-120000.zip</d:href></d:response>\
<d:response><d:href>/dav/voucher-attachments-1c81db15.zip</d:href></d:response>\
</d:multistatus>";
    let mock = MockWebdav::start(vec![(207, xml.to_string())]);
    webdav_ctx(&ctx, &mock.url)?;

    assert_eq!(backup::test_webdav(&ctx)?, 3);
    Ok(())
}

/// 残余清理（B12）：远端附件包回收必须 **fail-closed**——被淘汰备份的 DELETE 失败时，
/// 不得继续回收附件包，否则远端会留下「快照/备份还在、附件包已被删」的永久不可恢复备份。
///
/// 构造：PROPFIND 列出两份旧全量 + 一个附件包，`remote_retention=1` → 淘汰字典序最小的
/// 那份；让它的 DELETE 返回 500（确定性失败）→ 断言**没有**对附件包发 DELETE。
/// fail-closed 生效时表现为「少一次请求」，从返回值看不出来 → 断言必须落在请求记录上。
///
/// 用旧全量而非快照：快照会触发 manifest GET，而 mock 只能回字符串（回不了二进制 zip），
/// 那会让「manifest 不可读」也置为跳过，混入另一种原因、失去判别力。
#[test]
fn webdav_cleanup_skips_bundle_reclaim_when_delete_fails() -> Result<()> {
    let (dir, ctx) = common::test_ctx();
    let xml = "<?xml version=\"1.0\"?>\
<d:multistatus xmlns:d=\"DAV:\">\
<d:response><d:href>/dav/</d:href></d:response>\
<d:response><d:href>/dav/voucher-backup-20260901-000000.zip</d:href></d:response>\
<d:response><d:href>/dav/voucher-backup-20260908-000000.zip</d:href></d:response>\
<d:response><d:href>/dav/voucher-attachments-1c81db15.zip</d:href></d:response>\
</d:multistatus>";
    // 回放顺序：PUT(201) → PROPFIND(207) → DELETE 旧全量(500)；耗尽后重复最后一个
    let mock = MockWebdav::start(vec![
        (201, String::new()),
        (207, xml.to_string()),
        (500, String::new()),
    ]);
    webdav_ctx_with_retention(&ctx, &mock.url, 1)?;

    let src = plain_backup_file(dir.path());
    backup::upload_backup_to_webdav(&ctx, &src)?;

    let reqs = mock.recorded();
    assert!(
        reqs.iter()
            .any(|r| r.method == "DELETE" && r.path.contains("voucher-backup-20260901")),
        "应先尝试删除被淘汰的旧备份：{reqs:?}"
    );
    assert!(
        !reqs
            .iter()
            .any(|r| r.method == "DELETE" && r.path.contains("voucher-attachments-")),
        "被淘汰备份删除失败时必须跳过附件包回收（fail-closed），实际请求：{reqs:?}"
    );
    Ok(())
}
