//! tmux 后端：一个服务 = 一个 tmux 会话（`devtray-<项目>-<服务>`）。
//!
//! 会话创建分四步，缺一不可（这是实测出来的，顺序有讲究）：
//!   ① `new-session` 起一个占位 shell —— 为什么不用真命令？因为 tmux 默认
//!      `exit-empty on`，真命令若瞬间退出，session 立刻销毁、server 跟着关，
//!      后面几步全落空（实测踩过）。
//!   ② 设 `remain-on-exit on`：进程死了 pane 还留着，才能读到退出码。
//!   ③ `pipe-pane` 挂独立日志采集器，按大小轮转并保留重启前的归档。
//!   ④ `respawn-pane -k` 换真命令，无竞态。

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::config::{Target, shell_quote};

/// 占位 shell：活到被 respawn 掉为止
const PLACEHOLDER: &str = "sleep 86400";
const HISTORY_LIMIT: &str = "50000";
const SIGINT_GRACE_STEPS: usize = 6; // 6 × 500ms = 3s

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// 没有对应 tmux 会话
    NotStarted,
    /// 活着，且没配置任何就绪探测
    Running,
    /// 活着，但就绪探测还没通过
    Starting,
    /// 活着且探测通过
    Ready,
    /// pane 已死（保留中）；None = 被信号杀死（例如我们自己的停止）
    Exited(Option<i32>),
}

impl State {
    /// 「进程还活着」——决定菜单里启动/停止哪个可用
    pub fn is_live(&self) -> bool {
        matches!(self, State::Running | State::Starting | State::Ready)
    }

    /// 非零退出码才算失败（信号退出不算，避免把自己的停止报成崩溃）
    pub fn is_failure(&self) -> bool {
        matches!(self, State::Exited(Some(code)) if *code != 0)
    }

    /// 给人看的（菜单、CLI 输出）
    pub fn text(&self) -> String {
        match self {
            State::NotStarted => "○ 未启动".into(),
            State::Running => "● 运行中".into(),
            State::Starting => "◌ 启动中".into(),
            State::Ready => "● 就绪".into(),
            State::Exited(Some(0)) => "✓ 已结束 (0)".into(),
            State::Exited(Some(code)) => format!("✕ 失败 (exit {code})"),
            State::Exited(None) => "■ 已停止".into(),
        }
    }

    /// 给机器读的稳定标识（MCP / JSON 用，改了就是破坏性变更）
    pub fn token(&self) -> &'static str {
        match self {
            State::NotStarted => "not_started",
            State::Running => "running",
            State::Starting => "starting",
            State::Ready => "ready",
            State::Exited(Some(0)) => "exited_ok",
            State::Exited(Some(_)) => "exited_failed",
            State::Exited(None) => "exited_signal",
        }
    }
}

fn run(args: &[&str]) -> Result<String, String> {
    let out = Command::new("tmux")
        .args(args)
        .output()
        .map_err(|e| format!("执行 tmux 失败：{e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "tmux {} 失败：{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// 一个会话的现场：是否已死、退出码、pane 的进程号
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneInfo {
    pub dead: bool,
    pub status: Option<i32>,
    pub pid: u32,
}

pub type Snapshot = HashMap<String, PaneInfo>;

/// 一次 `list-panes -a` 拿到全部会话状态（每个 tick 只起一个进程）
pub fn snapshot() -> Snapshot {
    let mut map = HashMap::new();
    let Ok(out) = run(&[
        "list-panes",
        "-a",
        "-F",
        "#{session_name}\t#{pane_dead}\t#{pane_dead_status}\t#{pane_pid}",
    ]) else {
        return map; // 没有 tmux server = 什么都没跑
    };
    for line in out.lines() {
        let mut it = line.splitn(4, '\t');
        let (Some(name), Some(dead), status, pid) = (it.next(), it.next(), it.next(), it.next())
        else {
            continue;
        };
        map.insert(
            name.to_string(),
            PaneInfo {
                dead: dead == "1",
                status: status.and_then(|s| s.trim().parse::<i32>().ok()),
                pid: pid.and_then(|s| s.trim().parse().ok()).unwrap_or(0),
            },
        );
    }
    map
}

/// 就绪判定必须是**有记忆的**：日志会一直滚，`ready in` 这类 banner 很快被顶出可见区，
/// 每次都重新匹配就会在「就绪/启动中」之间抖动（实测踩过）。所以一旦探测通过就锁住，
/// 只有 pane 的 pid 变了（服务被重建）才失效。
#[derive(Default)]
pub struct Prober {
    latched: HashMap<String, u32>,
}

impl Prober {
    pub fn state_of(&mut self, t: &Target, snap: &Snapshot) -> State {
        let Some(info) = snap.get(&t.session) else {
            self.latched.remove(&t.session);
            return State::NotStarted;
        };
        if info.dead {
            self.latched.remove(&t.session);
            return State::Exited(info.status);
        }
        if t.service.port.is_none() && t.ready_re.is_none() {
            return State::Running;
        }
        if self.latched.get(&t.session) == Some(&info.pid) {
            return State::Ready;
        }
        if probe_ok(t) {
            self.latched.insert(t.session.clone(), info.pid);
            State::Ready
        } else {
            State::Starting
        }
    }
}

/// 配了 port 或 ready 才算探测通过（两者是「或」）
fn probe_ok(t: &Target) -> bool {
    if let Some(port) = t.service.port
        && tcp_open(port)
    {
        return true;
    }
    if let Some(re) = &t.ready_re
        && let Some(text) = capture(&t.session)
        && re.is_match(&text)
    {
        return true;
    }
    false
}

fn tcp_open(port: u16) -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

/// 抓**全量历史**而不是可见区域：启动 banner 会被后续输出顶出屏幕
pub fn capture(session: &str) -> Option<String> {
    run(&["capture-pane", "-p", "-S", "-", "-t", session]).ok()
}

pub fn exists(session: &str) -> bool {
    run(&["has-session", "-t", session]).is_ok()
}

/// 启动；已存在会话则先清掉（含残留的死 pane）
pub fn start(t: &Target) -> Result<(), String> {
    if exists(&t.session) {
        let _ = run(&["kill-session", "-t", &t.session]);
    }
    crate::logs::prepare(&t.log_path)?;

    let cwd = t.service.cwd.clone();
    let cwd = crate::config::expand_home(&cwd);
    let cwd = cwd.to_string_lossy().into_owned();

    run(&[
        "new-session",
        "-d",
        "-s",
        &t.session,
        "-c",
        &cwd,
        PLACEHOLDER,
    ])?;
    run(&["set-option", "-w", "-t", &t.session, "remain-on-exit", "on"])?;
    run(&[
        "set-option",
        "-t",
        &t.session,
        "history-limit",
        HISTORY_LIMIT,
    ])?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let pipe = format!(
        "{} _log-writer {}",
        shell_quote(&exe.to_string_lossy()),
        shell_quote(&t.log_path.to_string_lossy())
    );
    run(&["pipe-pane", "-t", &t.session, "-o", &pipe])?;
    run(&[
        "respawn-pane",
        "-k",
        "-t",
        &t.session,
        "-c",
        &cwd,
        "sh",
        "-lc",
        &t.service.full_cmd(),
    ])?;
    Ok(())
}

/// 优雅停止：先 C-c，给 3s；不退就 kill-session
pub fn stop(t: &Target) -> Result<(), String> {
    if !exists(&t.session) {
        return Ok(());
    }
    let _ = run(&["send-keys", "-t", &t.session, "C-c"]);
    for _ in 0..SIGINT_GRACE_STEPS {
        std::thread::sleep(Duration::from_millis(500));
        if !exists(&t.session) || pane_dead(&t.session) {
            break;
        }
    }
    run(&["kill-session", "-t", &t.session]).map(|_| ())
}

pub fn restart(t: &Target) -> Result<(), String> {
    stop(t)?;
    start(t)
}

pub fn pane_dead(session: &str) -> bool {
    run(&["list-panes", "-t", session, "-F", "#{pane_dead}"])
        .map(|s| s.trim() == "1")
        .unwrap_or(true)
}

/// 读日志文件尾部（用于失败通知里塞错误摘要）
pub fn error_snippet(log: &Path) -> Option<String> {
    const TAIL: usize = 64 * 1024;
    let mut file = fs::File::open(log).ok()?;
    let size = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(size.saturating_sub(TAIL as u64)))
        .ok()?;
    let mut bytes = Vec::new();
    file.take(TAIL as u64).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    // 按字节取尾部，但起点必须落在 UTF-8 字符边界上：日志里有中文/emoji 时，
    // `len - TAIL` 很可能切在一个多字节字符中间，直接切片会 panic。
    let mut start = text.len().saturating_sub(TAIL);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let tail = &text[start..];
    let needles = [
        "error[",
        "error:",
        "Error:",
        "error ",
        "ERROR",
        "panic",
        "panicked",
        "EADDRINUSE",
        "Traceback",
        "FAILED",
        "failed to",
    ];
    let hits: Vec<String> = tail
        .lines()
        .filter(|l| needles.iter().any(|n| l.contains(n)))
        .map(|l| l.trim().chars().take(140).collect())
        .filter(|l: &String| !l.is_empty())
        .collect();
    if hits.is_empty() {
        return None;
    }
    Some(hits[hits.len().saturating_sub(2)..].join(" | "))
}

/// 点「查看日志」时要在终端里跑的命令
pub fn attach_cmd(t: &Target) -> Vec<String> {
    vec![
        "tmux".into(),
        "attach".into(),
        "-t".into(),
        t.session.clone(),
    ]
}

/// 我们的会话里是否还有任何一个在跑（退出时用来提示）
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_classification() {
        assert!(State::Ready.is_live());
        assert!(!State::NotStarted.is_live());
        assert!(State::Exited(Some(101)).is_failure());
        assert!(!State::Exited(Some(0)).is_failure());
        assert!(!State::Exited(None).is_failure()); // 信号退出 = 我们自己停的
    }

    /// 日志尾部按字节切，起点落在多字节字符中间时不能 panic（真实日志里有中文）。
    #[test]
    fn error_snippet_survives_utf8_boundary() {
        let dir = std::env::temp_dir().join("devtray-test-error-snippet");
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("x.log");
        // 131071 字节：len - 64KiB = 65535，正好落在“中”的三字节中间
        let mut text = "a".repeat(65534);
        text.push('中');
        text.push_str("error: boom\n");
        text.push_str(&"b".repeat(65522));
        assert_eq!(text.len(), 131071);
        assert_eq!(text.len() - 65536, 65535);
        assert!(!text.is_char_boundary(65535));
        fs::write(&log, &text).unwrap();

        let snippet = error_snippet(&log).expect("应能从尾部找到 error 行");
        assert!(snippet.contains("error: boom"), "{snippet}");
        fs::remove_file(&log).ok();
    }
}
