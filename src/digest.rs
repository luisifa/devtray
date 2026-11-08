//! 日志分诊：把几百行日志压成「错误类型 + 位置 + 次数」。
//!
//! 目的是给 agent 一个低 token 的入口：先看分诊结果，只有定位不了才去读原文。
//! 全部是确定性规则（正则 + 去重），不调模型；CLI `devtray logs --digest` 与
//! MCP `devtray_logs { digest: true }` 共用同一条路径。

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

/// 最多展示多少类问题；剩下的只报数字。
const MAX_FINDINGS: usize = 8;
/// 单条摘要的最大字符数，防止把一整行 stack trace 原样塞回去。
const SUMMARY_CHARS: usize = 240;
const SAMPLE_CHARS: usize = 160;

static RUST_ERROR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^error(?:\[(E\d{4})\])?:\s*(.+)$").unwrap());
static RUST_WARNING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^warning(?:\[E?\w+\])?:\s*(.+)$").unwrap());
static ARROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*-->\s*(\S+?):(\d+)(?::(\d+))?").unwrap());
static PANIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^thread '(.+?)' panicked at (.+?):(\d+):(\d+):?\s*$").unwrap());
static PANIC_BARE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^panicked at (.+?):(\d+):(\d+):?\s*$").unwrap());
static PY_FRAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s+File "(.+?)", line (\d+)"#).unwrap());
static PY_FINAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^([A-Za-z_][A-Za-z0-9_.]*(?:Error|Exception|Interrupt|Exit|Warning))(?::\s*(.*))?$",
    )
    .unwrap()
});
static JS_ERROR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:Uncaught )?([A-Za-z]*Error)\b:?\s*(.*)$").unwrap());
static JS_FRAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s+at .*?\(?(\S+?):(\d+):(\d+)\)?\s*$").unwrap());
static PORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(EADDRINUSE|address already in use|端口.{0,8}(?:被)?占用)").unwrap()
});
static ADDR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d{1,3}(?:\.\d{1,3}){3}|localhost|0\.0\.0\.0|::1|\[::\]|::):(\d{2,5})").unwrap()
});
static ELIFECYCLE: LazyLock<Regex> =
    // 允许行首有控制字符（`^C [ELIFECYCLE] ...`）和空白，但不匹配 `npm ERR! code ELIFECYCLE`
    LazyLock::new(|| {
        Regex::new(r"(?i)^(?:\^C\s*)?[\x00-\x20]*\[?ELIFECYCLE\]?:?\s*(.*)$").unwrap()
    });
/// 关机/重启/Ctrl-C 这类「不是代码错」的收尾，单独作为上下文保留
static LIFECYCLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:the system will (?:reboot|power off|shut ?down)(?: now)?|keyboard interrupt|\^C\b|received signal|shutting down)")
        .unwrap()
});
static PATH_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bpath=([^\s}\],;:]+)").unwrap());
static PROXY_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:proxy error|error):\s*(/[A-Za-z0-9_./-]+)").unwrap());

/// 没有级别词、但明显是失败的写法：`connection error: ...`、`build failed: ...`
static SOFT_ERROR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[a-z_][\w-]{2,15}\s+(?:error|failed|failure|panic)\s*[:：]").unwrap()
});
static PACKAGE_MANAGER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:npm|pnpm|yarn)\s+(?:ERR!|error):?\s*(.*)$").unwrap());
static PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:\[[^\]]*\d[^\]]*\]\s*|\d{4}-\d{2}-\d{2}[T ][\d:.+Z-]+\s*|\d{2}:\d{2}:\d{2}(?:\.\d+)?\s*)?\[?\s*(?:trace|debug|info|warn(?:ing)?|error|fatal|panic(?:ked)?)\b:?\s*",
    )
    .unwrap()
});
static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").unwrap());
/// tracing/axum 的 span 前缀：`request{request_id=..., path=...}:`
static SPAN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z_][\w.:]*\{[^}]*\}:\s*").unwrap());
static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z0-9_-]{8,}").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Finding {
    /// compile_error / panic / port / traceback / runtime_error / warning / error
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub summary: String,
    pub count: usize,
    /// 同类问题的原文样例（最多两条），便于判断是不是同一个原因
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<String>,
    #[serde(skip)]
    key: String,
    #[serde(skip)]
    first: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub scanned: usize,
    pub issues: usize,
    pub findings: Vec<Finding>,
    pub last_line: Option<String>,
    /// 日志里的关机/重启/中断标记；能解释「服务为什么没了」
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    /// 超出展示上限、只计数的问题类数
    pub hidden_findings: usize,
}

impl Finding {
    fn new(kind: &'static str, summary: impl Into<String>, at: usize) -> Self {
        let summary = clip(&summary.into(), SUMMARY_CHARS);
        let key = normalize(&summary);
        Self {
            kind,
            code: None,
            location: None,
            summary,
            count: 1,
            samples: Vec::new(),
            key,
            first: at,
        }
    }

    fn with_code(mut self, code: Option<String>) -> Self {
        self.code = code;
        self
    }

    fn at(mut self, location: Option<String>) -> Self {
        if let Some(location) = location {
            self.key.push('@');
            self.key.push_str(&normalize(&location));
            self.location = Some(location);
        }
        self
    }

    fn label(&self) -> &'static str {
        match self.kind {
            "compile_error" => "编译错误",
            "panic" => "panic",
            "port" => "端口占用",
            "traceback" => "Python 异常",
            "runtime_error" => "运行时错误",
            "script_failed" => "脚本失败",
            "warning" => "警告",
            _ => "错误",
        }
    }
}

/// 分诊入口：`lines` 按时间顺序，返回压缩后的问题列表。
pub fn analyze(lines: &[String]) -> Report {
    let mut report = Report {
        scanned: lines.len(),
        ..Default::default()
    };
    let mut i = 0;
    while i < lines.len() {
        // 调用方通常已经剥过 ANSI；这里再兜一层，保证本模块单独使用时也正确。
        let stripped = crate::cli::strip_ansi(&lines[i]);
        let line = stripped.trim_end_matches('\r').trim_end();
        if !line.is_empty() {
            report.last_line = Some(clip(line, SUMMARY_CHARS));
        }
        if LIFECYCLE.is_match(line) {
            report.lifecycle = Some(clip(line, 120));
        }
        match classify(lines, i) {
            Some((finding, used)) => {
                report.issues += 1;
                push(&mut report.findings, finding);
                i += used.max(1);
            }
            None => i += 1,
        }
    }
    report
        .findings
        .sort_by(|a, b| b.count.cmp(&a.count).then(a.first.cmp(&b.first)));
    if report.findings.len() > MAX_FINDINGS {
        report.hidden_findings = report.findings.len() - MAX_FINDINGS;
        report.findings.truncate(MAX_FINDINGS);
    }
    report
}

/// 渲染成给人/agent 看的紧凑文本。
pub fn render(report: &Report, subject: &str, state: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str(subject);
    if let Some(state) = state {
        out.push_str(" · ");
        out.push_str(state);
    }
    out.push('\n');
    if report.findings.is_empty() {
        out.push_str(&format!("扫描 {} 行，没有错误级日志", report.scanned));
        if let Some(last) = &report.last_line {
            out.push_str(&format!("；最后一行：{last}"));
        }
        return out;
    }
    out.push_str(&format!(
        "扫描 {} 行，{} 处问题，{} 类",
        report.scanned,
        report.issues,
        report.findings.len() + report.hidden_findings
    ));
    if let Some(lifecycle) = &report.lifecycle {
        out.push_str(&format!(
            "\n上下文：{lifecycle}（日志里的关机/重启/中断标记）"
        ));
    }
    for (index, finding) in report.findings.iter().enumerate() {
        let code = finding
            .code
            .as_ref()
            .map(|c| format!(" {c}"))
            .unwrap_or_default();
        let at = finding
            .location
            .as_ref()
            .map(|l| format!(" {l}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "\n{}. [{}{code}{at}] ×{}",
            index + 1,
            finding.label(),
            finding.count
        ));
        out.push_str(&format!("\n   {}", finding.summary));
        for sample in &finding.samples {
            out.push_str(&format!("\n   | {sample}"));
        }
    }
    if report.hidden_findings > 0 {
        out.push_str(&format!("\n… 另有 {} 类问题未展示", report.hidden_findings));
    }
    out
}

/// 单行分类。返回问题本身和它「吃掉」的行数（跳过 `-->`、traceback 尾部等）。
fn classify(lines: &[String], i: usize) -> Option<(Finding, usize)> {
    let stripped = crate::cli::strip_ansi(&lines[i]);
    let line = stripped.trim_end_matches('\r').trim_end();

    if let Some(caps) = PANIC.captures(line) {
        let location = format!("{}:{}:{}", &caps[2], &caps[3], &caps[4]);
        let message = lines.get(i + 1).map(String::as_str).unwrap_or("");
        let summary = match (message.trim(), &caps[1]) {
            ("", thread) => thread.to_string(),
            (text, "main") => text.to_string(),
            (text, thread) => format!("{text}（{thread}）"),
        };
        return Some((Finding::new("panic", summary, i).at(Some(location)), 1));
    }
    if let Some(caps) = PANIC_BARE.captures(line) {
        let location = format!("{}:{}:{}", &caps[1], &caps[2], &caps[3]);
        let message = lines.get(i + 1).map(String::as_str).unwrap_or("");
        return Some((
            Finding::new("panic", message.trim().to_string(), i).at(Some(location)),
            1,
        ));
    }

    if let Some(caps) = RUST_ERROR.captures(line) {
        let code = caps.get(1).map(|m| m.as_str().to_string());
        let message = caps[2].trim().to_string();
        // 只有 `error[...]:` 后面跟了 `-->` 才是编译错误；`error: Script not found` 是运行时报错。
        if let Some((offset, location)) = arrow_after(lines, i) {
            return Some((
                Finding::new("compile_error", message, i)
                    .with_code(code)
                    .at(Some(location)),
                offset + 1,
            ));
        }
        return Some((Finding::new("error", message, i).with_code(code), 1));
    }

    if ELIFECYCLE.is_match(line) {
        // 保留整行：`^C [ELIFECYCLE] Command failed.` 里的 `^C` 是「用户手动停的」的关键线索
        return Some((Finding::new("script_failed", line.to_string(), i), 1));
    }

    if let Some(caps) = PACKAGE_MANAGER.captures(line) {
        return Some((Finding::new("error", caps[1].trim().to_string(), i), 1));
    }

    if let Some(caps) = RUST_WARNING.captures(line) {
        return Some((Finding::new("warning", caps[1].trim().to_string(), i), 1));
    }

    if line.starts_with("Traceback (most recent call last)") {
        return python_traceback(lines, i);
    }

    if PORT.is_match(line) {
        let location = ADDR.captures(line).map(|c| format!("{}:{}", &c[1], &c[2]));
        return Some((Finding::new("port", strip_prefix(line), i).at(location), 1));
    }

    if let Some(caps) = JS_ERROR.captures(line) {
        // 排除 `Error: ...` 之外的普通句子；必须有错误名或后续栈帧才算。
        let name = &caps[1];
        let summary = if caps[2].trim().is_empty() {
            name.to_string()
        } else {
            format!("{name}: {}", caps[2].trim())
        };
        let location = frame_after(lines, i);
        return Some((Finding::new("runtime_error", summary, i).at(location), 1));
    }

    if SOFT_ERROR.is_match(line) {
        return Some((Finding::new("error", line.to_string(), i), 1));
    }

    if crate::logs::level(line) == Some("error") {
        return Some((
            Finding::new("error", strip_prefix(line), i).at(endpoint_of(line)),
            1,
        ));
    }

    None
}

/// 编译错误块里的 `--> 文件:行:列`，只在紧邻几行内找。
fn arrow_after(lines: &[String], i: usize) -> Option<(usize, String)> {
    for (offset, line) in lines.iter().enumerate().skip(i + 1).take(6) {
        if let Some(caps) = ARROW.captures(line) {
            let location = match caps.get(3) {
                Some(col) => format!("{}:{}:{}", &caps[1], &caps[2], col.as_str()),
                None => format!("{}:{}", &caps[1], &caps[2]),
            };
            return Some((offset - i, location));
        }
    }
    None
}

/// JS 栈顶帧（跳过 node_modules）。
fn frame_after(lines: &[String], i: usize) -> Option<String> {
    for line in lines.iter().skip(i + 1).take(8) {
        let Some(caps) = JS_FRAME.captures(line) else {
            continue;
        };
        if caps[1].contains("node_modules")
            || caps[1].starts_with("node:")
            || caps[1].starts_with("internal/")
        {
            continue;
        }
        return Some(format!("{}:{}:{}", &caps[1], &caps[2], &caps[3]));
    }
    None
}

/// Python traceback：取最后一帧的位置和末尾异常行。
fn python_traceback(lines: &[String], i: usize) -> Option<(Finding, usize)> {
    let mut location = None;
    let mut summary = "Traceback".to_string();
    let mut used = 1;
    for (offset, line) in lines.iter().enumerate().skip(i + 1).take(60) {
        used = offset - i + 1;
        if let Some(caps) = PY_FRAME.captures(line) {
            location = Some(format!("{}:{}", &caps[1], &caps[2]));
            continue;
        }
        if !line.starts_with([' ', '\t'])
            && let Some(caps) = PY_FINAL.captures(line.trim_end())
        {
            summary = match caps.get(2) {
                Some(message) if !message.as_str().trim().is_empty() => {
                    format!("{}: {}", &caps[1], message.as_str().trim())
                }
                _ => caps[1].to_string(),
            };
            break;
        }
    }
    Some((Finding::new("traceback", summary, i).at(location), used))
}

fn push(findings: &mut Vec<Finding>, finding: Finding) {
    if let Some(existing) = findings.iter_mut().find(|e| e.key == finding.key) {
        existing.count += 1;
        if existing.samples.len() < 2 && finding.summary != existing.summary {
            let sample = clip(&finding.summary, SAMPLE_CHARS);
            if !existing.samples.contains(&sample) {
                existing.samples.push(sample);
            }
        }
        return;
    }
    findings.push(finding);
}

/// 从错误行里抠出请求端点（`path=/api/login`、vite 代理路径），给定位用。
fn endpoint_of(line: &str) -> Option<String> {
    let raw = PATH_FIELD
        .captures(line)
        .map(|c| c[1].to_string())
        .or_else(|| PROXY_PATH.captures(line).map(|c| c[1].to_string()))?;
    let trimmed = raw.trim_end_matches(['}', ',', ';', '"', '\'']);
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn strip_prefix(line: &str) -> String {
    let without_level = PREFIX.replace(line, "");
    // 去掉 tracing 的 span 前缀，留下「模块: 消息」，否则一行里半截都是 request 元数据。
    SPAN.replace(&without_level, "").trim().to_string()
}

/// 去重用的归一化：去掉数字、折叠空白、转小写。
fn normalize(text: &str) -> String {
    // ULID / UUID / 十六进制哈希这类「字母+数字」的长 token 直接抹掉，
    // 否则同一条错误因为 request_id 不同会被当成两类。
    let no_tokens = TOKEN.replace_all(text, |caps: &regex::Captures| {
        let token = &caps[0];
        if token.chars().any(|c| c.is_ascii_digit()) {
            "#".to_string()
        } else {
            token.to_string()
        }
    });
    let no_digits = DIGITS.replace_all(&no_tokens, "#");
    SPACES.replace_all(&no_digits, " ").trim().to_lowercase()
}

fn clip(text: &str, limit: usize) -> String {
    let text = SPACES.replace_all(text.trim(), " ");
    if text.chars().count() <= limit {
        return text.into_owned();
    }
    let cut: String = text.chars().take(limit).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn rust_compile_errors_keep_code_and_location() {
        let report = analyze(&lines(
            "   Compiling app v0.1.0\n\
             error[E0308]: mismatched types\n\
              --> src/git.rs:412:5\n\
               |\n\
             412|     foo(bar);\n\
               |         ^^^ expected `&str`, found `String`\n\
             \n\
             error: could not compile `app` (bin \"app\") due to 1 previous error\n",
        ));
        assert_eq!(report.findings.len(), 2, "{:?}", report.findings);
        let first = &report.findings[0];
        assert_eq!(first.kind, "compile_error");
        assert_eq!(first.code.as_deref(), Some("E0308"));
        assert_eq!(first.location.as_deref(), Some("src/git.rs:412:5"));
        assert!(first.summary.contains("mismatched types"));
        // `error: could not compile` 没有 `-->`，按普通错误处理
        assert_eq!(report.findings[1].kind, "error");
    }

    #[test]
    fn panic_uses_next_line_as_message() {
        let report = analyze(&lines(
            "thread 'main' panicked at src/main.rs:12:5:\n\
             called `Option::unwrap()` on a `None` value\n\
             note: run with `RUST_BACKTRACE=1`\n",
        ));
        assert_eq!(report.findings.len(), 1);
        let f = &report.findings[0];
        assert_eq!(f.kind, "panic");
        assert_eq!(f.location.as_deref(), Some("src/main.rs:12:5"));
        assert!(f.summary.contains("Option::unwrap"));
    }

    #[test]
    fn python_traceback_takes_last_frame_and_error() {
        let report = analyze(&lines(
            "Traceback (most recent call last):\n\
             \x20 File \"app.py\", line 10, in <module>\n\
             \x20   main()\n\
             \x20 File \"app.py\", line 7, in main\n\
             \x20   raise ValueError('boom')\n\
             ValueError: boom\n",
        ));
        let f = &report.findings[0];
        assert_eq!(f.kind, "traceback");
        assert_eq!(f.location.as_deref(), Some("app.py:7"));
        assert_eq!(f.summary, "ValueError: boom");
    }

    #[test]
    fn repeated_runtime_errors_deduplicate_across_timestamps() {
        let report = analyze(&lines(
            "2026-11-07T10:00:01Z ERROR request failed for user 42\n\
             2026-11-07T10:00:02Z ERROR request failed for user 77\n\
             2026-11-07T10:00:03Z ERROR request failed for user 91\n",
        ));
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        assert_eq!(report.findings[0].count, 3);
        assert_eq!(report.issues, 3);
    }

    #[test]
    fn port_conflict_is_named_with_address() {
        let report = analyze(&lines(
            "thread 'main' panicked at src/main.rs:3:9:\n\
             Error: Os { code: 98, kind: AddrInUse, message: \"Address already in use\" }\n\
             ...\n",
        ));
        let port = report
            .findings
            .iter()
            .find(|f| f.kind == "port")
            .expect("port finding");
        assert!(
            port.summary
                .to_lowercase()
                .contains("address already in use")
        );
    }

    #[test]
    fn js_error_carries_first_non_dependency_frame() {
        let report = analyze(&lines(
            "TypeError: Cannot read properties of undefined (reading 'id')\n\
             \x20   at render (/app/src/view.ts:88:12)\n\
             \x20   at node_modules/react/index.js:1:1\n",
        ));
        let f = &report.findings[0];
        assert_eq!(f.kind, "runtime_error");
        assert_eq!(f.location.as_deref(), Some("/app/src/view.ts:88:12"));
    }

    #[test]
    fn ulid_differences_do_not_split_identical_errors() {
        let report = analyze(&lines(
            "2026-11-07T04:51:57Z ERROR request{request_id=01M6T5JPWKN7R0ASPPB2DD8MB2 path=/api/login}: \
             server error error=DatabaseError(\"db error: value too long\") request_id=\"-\"\n\
             2026-11-07T04:52:55Z ERROR request{request_id=01M6T5MF4CXZBW1G8CZE8QHDY4 path=/api/login}: \
             server error error=DatabaseError(\"db error: value too long\") request_id=\"-\"\n",
        ));
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        assert_eq!(report.findings[0].count, 2);
    }

    #[test]
    fn tracing_span_prefix_is_dropped_from_the_summary() {
        let report = analyze(&lines(
            "2026-11-07T04:51:57Z ERROR request{request_id=01M6T5JP path=/api/login}: \
             hb_ecom::error: server error error=DatabaseError(\"value too long\")\n",
        ));
        let f = &report.findings[0];
        assert_eq!(
            f.summary,
            "hb_ecom::error: server error error=DatabaseError(\"value too long\")"
        );
    }

    #[test]
    fn elifeclycle_matches_mid_line() {
        let report = analyze(&lines("^C [ELIFECYCLE] Command failed.\n"));
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        assert_eq!(report.findings[0].kind, "script_failed");
        assert_eq!(
            report.findings[0].summary,
            "^C [ELIFECYCLE] Command failed."
        );
    }

    #[test]
    fn lifecycle_markers_become_context() {
        let report = analyze(&lines(
            "INFO up\nBroadcast message from u@h (Fri):\nThe system will reboot now!\n\
             [ELIFECYCLE] Command failed.\n",
        ));
        assert_eq!(
            report.lifecycle.as_deref(),
            Some("The system will reboot now!")
        );
        let text = render(&report, "demo/web", Some("○ 未启动"));
        assert!(
            text.contains("上下文：The system will reboot now!"),
            "{text}"
        );

        let report = analyze(&lines("^C [ELIFECYCLE] Command failed.\n"));
        assert_eq!(
            report.lifecycle.as_deref(),
            Some("^C [ELIFECYCLE] Command failed.")
        );
    }

    #[test]
    fn request_endpoint_is_lifted_into_location() {
        let report = analyze(&lines(
            "2026-11-07T04:51:57Z ERROR request{request_id=abc path=/api/auth/login}: \
             hb_ecom::error: server error\n",
        ));
        assert_eq!(
            report.findings[0].location.as_deref(),
            Some("/api/auth/login")
        );
    }

    #[test]
    fn bare_word_error_lines_are_surfaced() {
        let report = analyze(&lines(
            "connection error: db error\nconnection error: db error\n",
        ));
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].kind, "error");
        assert_eq!(report.findings[0].count, 2);
        // 以 error 开头的行本来就按错误级处理（与日志级别分类保持一致）
        assert_eq!(analyze(&lines("error rate: 0.00%\n")).findings.len(), 1);
    }

    #[test]
    fn node_internal_frames_are_not_locations() {
        let report = analyze(&lines(
            "Error: connect ECONNREFUSED 127.0.0.1:3060\n\
             \x20   at TCPConnectWrap.afterConnect [as oncomplete] (node:net:1706:16)\n",
        ));
        let f = &report.findings[0];
        assert_eq!(f.location, None, "{f:?}");
    }

    #[test]
    fn package_manager_failures_are_recognized() {
        let report = analyze(&lines(
            "$ vite preview --port 8097\n\
             \x1b[41m[ELIFECYCLE]\x1b[49m Command failed.\n",
        ));
        let f = &report.findings[0];
        assert_eq!(f.kind, "script_failed");
        assert_eq!(f.summary, "[ELIFECYCLE] Command failed.");

        let report = analyze(&lines(
            "npm ERR! code ELIFECYCLE\nnpm ERR! exit code 1\nnpm ERR! code ELIFECYCLE\n",
        ));
        assert_eq!(report.findings.len(), 2, "{:?}", report.findings);
        assert_eq!(report.findings[0].kind, "error");
        assert_eq!(report.findings[0].summary, "code ELIFECYCLE");
        assert_eq!(report.findings[0].count, 2);
    }

    #[test]
    fn clean_log_reports_no_errors_and_last_line() {
        let report = analyze(&lines("INFO boot\nINFO listen on 127.0.0.1:3060\n"));
        assert!(report.findings.is_empty());
        let text = render(&report, "demo/backend", Some("ready"));
        assert!(text.contains("没有错误级日志"));
        assert!(text.contains("最后一行：INFO listen on 127.0.0.1:3060"));
    }

    #[test]
    fn findings_are_capped_and_counted() {
        // 用互不相同的词，避免被数字归一化合并成同一条
        let mut text = String::new();
        for word in [
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
            "juliet", "kilo", "lima",
        ] {
            text.push_str(&format!("error: failure {word}\n"));
        }
        let report = analyze(&lines(&text));
        assert_eq!(report.findings.len(), MAX_FINDINGS);
        assert_eq!(report.hidden_findings, 4);
        let rendered = render(&report, "demo/x", None);
        assert!(rendered.contains("另有 4 类问题未展示"));
    }
}
