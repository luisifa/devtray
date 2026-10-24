//! 配置解析与校验：`~/.config/devtray/config.toml` → 运行期 `Target` 列表。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Deserialize;

pub const SESSION_PREFIX: &str = "devtray-";

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 托盘图标/通知的轮询间隔（菜单内容不靠它，靠 about_to_show 拉取）
    pub poll_ms: u64,
    pub notify: bool,
    pub notify_debounce_s: u64,
    /// 留空 = 自动探测（$TERMINAL → xdg-terminal-exec → kitty → …）
    pub terminal: String,
    /// 托盘图标名（放进 hicolor 主题的自定义图标名，或实测可辨的系统图标名）
    pub icon_ok: String,
    pub icon_starting: String,
    pub icon_error: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            poll_ms: 2000,
            notify: true,
            notify_debounce_s: 60,
            terminal: String::new(),
            icon_ok: crate::icons::DEFAULT_OK.into(),
            icon_starting: crate::icons::DEFAULT_STARTING.into(),
            icon_error: crate::icons::DEFAULT_ERROR.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub settings: Settings,
    #[serde(default, rename = "project")]
    pub projects: Vec<Project>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub name: String,
    #[serde(default, rename = "service")]
    pub services: Vec<Service>,
}

/// 单个被托管的服务。字段顺序/取值即对外契约，改动需同步 README。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Service {
    pub name: String,
    pub cwd: String,
    pub cmd: String,
    #[serde(default)]
    pub autostart: bool,
    /// 可选：TCP 探活端口
    pub port: Option<u16>,
    /// 可选：日志正则，命中即视为就绪
    pub ready: Option<String>,
    /// 可选：预览地址（前端 dev server 的 localhost 地址）。留空则从日志/port 里认
    pub url: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// 留空 = ~/.local/state/devtray/logs/<project>-<service>.log
    pub log_file: Option<String>,
}

impl Service {
    /// 真正交给 tmux 执行的命令串：env 前缀 + 用户命令
    pub fn full_cmd(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.env {
            s.push_str(&format!("export {}={}; ", k, shell_quote(v)));
        }
        s.push_str(&self.cmd);
        s
    }

    /// 影响运行结果的字段（用于判断「配置已变，需重启生效」）。
    /// 注意 `url` 故意不在内：它只影响「打开预览」按钮，改完立刻生效，不需要重启服务。
    pub fn runtime_eq(&self, other: &Self) -> bool {
        self.cwd == other.cwd
            && self.cmd == other.cmd
            && self.env == other.env
            && self.port == other.port
            && self.ready == other.ready
    }
}

/// 校验后可直接使用的服务描述
#[derive(Debug, Clone)]
pub struct Target {
    pub project: String,
    pub service: Service,
    pub session: String,
    pub log_path: PathBuf,
    pub ready_re: Option<Regex>,
}

pub fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return Path::new(&home).join(rest);
    }
    PathBuf::from(p)
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub fn state_dir() -> PathBuf {
    let base = std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| expand_home("~/.local/state"));
    base.join("devtray")
}

pub fn default_config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| expand_home("~/.config"));
    base.join("devtray/config.toml")
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("读不到 {}：{e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("TOML 解析失败：{e}"))
    }

    /// 从文本解析。表单保存前用它做「先校验、再落盘」，避免把配置写坏。
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| format!("TOML 解析失败：{e}"))
    }

    /// 展开成运行期目标；任何一处不合法都整体报错（避免半套配置生效）
    pub fn targets(&self) -> Result<Vec<Target>, String> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut log_paths = HashSet::new();
        for p in &self.projects {
            if p.name.trim().is_empty() {
                return Err("存在 name 为空的项目".into());
            }
            if p.services.is_empty() {
                return Err(format!("项目 {} 没有任何 service", p.name));
            }
            for s in &p.services {
                if s.name.trim().is_empty() {
                    return Err(format!("项目 {} 下存在 name 为空的 service", p.name));
                }
                if s.cmd.trim().is_empty() {
                    return Err(format!("{}/{} 的 cmd 为空", p.name, s.name));
                }
                if s.port == Some(0) {
                    return Err(format!("{}/{} 的 port 不能是 0", p.name, s.name));
                }
                let cwd = expand_home(&s.cwd);
                if !cwd.is_dir() {
                    return Err(format!(
                        "{}/{} 的 cwd 不存在：{}",
                        p.name,
                        s.name,
                        cwd.display()
                    ));
                }
                let session = format!(
                    "{SESSION_PREFIX}{}",
                    sanitize(&format!("{}-{}", p.name, s.name))
                );
                if !seen.insert(session.clone()) {
                    return Err(format!(
                        "tmux 会话名冲突：{}（项目/服务名去掉特殊字符后重复）",
                        session
                    ));
                }
                // 空字符串要当没配：否则空正则会匹配一切，服务瞬间变「就绪」
                let ready_re = match s.ready.as_deref().map(str::trim) {
                    Some(r) if !r.is_empty() => Some(
                        Regex::new(r)
                            .map_err(|e| format!("{}/{} 的 ready 正则非法：{e}", p.name, s.name))?,
                    ),
                    _ => None,
                };
                let log_path = match &s.log_file {
                    Some(f) if !f.trim().is_empty() => expand_home(f),
                    _ => state_dir().join("logs").join(format!(
                        "{}-{}.log",
                        sanitize(&p.name),
                        sanitize(&s.name)
                    )),
                };
                // A pipe child runs inside the service cwd; always pass an absolute log path.
                let log_path = if log_path.is_absolute() {
                    log_path
                } else {
                    std::env::current_dir()
                        .map_err(|e| e.to_string())?
                        .join(&cwd)
                        .join(log_path)
                };
                let log_key = log_path.canonicalize().unwrap_or_else(|_| log_path.clone());
                if !log_paths.insert(log_key) {
                    return Err(format!(
                        "日志路径冲突：{}（每个服务需要独立日志文件）",
                        log_path.display()
                    ));
                }
                out.push(Target {
                    project: p.name.clone(),
                    service: s.clone(),
                    session,
                    log_path,
                    ready_re,
                });
            }
        }
        Ok(out)
    }
}

/// tmux 会话名不能含 `.` `:` 与空白（target 分隔符），统一降级为 `-`
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_session_names() {
        assert_eq!(sanitize("clothes.mes:admin"), "clothes-mes-admin");
        assert_eq!(sanitize("a b"), "a-b");
    }

    #[test]
    fn quote_shell() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn reject_bad_config() {
        let cfg: Config = toml::from_str(
            r#"
            [[project]]
            name = "p"
            [[project.service]]
            name = "s"
            cwd = "/definitely/not/here"
            cmd = "echo hi"
        "#,
        )
        .unwrap();
        assert!(cfg.targets().unwrap_err().contains("cwd 不存在"));
    }

    #[test]
    fn relative_logs_resolve_inside_service_and_shared_logs_are_rejected() {
        let cfg = Config::parse(
            r#"[[project]]
name = "p"
[[project.service]]
name = "one"
cwd = "/tmp"
cmd = "true"
log_file = "devtray-relative.log"
"#,
        )
        .unwrap();
        assert_eq!(
            cfg.targets().unwrap()[0].log_path,
            PathBuf::from("/tmp/devtray-relative.log")
        );
        let mut cfg = cfg;
        let mut other = cfg.projects[0].services[0].clone();
        other.name = "two".into();
        cfg.projects[0].services.push(other);
        assert!(cfg.targets().unwrap_err().contains("日志路径冲突"));
    }

    #[test]
    fn build_full_cmd_with_env() {
        let s: Service = toml::from_str(
            r#"
            name = "s"
            cwd = "/tmp"
            cmd = "bun run dev"
            [env]
            NODE_OPTIONS = "--max-old-space-size=4096"
        "#,
        )
        .unwrap();
        assert_eq!(
            s.full_cmd(),
            "export NODE_OPTIONS='--max-old-space-size=4096'; bun run dev"
        );
    }
}
