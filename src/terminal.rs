//! 终端探测链：`$TERMINAL` → `xdg-terminal-exec` → 已知终端表 → 配置覆盖。
//! 表里那几行是为了抹平各家「执行一条命令」的参数差异（`-e` / `--` / `start --`）。

use std::process::Command;

/// (可执行名, 追加命令前的固定参数)
const TABLE: &[(&str, &[&str])] = &[
    ("kitty", &["-e"]),
    ("alacritty", &["-e"]),
    ("ghostty", &["-e"]),
    ("foot", &["-e"]),
    ("xfce4-terminal", &["-e"]),
    ("konsole", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("wezterm", &["start", "--"]),
];

pub fn which(prog: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    path.split(':')
        .any(|d| !d.is_empty() && std::path::Path::new(d).join(prog).is_file())
}

fn flag_for(prog: &str) -> &'static [&'static str] {
    TABLE
        .iter()
        .find(|(name, _)| *name == prog)
        .map(|(_, flags)| *flags)
        .unwrap_or(&["-e"])
}

/// 解析出「终端程序 + 固定参数」前缀；解析不到返回 None
pub fn resolve(configured: &str) -> Option<Vec<String>> {
    let explicit = if !configured.trim().is_empty() {
        Some(configured.trim().to_string())
    } else {
        std::env::var("TERMINAL")
            .ok()
            .filter(|s| !s.trim().is_empty())
    };
    if let Some(spec) = explicit {
        let words: Vec<String> = spec.split_whitespace().map(str::to_string).collect();
        let (prog, extra) = words.split_first()?;
        if !which(prog) {
            return None;
        }
        let mut argv = vec![prog.clone()];
        // 用户已经把启动参数写全了就别再追加，避免 `wezterm start -- start --`
        let has_flag = extra.iter().any(|a| a == "-e" || a == "--" || a == "start");
        argv.extend(extra.iter().cloned());
        if !has_flag {
            argv.extend(flag_for(prog).iter().map(|s| s.to_string()));
        }
        return Some(argv);
    }
    if which("xdg-terminal-exec") {
        return Some(vec!["xdg-terminal-exec".into()]);
    }
    TABLE
        .iter()
        .find(|(name, _)| which(name))
        .map(|(name, flags)| {
            let mut v = vec![(*name).to_string()];
            v.extend(flags.iter().map(|s| s.to_string()));
            v
        })
}

/// 在前台终端里跑一条命令（进程立刻返回，由终端自己持有）
pub fn spawn_in(base: &[String], cmd: &[String]) -> Result<(), String> {
    let (prog, args) = base.split_first().ok_or("终端命令为空")?;
    let mut c = Command::new(prog);
    c.args(args).args(cmd);
    // A GUI-triggered CLI exits immediately; the terminal must not retain its
    // QProcess pipes (or a caller waiting for EOF would hang until it closes).
    c.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::proc::spawn_detached(&mut c).map_err(|e| format!("启动终端 {prog} 失败：{e}"))
}

/// 编辑器的 argv（不含文件）：$EDITOR 优先，否则 nvim/vim/nano
pub fn editor() -> Vec<String> {
    if let Ok(e) = std::env::var("EDITOR") {
        let words: Vec<String> = e.split_whitespace().map(str::to_string).collect();
        if !words.is_empty() {
            return words;
        }
    }
    ["nvim", "vim", "nano"]
        .iter()
        .find(|p| which(p))
        .map(|p| vec![(*p).to_string()])
        .unwrap_or_else(|| vec!["vi".into()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_explicit_terminal() {
        let got = resolve("kitty --single-instance");
        if which("kitty") {
            assert_eq!(got.unwrap(), vec!["kitty", "--single-instance", "-e"]);
        }
    }

    #[test]
    fn no_duplicate_flag_when_user_specified() {
        let got = resolve("wezterm start --");
        if which("wezterm") {
            assert_eq!(got.unwrap(), vec!["wezterm", "start", "--"]);
        }
    }
}
