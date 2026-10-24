//! 桌面通知：直接调 `notify-send`（noctalia 已提供 org.freedesktop.Notifications）。
//! 不引入 zbus 直连版本，是为了避免和 ksni 抢 zbus 的 runtime feature。

use std::process::Command;

pub fn available() -> bool {
    crate::terminal::which("notify-send")
}

pub fn send(summary: &str, body: &str) {
    if !available() {
        return;
    }
    let mut cmd = Command::new("notify-send");
    cmd.arg("--app-name=devtray")
        .arg("--icon=utilities-terminal")
        .arg(summary)
        .arg(body);
    // 必须有人 wait，否则通知进程退出后会变僵尸（托盘常驻，会越攒越多）
    let _ = crate::proc::spawn_detached(&mut cmd);
}
