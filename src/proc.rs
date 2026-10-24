//! 子进程小工具：`spawn` 之后必须有人 `wait`，否则长驻进程会攒僵尸。
//!
//! 托盘里很多动作是「启动后就不管」的：`notify-send`、`xdg-open`、日志终端。
//! 直接 `Command::spawn()` 然后丢掉 `Child`，子进程退出后会变成 `<defunct>`
//! （实测确认过），devtray 又要常驻好几天，所以统一走这里起一个后台线程 wait。

use std::process::Command;

/// 启动一个不需要等结果的子进程，并在后台线程里 `wait` 掉它。
pub fn spawn_detached(cmd: &mut Command) -> std::io::Result<()> {
    let mut child = cmd.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// 回归测试：子进程退出后必须被 wait 掉，否则 `/proc/<pid>/.../children` 里会一直留着它。
    #[test]
    fn detached_child_is_reaped() {
        let pid = std::process::id();
        let children = format!("/proc/{pid}/task/{pid}/children");
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exit 0"]);
        spawn_detached(&mut cmd).unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let text = std::fs::read_to_string(&children).unwrap_or_default();
            if text.trim().is_empty() {
                return;
            }
            if Instant::now() > deadline {
                panic!("子进程没有被回收，仍留在 children 里：{text}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
