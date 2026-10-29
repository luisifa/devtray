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
    /// 额外允许清理的目录名（默认不含 dist/build/vendor）；仍需通过证据链
    pub clean_extra_dirs: Vec<String>,
    /// 资源页扫描缓存 TTL（秒）；0 = 不缓存
    pub disk_cache_ttl_s: u64,
    /// MCP 写操作策略：ask（yad 审批）/ deny / allow
    pub mcp_writes: String,
    /// Skills 上游更新检查缓存 TTL（秒）；默认 6 小时
    pub skill_check_ttl_s: u64,
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
            clean_extra_dirs: Vec::new(),
            disk_cache_ttl_s: 600,
            mcp_writes: "ask".into(),
            skill_check_ttl_s: 21600,
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
    /// 项目根目录。留空 = 用该 project 下所有 service.cwd 的公共祖先推断。
    /// 扫描磁盘占用、打开目录都以它为准；推断结果不可靠时请显式写。
    pub root: Option<String>,
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

/// 项目根目录的校验结果。`cleaning_allowed = false` 时扫描仍可用，但禁止清理。
#[derive(Debug, Clone)]
pub struct ProjectRoot {
    pub project: String,
    pub root: PathBuf,
    /// true = 由 service.cwd 推断，false = 配置显式指定
    pub inferred: bool,
    pub cleaning_allowed: bool,
    /// 禁止清理的原因；`cleaning_allowed = true` 时为 None
    pub reason: Option<String>,
    /// 非致命提示（例如推断时回退到第一个 cwd）
    pub notes: Vec<String>,
}

pub fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_default()
}

/// 根目录过宽时禁止清理。`/`、家目录、`~/Workspace` 是实测最容易因推断过宽出事的三类。
pub fn cleaning_refusal(root: &Path, home: &Path) -> Option<String> {
    if root == Path::new("/") {
        return Some("项目根是 /".into());
    }
    if !home.as_os_str().is_empty() && root == home {
        return Some("项目根是家目录（推断过宽）".into());
    }
    if !home.as_os_str().is_empty() && root == home.join("Workspace") {
        return Some("项目根是 ~/Workspace（推断过宽）".into());
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        let dirs = entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .count();
        if dirs > 50 {
            return Some(format!("项目根下直接子目录过多（{dirs} > 50）"));
        }
    }
    None
}

/// 所有路径的公共祖先；全在 `/` 下时返回 `/`（调用方负责判定"太宽"）。
fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    let first = paths.first()?;
    let mut acc = PathBuf::new();
    for comp in first.components() {
        let mut candidate = acc.clone();
        candidate.push(comp);
        if paths.iter().skip(1).all(|p| p.starts_with(&candidate)) {
            acc = candidate;
        } else {
            break;
        }
    }
    (!acc.as_os_str().is_empty()).then_some(acc)
}

/// 规范成绝对路径；目录存在时解析符号链接，免得同一个目录出现两种写法。
pub(crate) fn absolute(path: PathBuf) -> PathBuf {
    if let Ok(canon) = std::fs::canonicalize(&path) {
        return canon;
    }
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|d| d.join(&path))
            .unwrap_or(path)
    }
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

    /// 解析项目根目录：显式 `root` 优先，否则用 service.cwd 的公共祖先。
    /// 与 `targets()` 分开：托盘/启停只需要目标，资源页才需要根目录。
    pub fn project_roots(&self) -> Result<Vec<ProjectRoot>, String> {
        let home = home_dir();
        let mut out = Vec::new();
        for p in &self.projects {
            let mut notes = Vec::new();
            let (root, inferred) = match p.root.as_deref().map(str::trim) {
                Some(r) if !r.is_empty() => (absolute(expand_home(r)), false),
                _ => {
                    if p.services.is_empty() {
                        return Err(format!(
                            "项目 {} 既没有 root，也没有 service 可供推断",
                            p.name
                        ));
                    }
                    let cwds: Vec<PathBuf> = p
                        .services
                        .iter()
                        .map(|s| absolute(expand_home(&s.cwd)))
                        .collect();
                    match common_ancestor(&cwds) {
                        Some(c) if c != Path::new("/") => (c, true),
                        _ => {
                            notes.push("service.cwd 没有公共祖先，已回退到第一个 cwd".into());
                            (cwds[0].clone(), true)
                        }
                    }
                }
            };
            if !root.is_dir() {
                return Err(format!(
                    "项目 {} 的 root 不存在或不是目录：{}",
                    p.name,
                    root.display()
                ));
            }
            let refusal = cleaning_refusal(&root, &home);
            out.push(ProjectRoot {
                project: p.name.clone(),
                root,
                inferred,
                cleaning_allowed: refusal.is_none(),
                reason: refusal,
                notes,
            });
        }
        Ok(out)
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

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "devtray-config-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn dir(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(&p).unwrap();
            p
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn project_root_prefers_explicit_and_infers_common_ancestor() {
        let t = Tmp::new("root");
        let repo = t.dir("repo");
        let backend = t.dir("repo/backend");
        let frontend = t.dir("repo/frontend");
        let canonical = |p: &Path| std::fs::canonicalize(p).unwrap();

        let cfg = Config::parse(&format!(
            r#"[[project]]
name = "p"
[[project.service]]
name = "backend"
cwd = "{}"
cmd = "true"
[[project.service]]
name = "frontend"
cwd = "{}"
cmd = "true"
"#,
            backend.display(),
            frontend.display()
        ))
        .unwrap();
        let roots = cfg.project_roots().unwrap();
        assert_eq!(roots[0].root, canonical(&repo));
        assert!(roots[0].inferred);
        assert!(roots[0].cleaning_allowed);

        let cfg = Config::parse(&format!(
            r#"[[project]]
name = "p"
root = "{}"
[[project.service]]
name = "backend"
cwd = "{}"
cmd = "true"
"#,
            repo.display(),
            backend.display()
        ))
        .unwrap();
        let roots = cfg.project_roots().unwrap();
        assert_eq!(roots[0].root, canonical(&repo));
        assert!(!roots[0].inferred);
    }

    #[test]
    fn cleaning_refusal_covers_broad_roots() {
        let t = Tmp::new("broad");
        let home = t.dir("home");
        let workspace = t.dir("home/Workspace");
        let project = t.dir("home/Workspace/project");
        assert!(cleaning_refusal(Path::new("/"), &home).is_some());
        assert!(cleaning_refusal(&home, &home).is_some());
        assert!(cleaning_refusal(&workspace, &home).is_some());
        assert!(cleaning_refusal(&project, &home).is_none());
    }

    #[test]
    fn missing_explicit_root_is_reported() {
        let cfg = Config::parse(
            r#"[[project]]
name = "p"
root = "/definitely/not/here"
[[project.service]]
name = "s"
cwd = "/tmp"
cmd = "true"
"#,
        )
        .unwrap();
        assert!(cfg.project_roots().unwrap_err().contains("root 不存在"));
    }
}
