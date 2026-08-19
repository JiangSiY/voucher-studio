//! 微信收件夹监听（对应 v4.3 的 `wechat.py`）
//!
//! notify 事件监听 + 「文件写完再入库」稳定判据（微信图片先临时名再改名，
//! 需要等尺寸稳定）；轮询兜底待前端设置页提供开关后接入。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

use crate::services::imaging;
use crate::{CoreError, Result};

/// 启动目录监听线程。发现新图片（写入稳定后）经 `tx` 推送绝对路径。
/// 返回的 JoinHandle 持有 watcher；调用方 detach 即可（进程退出自动结束）。
pub fn spawn_watch(dir: PathBuf, tx: Sender<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Err(CoreError::Invalid(format!(
            "监听目录不存在: {}",
            dir.display()
        )));
    }

    let (evt_tx, evt_rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let _ = evt_tx.send(res);
    })?;
    watcher.watch(&dir, RecursiveMode::NonRecursive)?;

    std::thread::Builder::new()
        .name("wechat-watch".into())
        .spawn(move || {
            // watcher 必须存活于线程内
            let _watcher = watcher;
            let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
            loop {
                match evt_rx.recv_timeout(Duration::from_millis(400)) {
                    Ok(Ok(event)) => {
                        for p in event.paths {
                            if is_image_path(&p) && p.is_file() {
                                pending.insert(p, Instant::now());
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        log::warn!("目录监听异常: {e}");
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
                // 稳定判据：1.5s 内无新事件且文件尺寸两次采样一致
                let now = Instant::now();
                let settled: Vec<PathBuf> = pending
                    .iter()
                    .filter(|(_, t)| now.duration_since(**t) > Duration::from_millis(1500))
                    .map(|(p, _)| p.clone())
                    .collect();
                for p in settled {
                    pending.remove(&p);
                    if size_stable(&p) {
                        let _ = tx.send(p);
                    }
                }
            }
        })?;
    Ok(())
}

fn is_image_path(p: &Path) -> bool {
    imaging::is_supported(p)
}

fn size_stable(p: &Path) -> bool {
    let s1 = std::fs::metadata(p).map(|m| m.len()).ok();
    std::thread::sleep(Duration::from_millis(300));
    let s2 = std::fs::metadata(p).map(|m| m.len()).ok();
    matches!((s1, s2), (Some(a), Some(b)) if a == b && a > 0)
}
