//! 静态文件与受保护资源：前端 SPA + 附件/导出/备份的按需读取。
//!
//! 安全边界：`/api/asset` 与 `/api/download` 只允许读取**数据目录内**的文件
//! （附件、导出、备份、临时目录），先 canonicalize 再比对前缀，
//! 因此 `?path=../../etc/passwd` 这类路径穿越会被直接拒绝。

use std::path::{Path, PathBuf};

use crate::state::{ApiErr, AppState};

pub fn mime_of(path: &Path) -> &'static str {
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("gif") => "image/gif",
        Some("pdf") => "application/pdf",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("txt") | Some("csv") => "text/plain; charset=utf-8",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

/// 校验并解析数据目录内的绝对路径
pub fn resolve_in_data_dir(st: &AppState, raw: &str) -> Result<PathBuf, ApiErr> {
    if raw.is_empty() {
        return Err(ApiErr::bad_request("invalid", "缺少 path 参数"));
    }
    let p = PathBuf::from(raw);
    let canon = p
        .canonicalize()
        .map_err(|_| ApiErr::not_found("文件不存在"))?;
    let base = st
        .ctx
        .paths
        .base
        .canonicalize()
        .map_err(|e| ApiErr::internal(format!("数据目录不可用：{e}")))?;
    if !canon.starts_with(&base) {
        return Err(ApiErr::bad_request("invalid", "只能访问数据目录内的文件"));
    }
    Ok(canon)
}

/// 前端静态文件：`/` 与未知路径回 index.html（SPA 路由）。
///
/// 唯一例外是 `/assets/**`：这些是 Vite 产物的带哈希文件名，**不存在就必须 404**。
/// 早先一律回退 index.html，导致刚 build 完、浏览器还拿着旧 index 时会去取旧哈希的 chunk，
/// 服务端却回一坨 HTML —— 浏览器按 MIME 校验直接报「Failed to fetch dynamically imported module」，
/// 页面白屏且极难看出原因（真实踩坑：2026-11-18 网页版走查）。
///
/// 返回值第三项表示能否长缓存：assets 带内容哈希，可以 immutable；index.html 必须每次回源。
pub async fn read_spa(st: &AppState, path: &str) -> Option<(Vec<u8>, &'static str, bool)> {
    let rel = path.trim_start_matches('/');
    if rel.contains("..") {
        return None;
    }
    let candidate = if rel.is_empty() {
        st.web_dir.join("index.html")
    } else {
        st.web_dir.join(rel)
    };
    let assets = rel.starts_with("assets/");
    let target = if candidate.is_file() {
        candidate
    } else if assets {
        return None;
    } else {
        st.web_dir.join("index.html")
    };
    let bytes = tokio::fs::read(&target).await.ok()?;
    let is_index = target
        .file_name()
        .map(|n| n == "index.html")
        .unwrap_or(false);
    let mime = if is_index {
        "text/html; charset=utf-8"
    } else {
        mime_of(&target)
    };
    Some((bytes, mime, assets && !is_index))
}
