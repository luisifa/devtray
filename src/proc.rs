//! 子进程小工具：`spawn` 之后必须有人 `wait`，否则长驻进程会攒僵尸。
//!
//! 托盘里很多动作是「启动后就不管」的：`notify-send`、`xdg-open`、日志终端。
//! 直接 `Command::spawn()` 然后丢掉 `Child`，子进程退出后会变成 `<defunct>`
//! （实测确认过），devtray 又要常驻好几天，所以统一走这里起一个后台线程 wait。

use std::process::Command;

/// Feed an embedded program through stdin instead of Linux's bounded argv entries.
pub fn spawn_detached_with_input(cmd: &mut Command, input: Vec<u8>) -> std::io::Result<()> {
    use std::io::Write;
    let mut child = cmd.stdin(std::process::Stdio::piped()).spawn()?;
    // A one-shot CLI may exit as soon as this function returns, terminating any
    // writer thread. Deliver and close stdin before detaching the waiter.
    let result = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("missing child stdin"))
        .and_then(|mut stdin| stdin.write_all(&input));
    if let Err(error) = result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

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

    #[test]
    fn embedded_program_larger_than_one_argument_reaches_child() {
        let path = std::env::temp_dir().join(format!("devtray-input-{}.txt", std::process::id()));
        let output = std::fs::File::create(&path).unwrap();
        let mut command = Command::new("wc");
        command.arg("-c").stdout(output);
        spawn_detached_with_input(&mut command, vec![b'x'; 256 * 1024]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let count = std::fs::read_to_string(&path).unwrap_or_default();
            if count.trim() == "262144" {
                break;
            }
            assert!(Instant::now() < deadline, "stdin payload was not delivered");
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::remove_file(path).unwrap();
    }

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
