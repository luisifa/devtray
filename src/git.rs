//! Git 工作区：按 devtray 项目根识别仓库，提供只读状态、日志、diff，
//! 以及受控的 commit / push / pull。
//!
//! 安全边界：
//! - 不执行 force push、不自动 stash、不自动 merge/rebase；pull 只允许 fast-forward。
//! - 所有命令都不交互：禁用 Git 终端提示、SSH BatchMode，面板与批量任务不会卡在密码输入。
//! - commit 只接受「当前变更列表里存在的文件」，并在执行前重新读取状态，避免提交列表外的路径。
//! - 批量拉取跳过有未提交改动、正在变基/合并、没有上游或不是 Git 仓库的项目，并逐项汇报。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::{self, Config, ProjectRoot};
use crate::jobs::Job;

const LOCAL_TIMEOUT: Duration = Duration::from_secs(20);
const COMMIT_TIMEOUT: Duration = Duration::from_secs(120);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(90);
const DIFF_LIMIT: usize = 240_000;
const LOG_LIMIT: usize = 20;

// ── 进程执行 ────────────────────────────────────────────────────────────────

struct GitOutput {
    success: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

impl GitOutput {
    fn error(&self) -> String {
        if self.timed_out {
            return "git 命令超时（可能需要输入密码或网络不可用）".into();
        }
        let stderr = self.stderr.trim();
        if stderr.is_empty() {
            format!("git 命令失败（退出码 {}）", self.code.unwrap_or(-1))
        } else {
            stderr.to_string()
        }
    }
}

fn git(root: &Path, args: &[&str], timeout: Duration) -> Result<GitOutput, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(root)
        .arg("--no-pager")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // 面板与后台任务没有可交互的终端；宁可快速失败，也不要永远挂着。
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PAGER", "cat");
    // 用户自己设置过 GIT_SSH_COMMAND 时保持原样，否则补一个非交互的 ssh。
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        cmd.env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o ConnectTimeout=20",
        );
    }
    run(cmd, timeout)
}

fn run(mut cmd: Command, timeout: Duration) -> Result<GitOutput, String> {
    let mut child = cmd.spawn().map_err(|e| format!("无法执行 git：{e}"))?;
    let mut stdout = child.stdout.take().ok_or("无法读取 git 输出")?;
    let mut stderr = child.stderr.take().ok_or("无法读取 git 错误输出")?;
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("等待 git 失败：{e}"));
            }
        }
        if start.elapsed() > timeout {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let stdout = String::from_utf8_lossy(&out_reader.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    Ok(GitOutput {
        success: status.is_some_and(|s| s.success()),
        code: status.and_then(|s| s.code()),
        stdout,
        stderr,
        timed_out,
    })
}

// ── 数据结构 ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Change {
    pub path: String,
    /// 重命名/复制前的路径；提交时需要一并 stage。
    pub orig: Option<String>,
    /// porcelain 的 XY 状态码，例如 `M.`、`.M`、`R.`、`??`、`UU`。
    pub code: String,
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub conflict: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoInfo {
    /// 用于展示的主项目名（多个项目共用同一仓库时取第一个）。
    pub project: String,
    /// 共用同一仓库的所有项目名。
    pub projects: Vec<String>,
    pub root: String,
    pub is_repo: bool,
    pub error: Option<String>,
    pub branch: String,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub detached: bool,
    pub dirty: bool,
    pub operation: Option<String>,
    /// 仓库根过宽（家目录 / `/`）等提示；批量操作会跳过这类仓库。
    pub warning: Option<String>,
    pub changes: Vec<Change>,
}

impl RepoInfo {
    fn missing(project: &str, root: &Path, reason: String) -> Self {
        Self {
            project: project.into(),
            projects: vec![project.into()],
            root: root.display().to_string(),
            is_repo: false,
            error: Some(reason),
            branch: String::new(),
            head: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            detached: false,
            dirty: false,
            operation: None,
            warning: None,
            changes: Vec::new(),
        }
    }

    fn ready(&self) -> Result<(), String> {
        if !self.is_repo {
            return Err(self.error.clone().unwrap_or_else(|| "不是 Git 仓库".into()));
        }
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
struct StatusData {
    branch: String,
    head: Option<String>,
    upstream: Option<String>,
    ahead: u32,
    behind: u32,
    detached: bool,
    changes: Vec<Change>,
}

#[derive(Debug, Serialize)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    #[serde(default)]
    pub decorations: String,
}

#[derive(Debug, Deserialize)]
pub struct CommitRequest {
    pub project: String,
    pub message: String,
    #[serde(default)]
    pub paths: Vec<String>,
    /// CLI 的 `--all`：暂存全部变更。面板始终显式传勾选的文件。
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Serialize)]
pub struct CommitResult {
    pub project: String,
    pub hash: String,
    pub short: String,
    pub subject: String,
    pub files: usize,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PullResult {
    pub project: String,
    pub branch: String,
    pub upstream: String,
    pub updated: bool,
    pub commits: u64,
    pub old_head: Option<String>,
    pub new_head: Option<String>,
    pub warning: Option<String>,
    pub message: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct BatchItem {
    pub project: String,
    pub projects: Vec<String>,
    pub root: String,
    /// updated / uptodate / skipped / failed
    pub outcome: String,
    pub reason: Option<String>,
    pub commits: u64,
    pub old_head: Option<String>,
    pub new_head: Option<String>,
}

fn load_config() -> Result<Config, String> {
    let path = std::env::var("DEVTRAY_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| config::default_config_path());
    Config::load(&path)
}

// ── 仓库发现 ────────────────────────────────────────────────────────────────

fn toplevel(root: &Path) -> Result<PathBuf, String> {
    let out = git(root, &["rev-parse", "--show-toplevel"], LOCAL_TIMEOUT)?;
    if !out.success {
        return Err(out.error());
    }
    let line = out.stdout.trim();
    if line.is_empty() {
        return Err("不是 Git 仓库".into());
    }
    std::fs::canonicalize(line).map_err(|e| format!("仓库路径不可用：{e}"))
}

/// 仓库根过宽时禁止批量操作：家目录或 `/` 往往会让一次 pull 波及很多无关项目。
fn root_warning(root: &Path) -> Option<String> {
    let home = config::home_dir();
    if root == Path::new("/") {
        return Some("仓库根是 /，已禁止批量操作".into());
    }
    if !home.as_os_str().is_empty() && root == home {
        return Some("仓库根是家目录，已禁止批量操作".into());
    }
    if !home.as_os_str().is_empty() && root == home.join("Workspace") {
        return Some("仓库根是 ~/Workspace，已禁止批量操作".into());
    }
    None
}

fn operation(root: &Path) -> Option<String> {
    let out = git(root, &["rev-parse", "--absolute-git-dir"], LOCAL_TIMEOUT).ok()?;
    if !out.success {
        return None;
    }
    let dir = PathBuf::from(out.stdout.trim());
    let checks = [
        ("rebase-merge", "变基中"),
        ("rebase-apply", "变基中"),
        ("MERGE_HEAD", "合并中"),
        ("CHERRY_PICK_HEAD", "拣选中"),
        ("REVERT_HEAD", "回滚中"),
        ("BISECT_LOG", "二分中"),
    ];
    checks
        .iter()
        .find(|(name, _)| dir.join(name).exists())
        .map(|(_, label)| (*label).to_string())
}

fn parse_status(bytes: &[u8]) -> Result<StatusData, String> {
    let tokens: Vec<&[u8]> = bytes
        .split(|b| *b == 0)
        .filter(|token| !token.is_empty())
        .collect();
    let mut data = StatusData::default();
    let mut i = 0;
    while i < tokens.len() {
        let text = std::str::from_utf8(tokens[i])
            .map_err(|_| "Git 返回了非 UTF-8 路径，暂不支持在面板中操作".to_string())?;
        if let Some(value) = text.strip_prefix("# branch.head ") {
            data.branch = value.to_string();
            data.detached = value == "(detached)";
        } else if let Some(value) = text.strip_prefix("# branch.oid ") {
            if value != "(initial)" {
                data.head = Some(value.to_string());
            }
        } else if let Some(value) = text.strip_prefix("# branch.upstream ") {
            data.upstream = Some(value.to_string());
        } else if let Some(value) = text.strip_prefix("# branch.ab ") {
            for part in value.split_whitespace() {
                if let Some(v) = part.strip_prefix('+') {
                    data.ahead = v.parse().unwrap_or(0);
                } else if let Some(v) = part.strip_prefix('-') {
                    data.behind = v.parse().unwrap_or(0);
                }
            }
        } else if text.starts_with("1 ") {
            let fields: Vec<&str> = text.splitn(9, ' ').collect();
            if fields.len() == 9 {
                data.changes.push(change_from(fields[1], fields[8], None));
            }
        } else if text.starts_with("2 ") {
            let fields: Vec<&str> = text.splitn(10, ' ').collect();
            if fields.len() == 10 {
                let orig = tokens
                    .get(i + 1)
                    .and_then(|token| std::str::from_utf8(token).ok())
                    .map(str::to_string);
                data.changes.push(change_from(fields[1], fields[9], orig));
                i += 1;
            }
        } else if text.starts_with("u ") {
            let fields: Vec<&str> = text.splitn(11, ' ').collect();
            if fields.len() == 11 {
                let mut change = change_from(fields[1], fields[10], None);
                change.conflict = true;
                data.changes.push(change);
            }
        } else if let Some(path) = text.strip_prefix("? ") {
            data.changes.push(Change {
                path: path.to_string(),
                orig: None,
                code: "??".into(),
                staged: false,
                unstaged: true,
                untracked: true,
                conflict: false,
            });
        }
        i += 1;
    }
    data.changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(data)
}

fn change_from(xy: &str, path: &str, orig: Option<String>) -> Change {
    let mut chars = xy.chars();
    let (x, y) = (chars.next().unwrap_or('.'), chars.next().unwrap_or('.'));
    Change {
        path: path.to_string(),
        orig,
        code: xy.to_string(),
        staged: x != '.',
        unstaged: y != '.',
        untracked: false,
        conflict: false,
    }
}

fn status_of(root: &Path) -> Result<StatusData, String> {
    let out = git(
        root,
        &["status", "--porcelain=v2", "--branch", "-z"],
        LOCAL_TIMEOUT,
    )?;
    if !out.success {
        return Err(out.error());
    }
    parse_status(out.stdout.as_bytes())
}

fn describe(project: &str, root: &Path) -> RepoInfo {
    let mut info = RepoInfo {
        project: project.into(),
        projects: vec![project.into()],
        root: root.display().to_string(),
        is_repo: true,
        error: None,
        branch: String::new(),
        head: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        detached: false,
        dirty: false,
        operation: operation(root),
        warning: root_warning(root),
        changes: Vec::new(),
    };
    match status_of(root) {
        Ok(status) => {
            info.branch = status.branch;
            info.head = status.head;
            info.upstream = status.upstream;
            info.ahead = status.ahead;
            info.behind = status.behind;
            info.detached = status.detached;
            info.dirty = !status.changes.is_empty();
            info.changes = status.changes;
        }
        Err(error) => info.error = Some(error),
    }
    info
}

/// 所有项目根 → 去重后的 Git 仓库；不是仓库的项目也保留一行，便于界面说明原因。
pub fn repos_from(cfg: &Config) -> Result<Vec<RepoInfo>, String> {
    let roots = cfg.project_roots()?;
    let mut out: Vec<RepoInfo> = Vec::new();
    let mut index: Vec<(String, usize)> = Vec::new();
    for p in roots {
        match toplevel(&p.root) {
            Ok(repo_root) => {
                let key = repo_root.display().to_string();
                if let Some((_, existing)) = index.iter().find(|(k, _)| *k == key) {
                    out[*existing].projects.push(p.project.clone());
                } else {
                    let info = describe(&p.project, &repo_root);
                    index.push((key, out.len()));
                    out.push(info);
                }
            }
            Err(reason) => out.push(RepoInfo::missing(&p.project, &p.root, reason)),
        }
    }
    Ok(out)
}

pub fn repos() -> Result<Vec<RepoInfo>, String> {
    repos_from(&load_config()?)
}

fn find_project<'a>(roots: &'a [ProjectRoot], project: &str) -> Option<&'a ProjectRoot> {
    let expanded = config::expand_home(project);
    roots
        .iter()
        .find(|r| r.project == project || r.root == expanded)
}

fn resolve_in(cfg: &Config, project: &str) -> Result<RepoInfo, String> {
    let roots = cfg.project_roots()?;
    let matched = find_project(&roots, project).ok_or_else(|| {
        format!(
            "找不到 Git 项目 {project:?}。可用：{}",
            roots
                .iter()
                .map(|r| r.project.clone())
                .collect::<Vec<_>>()
                .join("、")
        )
    })?;
    let repo_root = toplevel(&matched.root)
        .map_err(|e| format!("项目 {} 的根目录不可用于 Git：{e}", matched.project))?;
    let mut info = describe(&matched.project, &repo_root);
    // 同一仓库被多个项目引用时，把项目名都带上，避免界面显示歧义。
    info.projects = roots
        .iter()
        .filter(|r| toplevel(&r.root).is_ok_and(|root| root == repo_root))
        .map(|r| r.project.clone())
        .collect();
    Ok(info)
}

fn resolve(project: &str) -> Result<RepoInfo, String> {
    resolve_in(&load_config()?, project)
}

// ── 只读操作 ────────────────────────────────────────────────────────────────

pub fn log_entries(root: &Path, limit: usize) -> Result<Vec<Commit>, String> {
    let n = limit.clamp(1, 200).to_string();
    let out = git(
        root,
        &[
            "log",
            "-n",
            &n,
            "--date=short",
            "--pretty=format:%H%x1f%h%x1f%an%x1f%ad%x1f%s%x1f%D%x1e",
        ],
        LOCAL_TIMEOUT,
    )?;
    if !out.success {
        let text = out.error();
        // 空仓库还没有提交：返回空列表，而不是把「does not have any commits」当错误弹给用户。
        if text.contains("does not have any commits")
            || text.contains("还没有任何提交")
            || text.contains("bad revision")
        {
            return Ok(Vec::new());
        }
        return Err(text);
    }
    let mut commits = Vec::new();
    for record in out.stdout.split('\u{1e}') {
        let record = record.trim_start_matches('\n');
        if record.trim().is_empty() {
            continue;
        }
        let mut fields = record.split('\u{1f}');
        let hash = fields.next().unwrap_or_default();
        let short = fields.next().unwrap_or_default();
        let author = fields.next().unwrap_or_default();
        let date = fields.next().unwrap_or_default();
        let subject = fields.next().unwrap_or_default();
        let decorations = fields.next().unwrap_or_default();
        commits.push(Commit {
            hash: hash.to_string(),
            short: short.to_string(),
            author: author.to_string(),
            date: date.to_string(),
            subject: subject.to_string(),
            decorations: decorations.to_string(),
        });
    }
    Ok(commits)
}

#[derive(Debug, Serialize)]
pub struct DiffResult {
    pub path: Option<String>,
    pub diff: String,
    pub truncated: bool,
    pub untracked: bool,
    pub binary: bool,
}

fn read_untracked(root: &Path, path: &str) -> DiffResult {
    let full = root.join(path);
    if full.is_dir() {
        let mut names: Vec<String> = std::fs::read_dir(&full)
            .map(|entries| {
                entries
                    .flatten()
                    .take(50)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        return DiffResult {
            path: Some(path.into()),
            diff: format!("新目录（未跟踪，{} 项）\n{}", names.len(), names.join("\n")),
            truncated: false,
            untracked: true,
            binary: false,
        };
    }
    let data = match std::fs::read(&full) {
        Ok(data) => data,
        Err(e) => {
            return DiffResult {
                path: Some(path.into()),
                diff: format!("无法读取文件：{e}"),
                truncated: false,
                untracked: true,
                binary: false,
            };
        }
    };
    if data.iter().take(8000).any(|b| *b == 0) {
        return DiffResult {
            path: Some(path.into()),
            diff: format!("新文件（未跟踪，二进制，{} 字节）", data.len()),
            truncated: false,
            untracked: true,
            binary: true,
        };
    }
    let text = String::from_utf8_lossy(&data);
    let (body, truncated) = truncate(&text, DIFF_LIMIT);
    DiffResult {
        path: Some(path.into()),
        diff: body,
        truncated,
        untracked: true,
        binary: false,
    }
}

fn truncate(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_string(), false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (
        format!(
            "{}\n\n… 内容过长，已截断到 {} KB",
            &text[..end],
            limit / 1024
        ),
        true,
    )
}

pub fn diff_at(root: &Path, path: Option<&str>, staged: bool) -> Result<DiffResult, String> {
    let status = status_of(root)?;
    if let Some(path) = path {
        let rel = Path::new(path);
        if rel.is_absolute() || path.split(['/', '\\']).any(|part| part == "..") {
            return Err("只接受仓库内的相对路径".into());
        }
        if !status.changes.iter().any(|c| c.path == path) {
            return Err(format!("{path} 不在当前变更列表中，请刷新后重试"));
        }
        if let Some(change) = status.changes.iter().find(|c| c.path == path)
            && change.untracked
        {
            return Ok(read_untracked(root, path));
        }
    }
    let mut args = vec!["diff", "--no-color", "-U3"];
    if staged {
        args.push("--cached");
    }
    if status.head.is_some() {
        args.push("HEAD");
    }
    if let Some(path) = path {
        args.extend(["--", path]);
    }
    let out = git(root, &args, LOCAL_TIMEOUT)?;
    if !out.success {
        return Err(out.error());
    }
    if out.stdout.trim().is_empty() {
        return Ok(DiffResult {
            path: path.map(str::to_string),
            diff: if staged {
                "（已暂存区没有差异）".into()
            } else {
                "（与 HEAD 没有差异）".into()
            },
            truncated: false,
            untracked: false,
            binary: false,
        });
    }
    let (diff, truncated) = truncate(&out.stdout, DIFF_LIMIT);
    Ok(DiffResult {
        path: path.map(str::to_string),
        diff,
        truncated,
        untracked: false,
        binary: false,
    })
}

// ── 写操作 ──────────────────────────────────────────────────────────────────

fn ensure_clean_operation(repo: &RepoInfo) -> Result<(), String> {
    if repo.operation.as_deref() == Some("变基中") {
        return Err("仓库正在变基，请先在终端完成或中止变基".into());
    }
    Ok(())
}

fn head_oid(root: &Path) -> Option<String> {
    let out = git(root, &["rev-parse", "HEAD"], LOCAL_TIMEOUT).ok()?;
    (out.success && !out.stdout.trim().is_empty()).then(|| out.stdout.trim().to_string())
}

fn rev_count(root: &Path, range: &str) -> u64 {
    git(root, &["rev-list", "--count", range], LOCAL_TIMEOUT)
        .ok()
        .filter(|out| out.success)
        .and_then(|out| out.stdout.trim().parse().ok())
        .unwrap_or(0)
}

fn add_paths(change: &Change) -> Vec<String> {
    let mut paths = vec![change.path.clone()];
    if let Some(orig) = &change.orig {
        paths.push(orig.clone());
    }
    paths
}

fn commit_repo(repo: &RepoInfo, req: &CommitRequest) -> Result<CommitResult, String> {
    repo.ready()?;
    let message = req.message.trim();
    if message.is_empty() {
        return Err("提交信息不能为空".into());
    }
    if message.chars().count() > 20_000 {
        return Err("提交信息过长（上限 20000 字符）".into());
    }
    ensure_clean_operation(repo)?;
    let root = Path::new(&repo.root);
    let status = status_of(root)?;
    let mut selected: Vec<String> = Vec::new();
    if req.all {
        if !req.paths.is_empty() {
            return Err("--all 与 --path 不能同时使用".into());
        }
        selected.push("-A".into());
    } else {
        if req.paths.is_empty() {
            return Err("先勾选要提交的文件".into());
        }
        let mut add: Vec<String> = Vec::new();
        for path in &req.paths {
            let rel = Path::new(path);
            if rel.is_absolute() || path.split(['/', '\\']).any(|part| part == "..") {
                return Err(format!("只接受仓库内的相对路径：{path}"));
            }
            let change = status
                .changes
                .iter()
                .find(|c| &c.path == path)
                .ok_or_else(|| format!("{path} 已不在变更列表中，请刷新后重试"))?;
            add.extend(add_paths(change));
        }
        add.sort();
        add.dedup();
        selected.extend(add);
    }
    let mut args: Vec<&str> = vec!["add"];
    args.extend(selected.iter().map(String::as_str));
    args.push("--");
    let out = git(root, &args, COMMIT_TIMEOUT)?;
    if !out.success {
        return Err(format!("暂存失败：{}", out.error()));
    }
    let staged = git(root, &["diff", "--cached", "--name-only"], LOCAL_TIMEOUT)?;
    if !staged.success {
        return Err(format!("读取暂存区失败：{}", staged.error()));
    }
    let files = staged
        .stdout
        .lines()
        .filter(|line| !line.is_empty())
        .count();
    if files == 0 {
        return Err("所选文件没有可提交的改动".into());
    }
    let out = git(root, &["commit", "-m", message], COMMIT_TIMEOUT)?;
    if !out.success {
        return Err(format!("提交失败：{}", out.error()));
    }
    let short = git(root, &["rev-parse", "--short", "HEAD"], LOCAL_TIMEOUT)
        .ok()
        .filter(|out| out.success)
        .map(|out| out.stdout.trim().to_string())
        .unwrap_or_default();
    let subject = git(root, &["log", "-1", "--pretty=%s"], LOCAL_TIMEOUT)
        .ok()
        .filter(|out| out.success)
        .map(|out| out.stdout.trim().to_string())
        .unwrap_or_else(|| message.lines().next().unwrap_or(message).to_string());
    let note = match repo.operation.as_deref() {
        Some("合并中") => Some("合并已提交完成".into()),
        Some("拣选中") => Some("拣选已提交完成".into()),
        Some("回滚中") => Some("回滚已提交完成".into()),
        _ => None,
    };
    Ok(CommitResult {
        project: repo.project.clone(),
        hash: short.clone(),
        short,
        subject,
        files,
        note,
    })
}

pub fn commit(req: CommitRequest) -> Result<CommitResult, String> {
    let repo = resolve(&req.project)?;
    commit_repo(&repo, &req)
}

/// 面板入口：提交参数走 stdin（JSON），避免把提交信息塞进命令行。
pub fn commit_command() -> ExitCode {
    emit((|| -> Result<String, String> {
        let mut input = String::new();
        std::io::stdin()
            .take(1024 * 1024)
            .read_to_string(&mut input)
            .map_err(|e| e.to_string())?;
        let req: CommitRequest =
            serde_json::from_str(&input).map_err(|e| format!("提交参数格式错误：{e}"))?;
        serde_json::to_string(&commit(req)?).map_err(|e| e.to_string())
    })())
}

fn push_repo(repo: &RepoInfo) -> Result<serde_json::Value, String> {
    repo.ready()?;
    ensure_clean_operation(repo)?;
    if repo.detached {
        return Err("当前处于分离 HEAD，无法推送；请先切换回分支".into());
    }
    let upstream = repo.upstream.clone().ok_or_else(|| {
        format!(
            "分支 {} 没有上游分支；先在终端执行：git push -u origin {}",
            repo.branch, repo.branch
        )
    })?;
    let ahead = rev_count(Path::new(&repo.root), &format!("{upstream}..HEAD"));
    let out = git(Path::new(&repo.root), &["push"], NETWORK_TIMEOUT)?;
    if !out.success {
        return Err(format!("推送失败：{}", out.error()));
    }
    let message = if ahead == 0 {
        format!("{} → {}（远端已是最新）", repo.branch, upstream)
    } else {
        format!("已推送 {} → {}（{ahead} 个提交）", repo.branch, upstream)
    };
    Ok(json!({
        "project": repo.project,
        "branch": repo.branch,
        "upstream": upstream,
        "commits": ahead,
        "message": message,
    }))
}

fn pull_blocker(repo: &RepoInfo, allow_dirty: bool) -> Option<String> {
    if !repo.is_repo {
        return Some(repo.error.clone().unwrap_or_else(|| "不是 Git 仓库".into()));
    }
    if let Some(error) = &repo.error {
        return Some(error.clone());
    }
    if let Some(operation) = &repo.operation {
        return Some(format!("仓库{operation}，已跳过"));
    }
    if repo.detached {
        return Some("当前处于分离 HEAD，已跳过".into());
    }
    if repo.upstream.is_none() {
        return Some("没有上游分支（先执行 git push -u origin <分支>）".into());
    }
    if let Some(warning) = &repo.warning {
        return Some(warning.clone());
    }
    if !allow_dirty && repo.dirty {
        return Some("有未提交改动，已跳过".into());
    }
    None
}

fn execute_pull(repo: &RepoInfo) -> Result<PullResult, String> {
    let root = Path::new(&repo.root);
    let upstream = repo
        .upstream
        .clone()
        .ok_or_else(|| "没有上游分支".to_string())?;
    let old_head = head_oid(root);
    let out = git(root, &["pull", "--ff-only", "--no-edit"], NETWORK_TIMEOUT)?;
    if !out.success {
        return Err(format!(
            "拉取失败：{}\n未执行合并或 rebase；本地分叉或冲突请到终端处理。",
            out.error()
        ));
    }
    let new_head = head_oid(root);
    let updated = old_head != new_head;
    let commits = if updated {
        match (&old_head, &new_head) {
            (Some(old), Some(new)) => rev_count(root, &format!("{old}..{new}")),
            _ => 0,
        }
    } else {
        0
    };
    let message = if updated {
        format!("已拉取 {}（{commits} 个提交）", repo.branch)
    } else {
        format!("{} 已是最新", repo.branch)
    };
    Ok(PullResult {
        project: repo.project.clone(),
        branch: repo.branch.clone(),
        upstream,
        updated,
        commits,
        old_head,
        new_head,
        warning: repo.dirty.then(|| "拉取前工作区有未提交改动".into()),
        message,
    })
}

fn pull_repo(repo: &RepoInfo, allow_dirty: bool) -> Result<PullResult, String> {
    if let Some(reason) = pull_blocker(repo, allow_dirty) {
        return Err(reason);
    }
    execute_pull(repo)
}

// ── 批量拉取任务 ────────────────────────────────────────────────────────────

fn batch_item(repo: &RepoInfo) -> BatchItem {
    let base = |outcome: &str, reason: Option<String>| BatchItem {
        project: repo.project.clone(),
        projects: repo.projects.clone(),
        root: repo.root.clone(),
        outcome: outcome.into(),
        reason,
        commits: 0,
        old_head: None,
        new_head: None,
    };
    if !repo.is_repo || repo.error.is_some() {
        let reason = repo.error.clone().unwrap_or_else(|| "不是 Git 仓库".into());
        return base("failed", Some(reason));
    }
    if let Some(reason) = pull_blocker(repo, false) {
        return base("skipped", Some(reason));
    }
    match execute_pull(repo) {
        Ok(result) if result.updated => BatchItem {
            project: repo.project.clone(),
            projects: repo.projects.clone(),
            root: repo.root.clone(),
            outcome: "updated".into(),
            reason: None,
            commits: result.commits,
            old_head: result.old_head,
            new_head: result.new_head,
        },
        Ok(result) => base("uptodate", Some(result.message)),
        Err(error) => base("failed", Some(error)),
    }
}

fn summary(items: &[BatchItem]) -> serde_json::Value {
    let count = |name: &str| items.iter().filter(|item| item.outcome == name).count();
    json!({
        "updated": count("updated"),
        "uptodate": count("uptodate"),
        "skipped": count("skipped"),
        "failed": count("failed"),
        "total": items.len(),
    })
}

fn summary_text(items: &[BatchItem]) -> String {
    let updated = items.iter().filter(|i| i.outcome == "updated").count();
    let uptodate = items.iter().filter(|i| i.outcome == "uptodate").count();
    let skipped = items.iter().filter(|i| i.outcome == "skipped").count();
    let failed = items.iter().filter(|i| i.outcome == "failed").count();
    format!("更新 {updated} 个，已最新 {uptodate} 个，跳过 {skipped} 个，失败 {failed} 个")
}

pub fn start_pull_command(args: &[String]) -> ExitCode {
    let result = (|| -> Result<String, String> {
        let mut wanted: Vec<String> = Vec::new();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--project" => wanted.push(it.next().ok_or("--project 缺少项目名")?.clone()),
                "--json" => {}
                other => return Err(format!("未知选项：{other}")),
            }
        }
        if wanted.is_empty() && !args.is_empty() && !args.iter().all(|a| a == "--json") {
            return Err("用法：devtray _git-pull-start [--project 名称]...".into());
        }
        let repos = repos()?;
        let eligible: Vec<&RepoInfo> = repos
            .iter()
            .filter(|repo| repo.is_repo)
            .filter(|repo| wanted.is_empty() || repo.projects.iter().any(|p| wanted.contains(p)))
            .collect();
        if eligible.is_empty() {
            return Err("没有可拉取的 Git 仓库；请检查项目根目录与 Git 配置".into());
        }
        let roots: Vec<String> = eligible.iter().map(|r| r.root.clone()).collect();
        let mut job = Job::new("git_pull", None, &roots);
        job.source = "panel".into();
        job.total = roots.len() as u64;
        job.message = Some(format!("准备拉取 {} 个仓库", roots.len()));
        job.save()?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut cmd = Command::new(exe);
        cmd.arg("_git-pull-run")
            .arg(&job.id)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::proc::spawn_detached(&mut cmd).map_err(|e| format!("启动拉取任务失败：{e}"))?;
        Ok(json!({
            "job_id": job.id,
            "count": roots.len(),
            "repos": eligible.iter().map(|r| json!({"project": r.project, "root": r.root})).collect::<Vec<_>>(),
        })
        .to_string())
    })();
    match result {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

pub fn run_pull_job(id: Option<&String>) -> ExitCode {
    let id = match id {
        Some(id) => id.clone(),
        None => {
            eprintln!("用法：devtray _git-pull-run <job-id>");
            return ExitCode::FAILURE;
        }
    };
    let result = (|| -> Result<(), String> {
        let mut job = Job::load(&id)?;
        let roots: Vec<String> = job.paths.clone();
        job.mark_running(Some("准备拉取"));
        job.total = roots.len() as u64;
        job.save()?;
        let repos = repos()?;
        let total = roots.len().max(1);
        let mut items: Vec<BatchItem> = Vec::new();
        for (i, root) in roots.iter().enumerate() {
            let item = match repos.iter().find(|r| &r.root == root) {
                Some(repo) => {
                    job.message = Some(format!("拉取 {}", repo.project));
                    job.save()?;
                    batch_item(repo)
                }
                None => BatchItem {
                    project: root.clone(),
                    projects: vec![],
                    root: root.clone(),
                    outcome: "skipped".into(),
                    reason: Some("仓库已不可用（配置可能已修改）".into()),
                    commits: 0,
                    old_head: None,
                    new_head: None,
                },
            };
            items.push(item);
            job.progress = (i + 1) as f32 / total as f32;
            job.message = Some(summary_text(&items));
            job.save()?;
        }
        job.finish_ok(json!({
            "items": items,
            "summary": summary(&items),
            "message": summary_text(&items),
        }));
        job.save()?;
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if let Ok(mut job) = Job::load(&id) {
                job.finish_error(error.clone());
                let _ = job.save();
            }
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

// ── CLI ─────────────────────────────────────────────────────────────────────

fn emit(result: Result<String, String>) -> ExitCode {
    match result {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn status_line(repo: &RepoInfo) -> String {
    if !repo.is_repo {
        return format!(
            "{:<24} {:<18} {}",
            repo.project,
            "非 Git 仓库",
            repo.error.as_deref().unwrap_or("")
        );
    }
    let mut state = repo.branch.clone();
    if repo.detached {
        state = "分离 HEAD".into();
    }
    if repo.ahead > 0 {
        state.push_str(&format!(" ↑{}", repo.ahead));
    }
    if repo.behind > 0 {
        state.push_str(&format!(" ↓{}", repo.behind));
    }
    if let Some(operation) = &repo.operation {
        state.push_str(&format!(" · {operation}"));
    }
    if repo.dirty {
        state.push_str(&format!(" · {} 处变更", repo.changes.len()));
    } else {
        state.push_str(" · 干净");
    }
    if let Some(error) = &repo.error {
        state.push_str(&format!(" · {error}"));
    }
    format!("{:<24} {:<28} {}", repo.project, state, repo.root)
}

fn usage() -> String {
    "用法：
  devtray git status [--json]
  devtray git log <项目> [-n N] [--json]
  devtray git diff <项目> [--path 文件] [--staged] [--json]
  devtray git commit <项目> -m \"提交信息\" (--all | --path 文件...)
  devtray git push <项目>
  devtray git pull <项目> | --all

约定：pull 只做 fast-forward；push 不做 force；有未提交改动的仓库在批量拉取时跳过。"
        .into()
}

pub fn command(args: &[String]) -> ExitCode {
    let Some(sub) = args.first().map(String::as_str) else {
        return emit(Err(usage()));
    };
    match sub {
        "status" => emit(status_cli(&args[1..])),
        "log" => emit(log_cli(&args[1..])),
        "diff" => emit(diff_cli(&args[1..])),
        "commit" => emit(commit_cli(&args[1..])),
        "push" => emit(push_cli(&args[1..])),
        "pull" => emit(pull_cli(&args[1..])),
        "help" | "--help" | "-h" => {
            println!("{}", usage());
            ExitCode::SUCCESS
        }
        other => emit(Err(format!("未知 git 子命令：{other}\n\n{}", usage()))),
    }
}

fn status_cli(args: &[String]) -> Result<String, String> {
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            other => return Err(format!("未知选项：{other}")),
        }
    }
    let repos = repos()?;
    if json {
        return serde_json::to_string(&json!({"repos": repos})).map_err(|e| e.to_string());
    }
    if repos.is_empty() {
        return Ok("（配置里没有任何项目）".into());
    }
    Ok(repos.iter().map(status_line).collect::<Vec<_>>().join("\n"))
}

fn log_cli(args: &[String]) -> Result<String, String> {
    let mut project = None;
    let mut json = false;
    let mut limit = LOG_LIMIT;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" => json = true,
            "-n" => {
                limit = it
                    .next()
                    .ok_or("-n 缺少行数")?
                    .parse()
                    .map_err(|_| "-n 需要整数".to_string())?;
            }
            other if other.starts_with('-') => return Err(format!("未知选项：{other}")),
            other if project.is_none() => project = Some(other.to_string()),
            other => return Err(format!("多余的参数：{other}")),
        }
    }
    let project = project.ok_or("用法：devtray git log <项目> [-n N] [--json]")?;
    let repo = resolve(&project)?;
    repo.ready()?;
    let commits = log_entries(Path::new(&repo.root), limit)?;
    if json {
        return serde_json::to_string(&json!({
            "project": repo.project,
            "root": repo.root,
            "branch": repo.branch,
            "commits": commits,
        }))
        .map_err(|e| e.to_string());
    }
    if commits.is_empty() {
        return Ok(format!(
            "{} 还没有提交（分支 {}）",
            repo.project, repo.branch
        ));
    }
    Ok(commits
        .iter()
        .map(|c| format!("{:<9} {}  {:<12} {}", c.short, c.date, c.author, c.subject))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn diff_cli(args: &[String]) -> Result<String, String> {
    let mut project = None;
    let mut path = None;
    let mut staged = false;
    let mut json = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--staged" => staged = true,
            "--path" => path = Some(it.next().ok_or("--path 缺少文件")?.clone()),
            other if other.starts_with('-') => return Err(format!("未知选项：{other}")),
            other if project.is_none() => project = Some(other.to_string()),
            other => return Err(format!("多余的参数：{other}")),
        }
    }
    let project = project.ok_or("用法：devtray git diff <项目> [--path 文件] [--staged]")?;
    let repo = resolve(&project)?;
    repo.ready()?;
    let result = diff_at(Path::new(&repo.root), path.as_deref(), staged)?;
    if json {
        return serde_json::to_string(&result).map_err(|e| e.to_string());
    }
    Ok(result.diff)
}

fn commit_cli(args: &[String]) -> Result<String, String> {
    let mut project = None;
    let mut message = None;
    let mut paths = Vec::new();
    let mut all = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-m" | "--message" => message = Some(it.next().ok_or("-m 缺少提交信息")?.clone()),
            "--all" | "-a" => all = true,
            "--path" => paths.push(it.next().ok_or("--path 缺少文件")?.clone()),
            other if other.starts_with('-') => return Err(format!("未知选项：{other}")),
            other if project.is_none() => project = Some(other.to_string()),
            other => return Err(format!("多余的参数：{other}")),
        }
    }
    let project =
        project.ok_or("用法：devtray git commit <项目> -m 信息 (--all | --path 文件...)")?;
    let message = message.ok_or("缺少提交信息：-m \"...\"")?;
    let result = commit(CommitRequest {
        project,
        message,
        paths,
        all,
    })?;
    Ok(format!(
        "已提交 {}：{}（{} 个文件）{}",
        result.short,
        result.subject,
        result.files,
        result
            .note
            .as_deref()
            .map(|note| format!("\n{note}"))
            .unwrap_or_default()
    ))
}

fn push_cli(args: &[String]) -> Result<String, String> {
    let mut project = None;
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            other if other.starts_with('-') => return Err(format!("未知选项：{other}")),
            other if project.is_none() => project = Some(other.to_string()),
            other => return Err(format!("多余的参数：{other}")),
        }
    }
    let project = project.ok_or("用法：devtray git push <项目>")?;
    let repo = resolve(&project)?;
    let value = push_repo(&repo)?;
    if json {
        return serde_json::to_string(&value).map_err(|e| e.to_string());
    }
    Ok(value["message"].as_str().unwrap_or("已推送").to_string())
}

fn pull_cli(args: &[String]) -> Result<String, String> {
    let mut project = None;
    let mut all = false;
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--all" => all = true,
            "--json" => json = true,
            other if other.starts_with('-') => return Err(format!("未知选项：{other}")),
            other if project.is_none() => project = Some(other.to_string()),
            other => return Err(format!("多余的参数：{other}")),
        }
    }
    if all {
        if project.is_some() {
            return Err("--all 不能同时指定项目".into());
        }
        let repos = repos()?;
        let mut items = Vec::new();
        for repo in repos.iter().filter(|repo| repo.is_repo) {
            items.push(batch_item(repo));
        }
        let summary = summary(&items);
        if json {
            return serde_json::to_string(&json!({"items": items, "summary": summary}))
                .map_err(|e| e.to_string());
        }
        let mut lines: Vec<String> = items
            .iter()
            .map(|item| {
                format!(
                    "{:<24} {:<9} {}",
                    item.project,
                    item.outcome,
                    item.reason.clone().unwrap_or_default()
                )
            })
            .collect();
        lines.push(summary_text(&items));
        return Ok(lines.join("\n"));
    }
    let project = project.ok_or("用法：devtray git pull <项目> 或 devtray git pull --all")?;
    let repo = resolve(&project)?;
    let result = pull_repo(&repo, true)?;
    if json {
        return serde_json::to_string(&result).map_err(|e| e.to_string());
    }
    Ok(format!(
        "{}{}",
        result.message,
        result
            .warning
            .as_deref()
            .map(|w| format!("（{w}）"))
            .unwrap_or_default()
    ))
}

// ── 测试 ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "devtray-git-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sh(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "tester")
            .env("GIT_AUTHOR_EMAIL", "tester@example.com")
            .env("GIT_COMMITTER_NAME", "tester")
            .env("GIT_COMMITTER_EMAIL", "tester@example.com")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo(tag: &str) -> PathBuf {
        let dir = temp_dir(tag);
        sh(&dir, &["init", "-q"]);
        sh(&dir, &["config", "user.email", "tester@example.com"]);
        sh(&dir, &["config", "user.name", "tester"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        sh(&dir, &["add", "a.txt"]);
        sh(&dir, &["commit", "-qm", "init"]);
        dir
    }

    fn project(name: &str, root: &Path) -> ProjectRoot {
        ProjectRoot {
            project: name.into(),
            root: root.to_path_buf(),
            inferred: false,
            cleaning_allowed: true,
            reason: None,
            notes: Vec::new(),
        }
    }

    fn config_for(roots: &[ProjectRoot]) -> Config {
        let text = roots
            .iter()
            .map(|r| format!("[[project]]\nname = {:?}\nroot = {:?}\n", r.project, r.root))
            .collect::<String>();
        toml::from_str(&text).unwrap()
    }

    #[test]
    fn parses_porcelain_with_rename_untracked_and_upstream() {
        let raw = b"# branch.oid abcdef\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -3\0\
1 MM N... 100644 100644 100644 aaa bbb src/a.rs\0\
2 R. N... 100644 100644 100644 ccc ddd R100 d.txt\0\
b.txt\0\
? c.txt\0";
        let status = parse_status(raw).unwrap();
        assert_eq!(status.branch, "main");
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (2, 3));
        assert_eq!(status.changes.len(), 3);

        let find = |path: &str| {
            status
                .changes
                .iter()
                .find(|change| change.path == path)
                .unwrap_or_else(|| panic!("missing change {path}"))
        };
        let modified = find("src/a.rs");
        assert!(modified.staged && modified.unstaged);
        assert_eq!(modified.code, "MM");

        let renamed = find("d.txt");
        assert_eq!(renamed.orig.as_deref(), Some("b.txt"));
        assert!(renamed.staged && !renamed.unstaged);

        let untracked = find("c.txt");
        assert!(untracked.untracked && !untracked.staged);
    }

    #[test]
    fn parses_detached_and_unborn_heads() {
        let detached = parse_status(b"# branch.oid abc\0# branch.head (detached)\0").unwrap();
        assert!(detached.detached);
        let unborn = parse_status(b"# branch.oid (initial)\0# branch.head main\0").unwrap();
        assert!(unborn.head.is_none());
    }

    #[test]
    fn discovers_repo_and_reports_changes() {
        let dir = init_repo("discover");
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.join("new.txt"), "new\n").unwrap();
        let cfg = config_for(&[project("demo", &dir)]);
        let repos = repos_from(&cfg).unwrap();
        assert_eq!(repos.len(), 1);
        let repo = &repos[0];
        assert!(repo.is_repo);
        assert!(repo.dirty);
        assert!(repo.branch == "main" || repo.branch == "master");
        assert!(repo.changes.iter().any(|c| c.path == "a.txt"));
        assert!(
            repo.changes
                .iter()
                .any(|c| c.path == "new.txt" && c.untracked)
        );
    }

    #[test]
    fn deduplicates_projects_sharing_a_repo() {
        let dir = init_repo("dedupe");
        let nested = dir.join("pkg");
        std::fs::create_dir_all(&nested).unwrap();
        let cfg = config_for(&[project("root", &dir), project("nested", &nested)]);
        let repos = repos_from(&cfg).unwrap();
        assert_eq!(repos.len(), 1, "same repository should appear once");
        assert_eq!(repos[0].projects, vec!["root", "nested"]);
    }

    #[test]
    fn marks_non_repo_projects_without_failing_the_list() {
        let plain = temp_dir("plain");
        let repo_dir = init_repo("mixed");
        let cfg = config_for(&[project("plain", &plain), project("repo", &repo_dir)]);
        let repos = repos_from(&cfg).unwrap();
        assert_eq!(repos.len(), 2);
        assert!(repos.iter().any(|r| !r.is_repo && r.project == "plain"));
    }

    #[test]
    fn commit_only_stages_selected_files() {
        let dir = init_repo("commit");
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.join("b.txt"), "brand new\n").unwrap();
        let repo = describe("demo", &dir);
        let result = commit_repo(
            &repo,
            &CommitRequest {
                project: "demo".into(),
                message: "update a".into(),
                paths: vec!["a.txt".into()],
                all: false,
            },
        )
        .unwrap();
        assert_eq!(result.files, 1);
        assert_eq!(result.subject, "update a");
        let status = status_of(&dir).unwrap();
        assert!(
            status
                .changes
                .iter()
                .any(|c| c.path == "b.txt" && c.untracked)
        );
        assert!(!status.changes.iter().any(|c| c.path == "a.txt"));
    }

    #[test]
    fn commit_rejects_paths_outside_the_change_list() {
        let dir = init_repo("commit-fence");
        let repo = describe("demo", &dir);
        let error = commit_repo(
            &repo,
            &CommitRequest {
                project: "demo".into(),
                message: "nope".into(),
                paths: vec!["../outside.txt".into()],
                all: false,
            },
        )
        .unwrap_err();
        assert!(error.contains("相对路径"), "{error}");
        let error = commit_repo(
            &repo,
            &CommitRequest {
                project: "demo".into(),
                message: "nope".into(),
                paths: vec!["missing.txt".into()],
                all: false,
            },
        )
        .unwrap_err();
        assert!(error.contains("不在变更列表"), "{error}");
    }

    #[test]
    fn pull_fast_forwards_and_batch_skips_dirty() {
        let remote = temp_dir("remote");
        sh(&remote, &["init", "-q", "--bare"]);
        let work = temp_dir("work");
        sh(&work, &["clone", "-q", remote.to_str().unwrap(), "."]);
        sh(&work, &["config", "user.email", "tester@example.com"]);
        sh(&work, &["config", "user.name", "tester"]);
        std::fs::write(work.join("a.txt"), "one\n").unwrap();
        sh(&work, &["add", "a.txt"]);
        sh(&work, &["commit", "-qm", "init"]);
        sh(&work, &["push", "-q", "-u", "origin", "HEAD"]);

        let other = temp_dir("other");
        sh(&other, &["clone", "-q", remote.to_str().unwrap(), "."]);
        sh(&other, &["config", "user.email", "tester@example.com"]);
        sh(&other, &["config", "user.name", "tester"]);
        std::fs::write(other.join("b.txt"), "two\n").unwrap();
        sh(&other, &["add", "b.txt"]);
        sh(&other, &["commit", "-qm", "second"]);
        sh(&other, &["push", "-q"]);

        let repo = describe("demo", &work);
        let result = pull_repo(&repo, true).unwrap();
        assert!(result.updated, "expected fast-forward pull");
        assert_eq!(result.commits, 1);
        assert!(work.join("b.txt").exists());

        // 脏仓库在批量模式下应跳过，且不能改动工作区。
        std::fs::write(work.join("local.txt"), "draft\n").unwrap();
        let dirty = describe("demo", &work);
        let item = batch_item(&dirty);
        assert_eq!(item.outcome, "skipped");
        assert!(item.reason.unwrap().contains("未提交"));
        assert!(work.join("local.txt").exists());

        let _ = std::fs::remove_dir_all(&remote);
        let _ = std::fs::remove_dir_all(&work);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn log_and_diff_cover_selected_file() {
        let dir = init_repo("log");
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        let commits = log_entries(&dir, 5).unwrap();
        assert_eq!(commits[0].subject, "init");
        let diff = diff_at(&dir, Some("a.txt"), false).unwrap();
        assert!(diff.diff.contains("+two"), "{}", diff.diff);
    }
}
