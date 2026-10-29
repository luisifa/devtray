//! 本机 Agent 检测（只读）：二进制、版本、安装来源、对应升级命令。
//!
//! 这一版不执行升级、不联网查最新版本；只把"当前装了什么、从哪装的、该用什么命令升级"
//! 如实列出来。写操作要等 Skills 写路径的门禁一起上线。

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Serialize;

static VERSION_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?").ok());

struct Spec {
    id: &'static str,
    name: &'static str,
    bin: &'static str,
    npm_package: &'static str,
    /// native 安装时使用的自更新命令；None = 不提供
    native_update: Option<&'static str>,
}

const SPECS: &[Spec] = &[
    Spec {
        id: "claude-code",
        name: "Claude Code",
        bin: "claude",
        npm_package: "@anthropic-ai/claude-code",
        native_update: Some("claude update"),
    },
    Spec {
        id: "codex",
        name: "Codex",
        bin: "codex",
        npm_package: "@openai/codex",
        native_update: Some("codex update"),
    },
    Spec {
        id: "pi",
        name: "Pi",
        bin: "pi",
        npm_package: "",
        native_update: Some("pi update self"),
    },
    Spec {
        id: "dsh",
        name: "DSH",
        bin: "dsh",
        npm_package: "@deepseek-ai/dsh",
        native_update: None,
    },
    Spec {
        id: "gemini-cli",
        name: "Gemini CLI",
        bin: "gemini",
        npm_package: "@google/gemini-cli",
        native_update: None,
    },
    Spec {
        id: "opencode",
        name: "OpenCode",
        bin: "opencode",
        npm_package: "opencode-ai",
        native_update: None,
    },
    Spec {
        id: "qwen-code",
        name: "Qwen Code",
        bin: "qwen",
        npm_package: "@qwen-code/qwen-code",
        native_update: None,
    },
    Spec {
        id: "aider",
        name: "Aider",
        bin: "aider",
        npm_package: "",
        native_update: None,
    },
    Spec {
        id: "kimi-cli",
        name: "Kimi Code",
        bin: "kimi",
        npm_package: "",
        native_update: None,
    },
    Spec {
        id: "hermes",
        name: "Hermes Agent",
        bin: "hermes",
        npm_package: "",
        native_update: Some("hermes update"),
    },
];

#[derive(Debug, Clone, Serialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub bin: String,
    pub path: String,
    pub version: Option<String>,
    /// pacman / dpkg / npm / native / unknown
    pub provenance: String,
    /// 例如 pacman 包名 `openai-codex`
    pub package: Option<String>,
    /// 建议的升级命令（只展示，不执行）
    pub update_command: Option<String>,
    pub needs_sudo: bool,
    pub notes: Vec<String>,
}

pub fn detect() -> Vec<AgentInfo> {
    let mut out: Vec<AgentInfo> = SPECS
        .iter()
        .filter_map(|spec| {
            let path = find_in_path(spec.bin)?;
            Some(inspect(spec, path))
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn find_in_path(bin: &str) -> Option<PathBuf> {
    search_dirs()
        .into_iter()
        .map(|dir| dir.join(bin))
        .find(|candidate| {
            candidate
                .metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

// A systemd or desktop launcher does not inherit an interactive shell's PATH.
// Keep explicit PATH entries first; never execute shell startup files to discover tools.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    let home = crate::config::home_dir();
    for relative in [
        "bin",
        ".local/bin",
        ".cargo/bin",
        ".bun/bin",
        ".npm-global/bin",
        ".local/share/pnpm",
        ".local/share/pnpm/bin",
        ".pi/agent/bin",
        ".opencode/bin",
        ".volta/bin",
        ".asdf/shims",
        ".kimi-code/bin",
        ".hermes/venv/bin",
    ] {
        dirs.push(home.join(relative));
    }
    for variable in [
        "NPM_CONFIG_PREFIX",
        "npm_config_prefix",
        "VOLTA_HOME",
        "BUN_INSTALL",
    ] {
        if let Some(value) = std::env::var_os(variable) {
            dirs.push(PathBuf::from(value).join("bin"));
        }
    }
    if let Some(value) = std::env::var_os("PNPM_HOME") {
        dirs.push(PathBuf::from(value));
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|path| seen.insert(path.clone()));
    dirs
}

fn inspect(spec: &Spec, path: PathBuf) -> AgentInfo {
    let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let version = command_output(&path, &["--version"]).and_then(|text| parse_version(&text));
    let mut notes = Vec::new();
    if version.is_none() {
        notes.push("无法从 --version 解析出版本".into());
    }
    let (provenance, package) = provenance(spec, &canonical, &path);
    let (update_command, needs_sudo) = update_command(spec, &provenance, package.as_deref());
    if provenance == "unknown" {
        notes.push("无法判断安装来源；请手动确认升级方式".into());
    }
    AgentInfo {
        id: spec.id.into(),
        name: spec.name.into(),
        bin: spec.bin.into(),
        path: canonical.display().to_string(),
        version,
        provenance,
        package,
        update_command,
        needs_sudo,
        notes,
    }
}

fn command_output(bin: &Path, args: &[&str]) -> Option<String> {
    // Version output can be on stderr. Use files to avoid pipe-buffer deadlocks,
    // and bound each probe so one broken CLI cannot hang the complete inventory.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let file_path =
        std::env::temp_dir().join(format!("devtray-probe-{}-{stamp}", std::process::id()));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&file_path)
        .ok()?;
    let _ = std::fs::remove_file(&file_path);
    let mut dirs = search_dirs();
    if let Some(parent) = bin.parent() {
        dirs.insert(0, parent.to_path_buf());
    }
    let mut child = Command::new(bin)
        .args(args)
        .env("PATH", std::env::join_paths(dirs).ok()?)
        .stdin(Stdio::null())
        .stdout(file.try_clone().ok()?)
        .stderr(file.try_clone().ok()?)
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !status.success() {
        return None;
    }
    use std::io::{Read, Seek};
    let mut file = file;
    file.rewind().ok()?;
    let mut text = String::new();
    file.take(65536).read_to_string(&mut text).ok()?;
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

pub fn parse_version(text: &str) -> Option<String> {
    VERSION_RE
        .as_ref()?
        .find(text)
        .map(|m| m.as_str().to_string())
}

/// 来源判断顺序：系统包（pacman / dpkg）→ npm 全局 → 原生安装目录 → 未知。
fn provenance(spec: &Spec, canonical: &Path, original: &Path) -> (String, Option<String>) {
    if let Some(pkg) = pacman_owner(canonical) {
        return ("pacman".into(), Some(pkg));
    }
    if let Some(pkg) = dpkg_owner(canonical) {
        return ("dpkg".into(), Some(pkg));
    }
    let path_text = canonical.to_string_lossy();
    if path_text.contains("/node_modules/") {
        return ("npm".into(), Some(spec.npm_package.to_string()));
    }
    let home = crate::config::home_dir();
    let native_root = home.join(".local/share");
    let versioned = path_text.contains("/versions/");
    let local_link = original.starts_with(home.join(".local/bin")) && canonical != original;
    if (canonical.starts_with(&native_root) && versioned)
        || local_link
        || canonical.starts_with(home.join(".pi"))
    {
        return ("native".into(), None);
    }
    ("unknown".into(), None)
}

fn pacman_owner(path: &Path) -> Option<String> {
    let out = Command::new("pacman")
        .args(["-Qoq", &path.to_string_lossy()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

fn dpkg_owner(path: &Path) -> Option<String> {
    let out = Command::new("dpkg")
        .args(["-S", &path.to_string_lossy()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.split(':')
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn update_command(spec: &Spec, provenance: &str, package: Option<&str>) -> (Option<String>, bool) {
    match provenance {
        "pacman" => (package.map(|pkg| format!("sudo pacman -S {pkg}")), true),
        "dpkg" => (
            package.map(|pkg| format!("sudo apt install --only-upgrade {pkg}")),
            true,
        ),
        "npm" if !spec.npm_package.is_empty() => {
            (Some(format!("npm i -g {}@latest", spec.npm_package)), false)
        }
        "native" => (spec.native_update.map(str::to_string), false),
        _ => (None, false),
    }
}

pub fn command(args: &[String]) -> ExitCode {
    let mut json_flag = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json_flag = true,
            other => {
                eprintln!("未知选项：{other}");
                return ExitCode::FAILURE;
            }
        }
    }
    let agents = detect();
    let text = if json_flag {
        match serde_json::to_string_pretty(&agents) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        render(&agents)
    };
    println!("{text}");
    ExitCode::SUCCESS
}

fn render(agents: &[AgentInfo]) -> String {
    if agents.is_empty() {
        return "没有检测到已知 Agent（claude / codex / gemini / opencode / qwen / aider）".into();
    }
    let mut out = String::new();
    for agent in agents {
        out.push_str(&format!(
            "{}  {}\n  版本 {}   来源 {}{}\n  路径 {}\n  升级 {}\n",
            agent.name,
            agent.id,
            agent.version.as_deref().unwrap_or("未知"),
            agent.provenance,
            agent
                .package
                .as_ref()
                .map(|p| format!(" ({p})"))
                .unwrap_or_default(),
            agent.path,
            agent.update_command.as_deref().unwrap_or("—")
        ));
        for note in &agent.notes {
            out.push_str(&format!("  注意：{note}\n"));
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions_from_common_agent_output() {
        assert_eq!(
            parse_version("2.1.312 (Claude Code)").as_deref(),
            Some("2.1.312")
        );
        assert_eq!(
            parse_version("codex-cli 0.165.0").as_deref(),
            Some("0.165.0")
        );
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn update_commands_follow_provenance() {
        let spec = &SPECS[1]; // codex
        assert_eq!(
            update_command(spec, "pacman", Some("openai-codex")),
            (Some("sudo pacman -S openai-codex".into()), true)
        );
        assert_eq!(
            update_command(spec, "npm", Some("@openai/codex")),
            (Some("npm i -g @openai/codex@latest".into()), false)
        );
        assert_eq!(
            update_command(spec, "native", None),
            (Some("codex update".into()), false)
        );
        assert_eq!(update_command(spec, "unknown", None), (None, false));
    }
}
