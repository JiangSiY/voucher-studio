//! 微信收件夹监听（对应 v4.3 的 `wechat.py`）
//!
//! notify 事件监听 + 「写入稳定」双判据：事件静默期 + 尺寸双采样
//! （微信图片先临时名再改名、大图持续写入，需等尺寸稳定再入库）。
//!
//! 递归监听：微信 `FileStorage\Image` 按月份分子目录存放，非递归会漏掉子目录新图。
//! 轮询兜底待前端设置页提供开关后接入（当前 mode 固定 events）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

use crate::services::imaging;
use crate::{CoreError, Result};

/// 事件静默期：路径最后一次事件后静默这么久，才进入尺寸采样
const QUIET_PERIOD: Duration = Duration::from_millis(1500);
/// 主循环节拍（也是尺寸两次采样的最小间隔）
const TICK: Duration = Duration::from_millis(400);
/// 采样超过该时长仍不稳定（持续变化/文件消失）→ 放弃跟踪，防 pending 泄漏
const GIVE_UP_AFTER: Duration = Duration::from_secs(30);

/// 停止句柄：`stop()` 后监听线程在一个 TICK 内退出并释放 watcher
pub struct StopHandle(Sender<()>);

impl StopHandle {
    pub fn stop(&self) {
        let _ = self.0.send(());
    }
}

/// 启动目录监听线程（递归）。发现新图片（写入稳定后）经 `tx` 推送绝对路径。
/// 返回停止句柄；配置变更时先 `stop()` 旧的再重启新的，实现热重载。
pub fn spawn_watch(dir: PathBuf, tx: Sender<PathBuf>) -> Result<StopHandle> {
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
    watcher.watch(&dir, RecursiveMode::Recursive)?;
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

    std::thread::Builder::new()
        .name("wechat-watch".into())
        .spawn(move || {
            // watcher 必须存活于线程内
            let _watcher = watcher;
            let mut tracker = SettleTracker::default();
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                match evt_rx.recv_timeout(TICK) {
                    Ok(Ok(event)) => {
                        // rename 事件的 paths 含改名前后两个路径：
                        // 旧路径已不存在（is_file 为假）自然跳过，新路径进入跟踪
                        let now = Instant::now();
                        for p in event.paths {
                            if is_image_path(&p) && p.is_file() {
                                tracker.observe(p, now);
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        log::warn!("目录监听异常: {e}");
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
                for p in tracker.poll(Instant::now()) {
                    // 消费端已退出（应用关闭）→ 结束监听，避免空转
                    if tx.send(p).is_err() {
                        return;
                    }
                }
            }
        })?;
    Ok(StopHandle(stop_tx))
}

fn is_image_path(p: &Path) -> bool {
    imaging::is_supported(p)
}

/// 「写入稳定」追踪器：
/// 1. 路径产生新事件 → 刷新计时并清空采样；
/// 2. 静默期满（>QUIET_PERIOD）→ 每轮 TICK 采样一次文件尺寸；
/// 3. 连续两次采样尺寸一致且非零 → 判定稳定，推送；
/// 4. 跟踪超过 GIVE_UP_AFTER 仍未稳定 → 放弃（防泄漏）。
///
/// 全程零 sleep，不会阻塞监听循环。
#[derive(Default)]
struct SettleTracker {
    last_event: HashMap<PathBuf, Instant>,
    /// 上一次采样的尺寸（None = 尚未采到/文件不存在）
    sizes: HashMap<PathBuf, Option<u64>>,
}

impl SettleTracker {
    /// 记录一次事件（`now` 由调用方传入，便于测试用合成时间线）
    fn observe(&mut self, path: PathBuf, now: Instant) {
        self.last_event.insert(path.clone(), now);
        // 新事件重置尺寸采样（写入仍在进行）
        self.sizes.remove(&path);
    }

    fn poll(&mut self, now: Instant) -> Vec<PathBuf> {
        let mut settled = Vec::new();
        let paths: Vec<PathBuf> = self.last_event.keys().cloned().collect();
        for path in paths {
            let last = self.last_event[&path];
            let elapsed = now.duration_since(last);
            if elapsed < QUIET_PERIOD {
                continue;
            }
            let cur = std::fs::metadata(&path).map(|m| m.len()).ok();
            let prev = self.sizes.get(&path).copied().flatten();
            match (prev, cur) {
                (Some(a), Some(b)) if a == b && a > 0 => {
                    settled.push(path.clone());
                    self.remove(&path);
                    continue;
                }
                _ => {
                    self.sizes.insert(path.clone(), cur);
                }
            }
            if elapsed > GIVE_UP_AFTER {
                self.remove(&path);
            }
        }
        settled
    }

    fn remove(&mut self, p: &Path) {
        self.last_event.remove(p);
        self.sizes.remove(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settle_requires_quiet_then_two_equal_samples() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.jpg");
        std::fs::write(&f, b"image-bytes").unwrap();

        let mut t = SettleTracker::default();
        let t0 = Instant::now();
        t.observe(f.clone(), t0);
        // 静默期内不采样
        assert!(t.poll(t0 + Duration::from_millis(100)).is_empty());
        // 静默期满：第一次采样，尚不能判定
        assert!(t.poll(t0 + QUIET_PERIOD + TICK).is_empty());
        // 下一轮采样尺寸一致 → 稳定
        let out = t.poll(t0 + QUIET_PERIOD + TICK + TICK);
        assert_eq!(out, vec![f.clone()]);
        assert_eq!(t.last_event.len(), 0, "稳定后应移出跟踪表");
    }

    #[test]
    fn new_event_resets_sampling() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("b.jpg");
        std::fs::write(&f, b"first").unwrap();

        let mut t = SettleTracker::default();
        let t0 = Instant::now();
        t.observe(f.clone(), t0);
        // 采样一（尺寸 = 5）
        let _ = t.poll(t0 + QUIET_PERIOD + TICK);
        // 采样二之前文件继续写入并产生新事件 → 计时与采样都重置
        std::fs::write(&f, b"second-longer").unwrap();
        let ev2 = t0 + QUIET_PERIOD + TICK + Duration::from_millis(1);
        t.observe(f.clone(), ev2);
        // 新事件静默期内：不采样不产出
        assert!(t.poll(ev2 + TICK).is_empty());
        // 静默期满：重新采样一（尺寸 = 13）
        assert!(t.poll(ev2 + QUIET_PERIOD + TICK).is_empty());
        // 采样二一致 → 稳定
        let out = t.poll(ev2 + QUIET_PERIOD + TICK + TICK);
        assert_eq!(out, vec![f.clone()]);
        assert_eq!(t.last_event.len(), 0);
    }

    #[test]
    fn stuck_or_missing_files_expire() {
        // 文件不存在（被改名走/删除）→ 不会稳定，超时后放弃，防止 pending 泄漏
        let f = PathBuf::from("/nonexistent/dir/c.jpg");
        let mut t = SettleTracker::default();
        let t0 = Instant::now();
        t.observe(f, t0);
        // 静默期内持续跟踪
        assert!(t.poll(t0 + TICK).is_empty());
        assert_eq!(t.last_event.len(), 1);
        // 远超放弃时限 → 过期清除
        assert!(t.poll(t0 + GIVE_UP_AFTER + TICK).is_empty());
        assert_eq!(t.last_event.len(), 0);
    }

    #[test]
    fn empty_file_never_settles_but_expires() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("d.jpg");
        std::fs::write(&f, b"").unwrap(); // 零字节（占位文件）

        let mut t = SettleTracker::default();
        let t0 = Instant::now();
        t.observe(f.clone(), t0);
        let _ = t.poll(t0 + QUIET_PERIOD + TICK);
        // 零字节不判稳定（a > 0 条件）
        assert!(t.poll(t0 + QUIET_PERIOD + TICK + TICK).is_empty());
        assert_eq!(t.last_event.len(), 1, "零字节应继续跟踪而非误报稳定");
        // 超时放弃
        assert!(t.poll(t0 + GIVE_UP_AFTER + TICK).is_empty());
        assert_eq!(t.last_event.len(), 0);
    }
}
