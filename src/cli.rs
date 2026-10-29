//! CLI 底座：解析目标、执行动作、输出状态。托盘、MCP、skill 三条路都走这里，
//! 保证「agent 通过 MCP 做的」和「你手敲命令做的」是同一件事。

use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::config::{Config, Target, default_config_path};
use crate::tmux::{self, Prober, State};

/// 一次命令的上下文：配置 + 已解析的目标
pub struct Ctx {
    pub targets: Vec<Target>,
}

impl Ctx {
    pub fn load() -> Result<Self, String> {
        let path = std::env::var("DEVTRAY_CONFIG")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| default_config_path());
        let cfg = Config::load(&path)?;
        Ok(Self {
            targets: cfg.targets()?,
        })
    }

    /// 目标写法：`项目/服务` 全名、唯一的服务名、或 tmux 会话名
    pub fn resolve(&self, spec: &str) -> Result<&Target, String> {
        let spec = spec.trim();
        if let Some(t) = self
            .targets
            .iter()
            .find(|t| format!("{}/{}", t.project, t.service.name) == spec || t.session == spec)
        {
            return Ok(t);
        }
        let hits: Vec<&Target> = self
            .targets
            .iter()
            .filter(|t| t.service.name == spec)
            .collect();
        match hits.len() {
            1 => Ok(hits[0]),
            0 => Err(format!(
                "找不到服务 {spec:?}。可用：{}",
                self.list_names().join("、")
            )),
            _ => Err(format!(
                "{spec:?} 有歧义，请用 项目/服务 全名：{}",
                hits.iter()
                    .map(|t| format!("{}/{}", t.project, t.service.name))
                    .collect::<Vec<_>>()
                    .join("、")
            )),
        }
    }

    pub fn list_names(&self) -> Vec<String> {
        self.targets
            .iter()
            .map(|t| format!("{}/{}", t.project, t.service.name))
            .collect()
    }

    /// 当前状态快照（一次 tmux 调用 + 每个服务各自的探测）
    pub fn statuses(&self) -> Vec<ServiceStatus> {
        let snap = tmux::snapshot();
        let mut prober = Prober::default();
        let acknowledgements = crate::alerts::Acknowledgements::load_at(
            &crate::alerts::store_path(&crate::config_path()),
        );
        self.targets
            .iter()
            .map(|t| {
                let state = prober.state_of(t, &snap);
                let failure_token = snap.get(&t.session).and_then(crate::alerts::failure_token);
                let alert_acknowledged = failure_token
                    .as_deref()
                    .is_some_and(|token| acknowledgements.contains(&t.session, token));
                ServiceStatus {
                    project: t.project.clone(),
                    service: t.service.name.clone(),
                    session: t.session.clone(),
                    state,
                    alert_acknowledged,
                    failure_token,
                    exit_code: match state {
                        State::Exited(code) => code,
                        _ => None,
                    },
                    cwd: t.service.cwd.clone(),
                    cmd: t.service.cmd.clone(),
                    log: t.log_path.display().to_string(),
                    autostart: t.service.autostart,
                    url: preview_url(t),
                }
            })
            .collect()
    }
}

pub struct ServiceStatus {
    pub project: String,
    pub service: String,
    pub session: String,
    pub state: State,
    pub alert_acknowledged: bool,
    pub failure_token: Option<String>,
    pub exit_code: Option<i32>,
    pub cwd: String,
    pub cmd: String,
    pub log: String,
    pub autostart: bool,
    pub url: Option<String>,
}

impl ServiceStatus {
    pub fn name(&self) -> String {
        format!("{}/{}", self.project, self.service)
    }

    pub fn human(&self) -> String {
        let state_text = format!(
            "{}{}",
            self.state.text(),
            if self.alert_acknowledged {
                " · 已读"
            } else {
                ""
            }
        );
        format!("{:<28} {:<18} {}", self.name(), state_text, self.session)
    }

    pub fn json(&self) -> String {
        format!(
            r#"{{"project":{},"service":{},"name":{},"state":{},"state_text":{},"exit_code":{},"session":{},"cwd":{},"cmd":{},"log":{},"autostart":{},"url":{},"alert_acknowledged":{},"failure_token":{}}}"#,
            json_str(&self.project),
            json_str(&self.service),
            json_str(&self.name()),
            json_str(self.state.token()),
            json_str(&self.state.text()),
            self.exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "null".into()),
            json_str(&self.session),
            json_str(&self.cwd),
            json_str(&self.cmd),
            json_str(&self.log),
            self.autostart,
            self.url
                .as_deref()
                .map(json_str)
                .unwrap_or_else(|| "null".into()),
            self.alert_acknowledged,
            self.failure_token
                .as_deref()
                .map(json_str)
                .unwrap_or_else(|| "null".into()),
        )
    }
}

/// 状态汇总（给 MCP/skill 用的一句话结论）
pub fn statuses_json(statuses: &[ServiceStatus]) -> String {
    let items: Vec<String> = statuses.iter().map(|s| s.json()).collect();
    format!(r#"{{"services":[{}]}}"#, items.join(","))
}

pub fn statuses_human(statuses: &[ServiceStatus]) -> String {
    if statuses.is_empty() {
        return "（配置里没有任何服务）".into();
    }
    statuses
        .iter()
        .map(|s| s.human())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn start(t: &Target) -> Result<String, String> {
    let _guard = crate::ops::acquire(std::time::Duration::from_secs(5))?;
    tmux::start(t)?;
    Ok(format!(
        "已启动 {}/{}（tmux 会话 {}）",
        t.project, t.service.name, t.session
    ))
}

pub fn stop(t: &Target) -> Result<String, String> {
    let _guard = crate::ops::acquire(std::time::Duration::from_secs(5))?;
    tmux::stop(t)?;
    Ok(format!("已停止 {}/{}", t.project, t.service.name))
}

pub fn restart(t: &Target) -> Result<String, String> {
    let _guard = crate::ops::acquire(std::time::Duration::from_secs(5))?;
    tmux::restart(t)?;
    Ok(format!("已重启 {}/{}", t.project, t.service.name))
}

// ── 预览地址 ────────────────────────────────────────────────────────────────
// 解析优先级：配置的 url > 日志里认出来的 > 用 port 拼。为什么不一律从日志猜？
// 因为日志里的 URL 未必是"给浏览器用的那个"：vite 会同时打印 Network(198.18.0.1)
// 和 Local(localhost)，后端日志里还会混进别处的 URL（比如代理报错信息）。

static URL_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"https?://[A-Za-z0-9._~:/?#@!$&'()*+,;=%-]+"#).ok());

pub fn preview_url(t: &Target) -> Option<String> {
    if let Some(u) = t.service.url.as_deref().map(str::trim)
        && !u.is_empty()
    {
        return Some(with_scheme(u));
    }
    if let Some(u) = url_from_log(&t.log_path) {
        return Some(u);
    }
    // 配了 port 就直接拼一个（比猜日志靠谱，但不如上面两条精确）
    t.service.port.map(|p| format!("http://127.0.0.1:{p}/"))
}

fn with_scheme(u: &str) -> String {
    if u.starts_with("http://") || u.starts_with("https://") {
        u.to_string()
    } else {
        format!("http://{u}")
    }
}

/// 读日志开头一段。
/// 为什么不是 64KB？实测 `cargo run` 会先吐 150KB 编译进度条/告警，banner 被挤到很后面
/// （simple-oa 的 `listen on` 在 151KB 处），窗口给小了就直接认不出 URL。
/// 1MB 足够覆盖常见编译输出；再深的就显式配 `url` 吧。
const URL_SCAN_LIMIT: u64 = 1024 * 1024;

fn read_head(path: &Path, limit: u64) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; limit as usize];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// 从日志里认 URL：先剥 ANSI（vite 的颜色码会夹在 `http://localhost:\e[1m5173\e[22m/` 中间），
/// 优先 localhost/127.0.0.1，取第一条（启动 banner 一般在前）。
fn url_from_log(path: &Path) -> Option<String> {
    let raw = read_head(path, URL_SCAN_LIMIT)?;
    let text = strip_ansi(&raw);
    let re = URL_RE.as_ref()?;
    let mut fallback: Option<String> = None;
    for m in re.find_iter(&text) {
        let u = m
            .as_str()
            .trim_end_matches(['.', ',', '"', '\'', ')', ']', '}']);
        if u.contains("localhost") || u.contains("127.0.0.1") {
            return Some(u.to_string());
        }
        fallback.get_or_insert_with(|| u.replace("0.0.0.0", "127.0.0.1"));
    }
    fallback
}

/// 菜单标签里显示的 `host:port`（太长就截断）
pub fn url_short(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split('/').next().unwrap_or(rest);
    if host.chars().count() > 26 {
        format!("{}…", host.chars().take(25).collect::<String>())
    } else {
        host.to_string()
    }
}

/// 用系统默认程序打开预览地址
pub fn open_preview(t: &Target) -> Result<String, String> {
    let name = format!("{}/{}", t.project, t.service.name);
    let Some(url) = preview_url(t) else {
        return Err(format!(
            "{name} 没有可用的预览地址：在配置里给它加一行 url = \"http://localhost:<端口>/\"，或者配 port = <端口>"
        ));
    };
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(&url);
    // 不 wait 会留僵尸；托盘常驻，反复打开预览会越攒越多
    crate::proc::spawn_detached(&mut cmd).map_err(|e| format!("调 xdg-open 失败：{e}"))?;
    Ok(format!("已用默认程序打开 {url}"))
}

/// 日志尾部：读落盘文件，所以服务死了/停了照样能看
pub fn logs(t: &Target, lines: usize) -> Result<String, String> {
    let result = crate::logs::query(
        &t.log_path,
        &crate::logs::Query {
            lines,
            ..Default::default()
        },
    )?;
    Ok(result
        .entries
        .into_iter()
        .map(|e| e.text)
        .collect::<Vec<_>>()
        .join("\n"))
}

/// 去掉 CSI 转义序列（ESC [ ... 终止字节）
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if it.peek() == Some(&']') {
            it.next();
            while let Some(c2) = it.next() {
                if c2 == '\u{7}' {
                    break;
                }
                if c2 == '\u{1b}' && it.peek() == Some(&'\\') {
                    it.next();
                    break;
                }
            }
        } else if it.peek() == Some(&'[') {
            it.next();
            for c2 in it.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c2) {
                    break;
                }
            }
        }
    }
    out
}

/// 就绪判定里「启动中」可能是刚起还没好，这里给 agent 一句可执行的建议
pub fn advice(statuses: &[ServiceStatus]) -> Option<String> {
    let starting: Vec<String> = statuses
        .iter()
        .filter(|s| s.state == State::Starting)
        .map(|s| s.name())
        .collect();
    let failed: Vec<String> = statuses
        .iter()
        .filter(|s| s.state.is_failure() && !s.alert_acknowledged)
        .map(|s| s.name())
        .collect();
    let mut parts = Vec::new();
    if !starting.is_empty() {
        parts.push(format!(
            "{} 还在启动中（首次编译/依赖预构建可能要几十秒到几分钟），稍后重新查状态",
            starting.join("、")
        ));
    }
    if !failed.is_empty() {
        parts.push(format!(
            "{} 已失败，用 logs 看错误（编译错误/端口占用都在里面）",
            failed.join("、")
        ));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("；"))
    }
}

/// 只够用的 JSON 字符串转义（避免为 CLI 引 serde_json 之外的东西）
pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_is_stripped() {
        assert_eq!(strip_ansi("\u{1b}[32mINFO\u{1b}[0m ok"), "INFO ok");
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn json_escapes() {
        assert_eq!(json_str("a\"b\\c\nd"), r#""a\"b\\c\nd""#);
    }

    #[test]
    fn url_short_hides_scheme_and_path() {
        assert_eq!(url_short("http://localhost:5173/"), "localhost:5173");
        assert_eq!(url_short("https://a.example.com/x/y"), "a.example.com");
    }

    #[test]
    fn url_from_log_prefers_localhost_and_survives_ansi() {
        let dir = std::env::temp_dir().join("devtray-test-url");
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("f.log");
        // 模拟 vite：ANSI 码夹在端口中间，且 Network 在前、Local 在后
        std::fs::write(
            &log,
            "  ➜  Network: http://198.18.0.1:\u{1b}[1m5173\u{1b}[22m/\n  ➜  Local:   http://localhost:\u{1b}[1m5173\u{1b}[22m/\n",
        )
        .unwrap();
        assert_eq!(url_from_log(&log).unwrap(), "http://localhost:5173/");
        std::fs::remove_file(&log).ok();
    }

    #[test]
    fn url_from_log_normalizes_wildcard_bind() {
        let dir = std::env::temp_dir().join("devtray-test-url2");
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("b.log");
        std::fs::write(&log, "🔄 listen on http://0.0.0.0:3060\n").unwrap();
        assert_eq!(url_from_log(&log).unwrap(), "http://127.0.0.1:3060");
        std::fs::remove_file(&log).ok();
    }

    #[test]
    fn with_scheme_adds_missing_one() {
        assert_eq!(with_scheme("localhost:5173"), "http://localhost:5173");
        assert_eq!(with_scheme("https://x/y"), "https://x/y");
    }

    #[test]
    fn resolve_by_short_name_and_reject_ambiguity() {
        let cfg: Config = toml::from_str(
            r#"
            [[project]]
            name = "p1"
            [[project.service]]
            name = "backend"
            cwd = "/tmp"
            cmd = "sleep 1"
            [[project]]
            name = "p2"
            [[project.service]]
            name = "backend"
            cwd = "/tmp"
            cmd = "sleep 1"
        "#,
        )
        .unwrap();
        let ctx = Ctx {
            targets: cfg.targets().unwrap(),
        };
        assert!(ctx.resolve("p1/backend").is_ok());
        assert!(ctx.resolve("backend").unwrap_err().contains("有歧义"));
        assert!(ctx.resolve("nope").unwrap_err().contains("找不到服务"));
        assert!(ctx.resolve("devtray-p1-backend").is_ok());
    }
}
