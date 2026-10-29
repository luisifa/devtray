//! 跨进程互斥：清理与启动 / 停止 / 重启共用一把锁。
//!
//! 为什么不用"锁文件 + 时间戳"判断陈旧：下载、网络卡顿或挂起超过阈值时，原操作可能仍在跑，
//! 按年龄解锁会放进第二个写进程。这里用 `File::try_lock`（flock），锁由持有进程的内核文件描述
//! 持有，进程崩溃或退出时自动释放，没有"陈旧"这个概念。

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::{Duration, Instant};

/// 持锁期间保持文件描述符打开；Drop 时关闭文件，锁随之释放。
pub struct Guard {
    _file: File,
}

pub fn acquire(timeout: Duration) -> Result<Guard, String> {
    acquire_at(&crate::config::state_dir().join("ops.lock"), timeout)
}

pub fn acquire_at(path: &Path, timeout: Duration) -> Result<Guard, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建 {} 失败：{e}", dir.display()))?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| format!("打开 {} 失败：{e}", path.display()))?;
    let deadline = Instant::now() + timeout;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Guard { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err("有清理或启停操作正在进行，请稍后重试".into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(format!("ops.lock 加锁失败：{e}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_and_released_on_drop() {
        let dir = std::env::temp_dir().join(format!(
            "devtray-ops-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ops.lock");

        let first = acquire_at(&path, Duration::ZERO).expect("首次加锁应成功");
        assert!(
            acquire_at(&path, Duration::ZERO).is_err(),
            "同一路径的第二次加锁必须失败"
        );

        drop(first);
        // 释放后给内核/文件系统一点时间；生产路径也有 5s 重试，这里不该用零超时。
        if let Err(e) = acquire_at(&path, Duration::from_secs(2)) {
            panic!("释放后应能重新加锁：{e}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
