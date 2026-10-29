//! 清理：六项门禁 + 后台任务 + 审计。
//!
//! 顺序很重要：先做只读检查给用户看，确认后取 `ops` 锁，再**重新**检查一次，才删。
//! 这样"审批期间/确认之后有人启动服务"不会变成删到运行中的目录。

use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Serialize;
use serde_json::json;

use crate::config::{self, Config, ProjectRoot, Target};
use crate::disk::{self, ArtifactKind, Evidence, ScanOptions};
use crate::jobs::{self, Job, JobState};
use crate::ops;
use crate::tmux::{self, Snapshot};

/// 清理上下文：一次构造，检查时只读。
pub struct Context {
    pub roots: Vec<ProjectRoot>,
    pub targets: Vec<Target>,
    pub snapshot: Snapshot,
    pub mounts: HashSet<PathBuf>,
    pub mounts_ok: bool,
    pub snapshot_ok: bool,
    pub extra_dirs: Vec<String>,
}

impl Context {
    pub fn load() -> Result<Self, String> {
        let path = std::env::var("DEVTRAY_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| config::default_config_path());
        let cfg = Config::load(&path)?;
        let roots = cfg.project_roots()?;
        let targets = cfg.targets()?;
        let extra_dirs = cfg.settings.clean_extra_dirs.clone();
        let snapshot = tmux::snapshot_result()?;
        let (mounts, mounts_ok) = match disk::read_mounts() {
            Ok(mounts) => (mounts, true),
            Err(_) => (HashSet::new(), false),
        };
        Ok(Self {
            roots,
            targets,
            snapshot,
            mounts,
            mounts_ok,
            snapshot_ok: true,
            extra_dirs,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    /// 绝对路径（实际删除用）
    pub path: String,
    /// 相对项目根（展示用）
    pub relative: String,
    pub name: String,
    pub kind: ArtifactKind,
    pub allocated: u64,
    pub build_file: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Failure {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Report {
    pub removed: Vec<String>,
    pub failed: Vec<Failure>,
    pub allocated: u64,
    pub warnings: Vec<String>,
}

/// 六项检查：根围栏、白名单、证据链、VCS、挂载边界、运行状态。
pub fn check(
    project: &ProjectRoot,
    path: &Path,
    allocated: u64,
    ctx: &Context,
) -> Result<Candidate, String> {
    if !project.cleaning_allowed {
        return Err(project
            .reason
            .clone()
            .unwrap_or_else(|| "项目根过宽，已禁用清理".into()));
    }
    if !ctx.snapshot_ok {
        return Err("无法确认服务状态（tmux 查询失败）".into());
    }
    if !ctx.mounts_ok {
        return Err("无法确认挂载边界（/proc/self/mountinfo 不可读）".into());
    }

    let root = project
        .root
        .canonicalize()
        .unwrap_or_else(|_| project.root.clone());
    let canon = safe_path(&root, path)?;
    let name = canon
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("目标没有目录名")?;
    let kind = ArtifactKind::from_name(&name, &ctx.extra_dirs)
        .ok_or_else(|| format!("目录名 {name:?} 不在允许清理的清单"))?;

    let evidence = disk::evidence_for(&root, &canon, kind, &ctx.mounts);
    reject_evidence(&canon, &evidence)?;
    if ctx.mounts.iter().any(|m| m == &canon) {
        return Err(format!("{} 本身是挂载点", canon.display()));
    }

    for target in &ctx.targets {
        let Some(info) = ctx.snapshot.get(&target.session) else {
            continue;
        };
        if info.dead {
            continue;
        }
        let project_root = ctx
            .roots
            .iter()
            .find(|r| r.project == target.project)
            .map(|r| r.root.as_path());
        let cwd = config::absolute(config::expand_home(&target.service.cwd));
        let overlaps = project_root
            .map(|r| roots_overlap(&canon, r))
            .unwrap_or(false)
            || cwd.starts_with(&canon)
            || canon.starts_with(&cwd);
        if overlaps {
            return Err(format!(
                "服务 {}/{} 正在运行（{}），请先停止",
                target.project, target.service.name, target.session
            ));
        }
    }

    Ok(Candidate {
        relative: canon
            .strip_prefix(&root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| canon.display().to_string()),
        path: canon.display().to_string(),
        name,
        kind,
        allocated,
        build_file: evidence.build_file.clone(),
    })
}

fn reject_evidence(path: &Path, evidence: &Evidence) -> Result<(), String> {
    if evidence.vcs_tracked {
        return Err(format!(
            "{} 内有被 git 跟踪的文件，禁止删除",
            path.display()
        ));
    }
    if let Some(mount) = &evidence.mount_point {
        return Err(format!("{} 内含挂载点 {mount}，禁止删除", path.display()));
    }
    if evidence.build_file.is_none() {
        let reason = if evidence.notes.is_empty() {
            "缺少构建工具归属证据".to_string()
        } else {
            evidence.notes.join("；")
        };
        return Err(format!("{} 不满足证据链：{reason}", path.display()));
    }
    Ok(())
}

/// 路径围栏：必须是根内真实目录，路径上任何一级都不能是符号链接。
fn safe_path(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("读不到 {}：{e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!("{} 是符号链接，拒绝删除", path.display()));
    }
    if !meta.is_dir() {
        return Err(format!("{} 不是目录", path.display()));
    }
    let canon = path
        .canonicalize()
        .map_err(|e| format!("解析 {} 失败：{e}", path.display()))?;
    if canon == root || !canon.starts_with(root) {
        return Err(format!(
            "{} 不在项目根 {} 之内（或就是根目录）",
            canon.display(),
            root.display()
        ));
    }
    let rel = canon
        .strip_prefix(root)
        .map_err(|_| "路径围栏计算失败".to_string())?;
    let mut current = root.to_path_buf();
    for comp in rel.components() {
        current.push(comp);
        if fs::symlink_metadata(&current)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(format!("路径 {} 含符号链接，拒绝删除", current.display()));
        }
    }
    Ok(canon)
}

fn roots_overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// 在已确认的候选上执行删除；每个失败都记录，不中断其他目录。
pub fn execute(
    candidates: &[Candidate],
    job: &mut Job,
    persist: bool,
    audit: bool,
) -> Result<Report, String> {
    let mut report = Report::default();
    let total = candidates.len().max(1);
    for (index, candidate) in candidates.iter().enumerate() {
        if job.state == JobState::Cancelled {
            break;
        }
        let path = Path::new(&candidate.path);
        let result = fs::remove_dir_all(path);
        match result {
            Ok(()) => {
                if path.exists() {
                    report.failed.push(Failure {
                        path: candidate.path.clone(),
                        error: "删除后目录仍存在".into(),
                    });
                } else {
                    report.removed.push(candidate.path.clone());
                    report.allocated += candidate.allocated;
                }
            }
            Err(e) => report.failed.push(Failure {
                path: candidate.path.clone(),
                error: e.to_string(),
            }),
        }
        job.progress = (index + 1) as f32 / total as f32;
        job.message = Some(format!("已处理 {}/{}", index + 1, total));
        if persist {
            job.save()?;
        }
    }
    if report.failed.is_empty() {
        job.finish_ok(json!({
            "removed": &report.removed,
            "allocated": report.allocated,
        }));
    } else {
        job.finish_ok(json!({
            "removed": &report.removed,
            "failed": &report.failed,
            "allocated": report.allocated,
        }));
    }
    if persist {
        job.save()?;
    }
    if audit && let Err(e) = write_audit(job, &report) {
        report.warnings.push(format!("审计写入失败：{e}"));
    }
    Ok(report)
}

fn write_audit(job: &Job, report: &Report) -> Result<(), String> {
    let result = if report.failed.is_empty() {
        "ok"
    } else {
        "partial"
    };
    append_audit(
        job,
        result,
        &report.removed,
        &report.failed,
        report.allocated,
    )
}

pub(crate) fn append_audit(
    job: &Job,
    result: &str,
    paths: &[String],
    failed: &[Failure],
    allocated: u64,
) -> Result<(), String> {
    let dir = config::state_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("创建状态目录失败：{e}"))?;
    let path = dir.join("audit.jsonl");
    if fs::metadata(&path)
        .map(|m| m.len() > 10 * 1024 * 1024)
        .unwrap_or(false)
    {
        let rotated = dir.join("audit.jsonl.1");
        let _ = fs::remove_file(&rotated);
        fs::rename(&path, &rotated).map_err(|e| format!("轮转审计日志失败：{e}"))?;
    }
    let entry = json!({
        "ts": jobs::now_secs(),
        "source": job.source,
        "action": job.action,
        "job_id": job.id,
        "project": job.project,
        "paths": paths,
        "failed": failed,
        "allocated": allocated,
        "result": result,
    });
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("打开审计日志失败：{e}"))?;
    writeln!(file, "{entry}").map_err(|e| format!("写审计日志失败：{e}"))
}

/// 启动一个后台 worker（`_clean` 或 `_approve`）；输出由 job 文件承载。
pub(crate) fn spawn_worker(command: &str, id: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg(command).arg(id);
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    crate::proc::spawn_detached(&mut cmd).map_err(|e| format!("启动后台任务失败：{e}"))
}

/// 扫描 + 选目标 + 六项检查。CLI、面板计划、面板启动共用这一条路径。
/// 扫描 + 选目标 + 六项检查。CLI、面板计划、面板启动共用这一条路径。
pub(crate) fn prepare_candidates(
    project: &ProjectRoot,
    ctx: &Context,
    kinds: &[String],
    paths: &[PathBuf],
    all: bool,
) -> Result<Vec<Candidate>, String> {
    if !paths.is_empty() && (all || !kinds.is_empty()) {
        return Err("--path 不能与 --kind / --all 同时使用".into());
    }
    let usage = disk::scan(
        project,
        &ScanOptions {
            extra_dirs: ctx.extra_dirs.clone(),
        },
    );
    if !usage.complete {
        return Err(format!(
            "扫描不完整：{} 个目录无法读取、{} 个挂载点/跨设备跳过；拒绝清理",
            usage.skipped,
            usage.mount_skipped.len()
        ));
    }

    if !paths.is_empty() {
        let mut candidates = Vec::new();
        for path in paths {
            let canon = path
                .canonicalize()
                .map_err(|e| format!("解析 {} 失败：{e}", path.display()))?;
            let rel = canon.strip_prefix(&project.root).ok();
            let hit = usage
                .artifacts
                .iter()
                .find(|a| rel.map(|r| r == Path::new(&a.path)).unwrap_or(false))
                .ok_or_else(|| {
                    format!(
                        "{} 不是项目 {} 的构建目录候选",
                        canon.display(),
                        project.project
                    )
                })?;
            if !hit.deletable {
                return Err(format!(
                    "{} 不满足清理条件：{}",
                    hit.path,
                    hit.evidence.notes.join("；")
                ));
            }
            candidates.push(check(project, &canon, hit.allocated, ctx)?);
        }
        return Ok(candidates);
    }

    let mut chosen: Vec<(PathBuf, u64)> = Vec::new();
    if all {
        for artifact in &usage.artifacts {
            if artifact.deletable {
                chosen.push((project.root.join(&artifact.path), artifact.allocated));
            }
        }
        if chosen.is_empty() {
            return Err("没有满足清理条件的目录".into());
        }
    } else {
        for kind in kinds {
            let hits: Vec<_> = usage.artifacts.iter().filter(|a| a.name == *kind).collect();
            if hits.is_empty() {
                return Err(format!("项目 {} 里没有找到目录 {kind}", project.project));
            }
            for hit in hits {
                if !hit.deletable {
                    return Err(format!(
                        "{} 不满足清理条件：{}",
                        hit.path,
                        hit.evidence.notes.join("；")
                    ));
                }
                chosen.push((project.root.join(&hit.path), hit.allocated));
            }
        }
    }

    let mut candidates = Vec::new();
    for (path, allocated) in &chosen {
        candidates.push(check(project, path, *allocated, ctx)?);
    }
    Ok(candidates)
}

// ── CLI ────────────────────────────────────────────────────────────────────

pub fn command(args: &[String]) -> std::process::ExitCode {
    match run(args) {
        Ok((text, failed)) => {
            let mut out = std::io::stdout();
            let _ = writeln!(out, "{text}");
            if failed {
                std::process::ExitCode::FAILURE
            } else {
                std::process::ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(String, bool), String> {
    let mut project = None;
    let mut kinds: Vec<String> = Vec::new();
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut all = false;
    let mut yes = false;
    let mut dry_run = false;
    let mut json = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--kind" => kinds.push(it.next().ok_or("--kind 缺少目录名")?.clone()),
            "--path" => paths.push(PathBuf::from(it.next().ok_or("--path 缺少目录")?)),
            "--all" => all = true,
            "--yes" | "-y" => yes = true,
            "--dry-run" => dry_run = true,
            "--json" => json = true,
            other if other.starts_with('-') => return Err(format!("未知选项：{other}")),
            other if project.is_none() => project = Some(other.to_string()),
            other => return Err(format!("多余的参数：{other}")),
        }
    }
    let project_name =
        project.ok_or("用法：devtray disk clean <项目> --kind node_modules [--yes]")?;
    if kinds.is_empty() && !all && paths.is_empty() {
        return Err("必须指定 --kind <目录名>、--path <目录> 或 --all；不会默认清空全部".into());
    }

    let ctx = Context::load()?;
    let project = ctx
        .roots
        .iter()
        .find(|r| r.project == project_name)
        .cloned()
        .ok_or_else(|| format!("找不到项目 {project_name:?}"))?;
    let candidates = prepare_candidates(&project, &ctx, &kinds, &paths, all)?;

    let total: u64 = candidates.iter().map(|c| c.allocated).sum();
    let summary = render_plan(&project, &candidates, total);
    if dry_run {
        if json {
            return Ok((
                serde_json::to_string_pretty(&candidates).map_err(|e| e.to_string())?,
                false,
            ));
        }
        return Ok((summary, false));
    }

    println!("{summary}");
    if !yes && !confirm(&project.project)? {
        return Err("已取消".into());
    }

    let _guard = ops::acquire(Duration::from_secs(5))?;
    // 确认期间可能有人启动服务 / 挂载新文件系统 / 改配置：拿锁后重新加载并检查一次
    let fresh = Context::load()?;
    let locked_project = fresh
        .roots
        .iter()
        .find(|r| r.project == project.project)
        .cloned()
        .ok_or_else(|| format!("项目 {} 在确认后消失了", project.project))?;
    for candidate in &candidates {
        check(
            &locked_project,
            Path::new(&candidate.path),
            candidate.allocated,
            &fresh,
        )?;
    }

    let paths: Vec<String> = candidates.iter().map(|c| c.path.clone()).collect();
    let mut job = Job::new("clean", Some(&project.project), &paths);
    job.source = "cli".into();
    job.save()?;
    job.mark_running(Some("清理中"));
    job.save()?;

    let report = execute(&candidates, &mut job, true, true)?;
    if !report.removed.is_empty() {
        disk::invalidate(&project.project);
    }
    let text = if json {
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
    } else {
        render_report(&report, total)
    };
    Ok((text, !report.failed.is_empty()))
}

fn confirm(project: &str) -> Result<bool, String> {
    print!("输入项目名 {project} 确认清理（其他任意输入取消）：");
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("读取确认失败：{e}"))?;
    Ok(line.trim() == project)
}

fn render_plan(project: &ProjectRoot, candidates: &[Candidate], total: u64) -> String {
    let mut out = format!(
        "将清理 {}（{}）：{} 个目录，分配 {}\n",
        project.project,
        if project.inferred {
            "推断根"
        } else {
            "显式根"
        },
        candidates.len(),
        disk::bytes_human(total)
    );
    for c in candidates {
        out.push_str(&format!(
            "  {:<14} {:<38} {:>9}  证据 {}\n",
            c.kind.label(),
            c.relative,
            disk::bytes_human(c.allocated),
            c.build_file.as_deref().unwrap_or("-")
        ));
    }
    out.trim_end().to_string()
}

fn render_report(report: &Report, planned: u64) -> String {
    let mut out = format!(
        "已删除 {} 个目录（计划分配 {}）；失败 {}\n",
        report.removed.len(),
        disk::bytes_human(planned),
        report.failed.len()
    );
    for path in &report.removed {
        out.push_str(&format!("  ✓ {path}\n"));
    }
    for failure in &report.failed {
        out.push_str(&format!("  ✕ {}：{}\n", failure.path, failure.error));
    }
    for warning in &report.warnings {
        out.push_str(&format!("  警告：{warning}\n"));
    }
    out.trim_end().to_string()
}

fn parse_selection(args: &[String]) -> Result<(String, Vec<String>, Vec<PathBuf>, bool), String> {
    let mut project = None;
    let mut kinds = Vec::new();
    let mut paths = Vec::new();
    let mut all = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--kind" => kinds.push(it.next().ok_or("--kind 缺少目录名")?.clone()),
            "--path" => paths.push(PathBuf::from(it.next().ok_or("--path 缺少目录")?)),
            "--all" => all = true,
            "--json" => {}
            other if other.starts_with('-') => return Err(format!("未知选项：{other}")),
            other if project.is_none() => project = Some(other.to_string()),
            other => return Err(format!("多余的参数：{other}")),
        }
    }
    let project = project.ok_or("缺少项目名")?;
    if kinds.is_empty() && paths.is_empty() && !all {
        return Err("必须指定 --kind、--path 或 --all".into());
    }
    Ok((project, kinds, paths, all))
}

fn emit(result: Result<String, String>) -> std::process::ExitCode {
    match result {
        Ok(text) => {
            println!("{text}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// 面板：只生成计划并做六项检查，不删除。
pub fn plan_command(args: &[String]) -> std::process::ExitCode {
    emit((|| {
        let (project_name, kinds, paths, all) = parse_selection(args)?;
        let ctx = Context::load()?;
        let project = ctx
            .roots
            .iter()
            .find(|r| r.project == project_name)
            .cloned()
            .ok_or_else(|| format!("找不到项目 {project_name:?}"))?;
        let candidates = prepare_candidates(&project, &ctx, &kinds, &paths, all)?;
        let total: u64 = candidates.iter().map(|c| c.allocated).sum();
        serde_json::to_string(&json!({
            "project": project.project,
            "root": project.root.display().to_string(),
            "candidates": candidates,
            "total": total,
        }))
        .map_err(|e| e.to_string())
    })())
}

/// 面板：检查通过后创建 job，并启动 `_clean` 子进程；立即返回 job_id。
pub fn start_command(args: &[String]) -> std::process::ExitCode {
    emit((|| {
        let (project_name, kinds, paths, all) = parse_selection(args)?;
        let ctx = Context::load()?;
        let project = ctx
            .roots
            .iter()
            .find(|r| r.project == project_name)
            .cloned()
            .ok_or_else(|| format!("找不到项目 {project_name:?}"))?;
        let candidates = prepare_candidates(&project, &ctx, &kinds, &paths, all)?;
        let job_paths: Vec<String> = candidates.iter().map(|c| c.path.clone()).collect();
        let total: u64 = candidates.iter().map(|c| c.allocated).sum();
        let mut job = Job::new("clean", Some(&project.project), &job_paths);
        job.source = "panel".into();
        job.save()?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut cmd = Command::new(exe);
        cmd.arg("_clean").arg(&job.id);
        // 子进程的输出由 job 文件承载；不要继承父进程 stdout，否则会污染 JSON 响应
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        crate::proc::spawn_detached(&mut cmd).map_err(|e| format!("启动清理任务失败：{e}"))?;
        serde_json::to_string(&json!({
            "job_id": job.id,
            "count": candidates.len(),
            "total": total,
        }))
        .map_err(|e| e.to_string())
    })())
}

fn settings_config() -> Result<Config, String> {
    let path = std::env::var("DEVTRAY_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| config::default_config_path());
    Config::load(&path)
}

fn deny(job: &mut Job, reason: &str) -> Result<(), String> {
    job.cancel();
    job.error = Some(reason.into());
    job.save()?;
    let _ = append_audit(job, "denied", &[], &[], 0);
    Ok(())
}

/// yad 审批：60 秒无响应按拒绝处理；yad 不可用也拒绝（fail closed）。
fn yad_confirm(job: &Job) -> bool {
    let project = job.project.as_deref().unwrap_or("?");
    let mut lines: Vec<String> = job
        .paths
        .iter()
        .take(20)
        .map(|p| format!("• {p}"))
        .collect();
    if job.paths.len() > 20 {
        lines.push(format!("… 另有 {} 个目录", job.paths.len() - 20));
    }
    let text = format!(
        "项目：{project}\n目录数：{}\n分配：{}\n\n{}\n\n删除不可撤销；执行前会再次检查运行中的服务、挂载点与 git 跟踪。\n来源：{}",
        job.paths.len(),
        disk::bytes_human(job.total),
        lines.join("\n"),
        job.source
    );
    let status = Command::new("yad")
        .args([
            "--title=devtray · 清理确认".to_string(),
            format!("--text={text}"),
            "--no-markup".to_string(),
            "--width=640".to_string(),
            "--center".to_string(),
            "--timeout=60".to_string(),
            "--button=允许:0".to_string(),
            "--button=拒绝:1".to_string(),
        ])
        .status();
    matches!(status, Ok(s) if s.success())
}

/// `mcp_writes=ask` 时的审批进程：弹窗、批准后启动 `_clean`。由 MCP 工具后台拉起。
pub fn approve_command(id: Option<&String>) -> std::process::ExitCode {
    let result = (|| -> Result<String, String> {
        let id = id.ok_or("用法：devtray _approve <job-id>")?;
        let mut job = Job::load(id)?;
        if job.state != JobState::PendingApproval {
            return Err(format!("任务 {} 不是待审批状态", job.id));
        }
        let mode = settings_config()?.settings.mcp_writes;
        match mode.as_str() {
            "deny" => {
                deny(&mut job, "mcp_writes = deny")?;
                Ok("已拒绝：MCP 写操作被配置禁用".into())
            }
            "allow" => {
                job.mark_running(Some("mcp_writes = allow 自动执行"));
                job.save()?;
                spawn_worker("_clean", &job.id)?;
                Ok("已按 allow 直接执行".into())
            }
            _ => {
                if !crate::terminal::which("yad") {
                    deny(&mut job, "yad 不可用，已拒绝（fail closed）")?;
                    return Ok("已拒绝：yad 不可用".into());
                }
                if yad_confirm(&job) {
                    job.mark_running(Some("用户已批准"));
                    job.save()?;
                    spawn_worker("_clean", &job.id)?;
                    Ok("已批准".into())
                } else {
                    deny(&mut job, "用户拒绝或审批超时（60s）")?;
                    Ok("已拒绝".into())
                }
            }
        }
    })();
    emit(result)
}

/// 面板 / MCP 的后台清理入口：按 job 里记录的路径重新检查并执行。
pub fn job_command(id: Option<&String>) -> std::process::ExitCode {
    let result = (|| -> Result<String, String> {
        let id = id.ok_or("用法：devtray _clean <job-id>")?;
        let mut job = Job::load(id)?;
        if job.state != JobState::PendingApproval && job.state != JobState::Running {
            return Err(format!("任务 {} 已是 {:?}，不重复执行", job.id, job.state));
        }
        let ctx = Context::load()?;
        let project_name = job.project.clone().ok_or("任务缺少项目")?;
        let project = ctx
            .roots
            .iter()
            .find(|r| r.project == project_name)
            .cloned()
            .ok_or_else(|| format!("找不到项目 {project_name:?}"))?;

        let usage = disk::scan(
            &project,
            &ScanOptions {
                extra_dirs: ctx.extra_dirs.clone(),
            },
        );
        if !usage.complete {
            return Err("扫描不完整（有目录无法读取或存在挂载点），拒绝清理".into());
        }
        let mut candidates = Vec::new();
        for path in &job.paths {
            let relative = Path::new(path).strip_prefix(&project.root).ok();
            let allocated = usage
                .artifacts
                .iter()
                .find(|a| relative.map(|r| r == Path::new(&a.path)).unwrap_or(false))
                .map(|a| a.allocated)
                .unwrap_or(0);
            candidates.push(check(&project, Path::new(path), allocated, &ctx)?);
        }

        let _guard = ops::acquire(Duration::from_secs(5))?;
        let fresh = Context::load()?;
        let locked_project = fresh
            .roots
            .iter()
            .find(|r| r.project == project.project)
            .cloned()
            .ok_or_else(|| format!("项目 {} 在确认后消失了", project.project))?;
        for candidate in &candidates {
            check(
                &locked_project,
                Path::new(&candidate.path),
                candidate.allocated,
                &fresh,
            )?;
        }

        job.mark_running(Some("清理中"));
        job.save()?;
        let report = execute(&candidates, &mut job, true, true)?;
        if !report.removed.is_empty() {
            disk::invalidate(&project.project);
        }
        Ok(render_report(&report, report.allocated))
    })();
    match result {
        Ok(text) => {
            println!("{text}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            if let Some(id) = id
                && let Ok(mut job) = Job::load(id)
            {
                job.finish_error(e.clone());
                let _ = job.save();
            }
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectRoot;
    use crate::tmux::{PaneInfo, Snapshot};

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "devtray-clean-{tag}-{}-{}",
                std::process::id(),
                jobs::now_secs()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn dir(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            fs::create_dir_all(&p).unwrap();
            p
        }

        fn file(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&p, b"x").unwrap();
            p
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn project(root: &Path, allowed: bool) -> ProjectRoot {
        ProjectRoot {
            project: "p".into(),
            root: fs::canonicalize(root).unwrap(),
            inferred: false,
            cleaning_allowed: allowed,
            reason: if allowed {
                None
            } else {
                Some("测试：根过宽".into())
            },
            notes: Vec::new(),
        }
    }

    fn ctx(root: &Path) -> Context {
        Context {
            roots: vec![project(root, true)],
            targets: Vec::new(),
            snapshot: Snapshot::new(),
            mounts: HashSet::new(),
            mounts_ok: true,
            snapshot_ok: true,
            extra_dirs: Vec::new(),
        }
    }

    fn node_dir(t: &Tmp) -> PathBuf {
        t.file("app/package.json");
        t.file("app/bun.lock");
        t.dir("app/node_modules/pkg");
        t.0.join("app/node_modules")
    }

    #[test]
    fn check_passes_for_evidenced_node_modules() {
        let t = Tmp::new("ok");
        let dir = node_dir(&t);
        let c = check(&project(&t.0, true), &dir, 123, &ctx(&t.0)).unwrap();
        assert_eq!(c.kind, ArtifactKind::NodeModules);
        assert_eq!(c.allocated, 123);
    }

    #[test]
    fn check_rejects_root_too_broad() {
        let t = Tmp::new("broad");
        let dir = node_dir(&t);
        let err = check(&project(&t.0, false), &dir, 0, &ctx(&t.0)).unwrap_err();
        assert!(err.contains("根过宽"));
    }

    #[test]
    fn check_rejects_symlink_target() {
        let t = Tmp::new("link");
        let outside = t.dir("outside");
        fs::create_dir_all(t.0.join("app")).unwrap();
        std::os::unix::fs::symlink(&outside, t.0.join("app/node_modules")).unwrap();
        let err = check(
            &project(&t.0, true),
            &t.0.join("app/node_modules"),
            0,
            &ctx(&t.0),
        )
        .unwrap_err();
        assert!(err.contains("符号链接"));
    }

    #[test]
    fn check_rejects_path_outside_root() {
        let t = Tmp::new("outside");
        t.dir("app");
        let other = t.dir("other");
        let err = check(&project(&t.0.join("app"), true), &other, 0, &ctx(&t.0)).unwrap_err();
        assert!(err.contains("不在项目根"));
    }

    #[test]
    fn check_rejects_missing_evidence() {
        let t = Tmp::new("noev");
        let dir = t.dir("app/node_modules");
        let err = check(&project(&t.0, true), &dir, 0, &ctx(&t.0)).unwrap_err();
        assert!(err.contains("证据"), "{err}");
    }

    #[test]
    fn check_rejects_dist_by_default() {
        let t = Tmp::new("dist");
        t.file("app/package.json");
        let dist = t.dir("app/dist");
        let err = check(&project(&t.0, true), &dist, 0, &ctx(&t.0)).unwrap_err();
        assert!(err.contains("不在允许清理"));
    }

    #[test]
    fn check_blocks_running_service_in_overlapping_project() {
        let t = Tmp::new("running");
        let dir = node_dir(&t);
        let mut context = ctx(&t.0);
        context.targets.push(Target {
            project: "p".into(),
            service: crate::config::Service {
                name: "frontend".into(),
                cwd: t.0.join("app").display().to_string(),
                cmd: "bun run dev".into(),
                autostart: false,
                port: None,
                ready: None,
                url: None,
                env: Default::default(),
                log_file: None,
            },
            session: "devtray-p-frontend".into(),
            log_path: t.0.join("x.log"),
            ready_re: None,
        });
        context.snapshot.insert(
            "devtray-p-frontend".into(),
            PaneInfo {
                dead: false,
                status: None,
                pid: 1,
            },
        );
        let err = check(&project(&t.0, true), &dir, 0, &context).unwrap_err();
        assert!(err.contains("正在运行"), "{err}");
    }

    #[test]
    fn check_rejects_unreadable_mountinfo() {
        let t = Tmp::new("mountinfo");
        let dir = node_dir(&t);
        let mut context = ctx(&t.0);
        context.mounts_ok = false;
        let err = check(&project(&t.0, true), &dir, 0, &context).unwrap_err();
        assert!(err.contains("挂载边界"));
    }

    #[test]
    fn check_rejects_mount_inside_candidate() {
        let t = Tmp::new("mount");
        let dir = node_dir(&t);
        let mut context = ctx(&t.0);
        context.mounts.insert(dir.join("sub"));
        let err = check(&project(&t.0, true), &dir, 0, &context).unwrap_err();
        assert!(err.contains("挂载点"), "{err}");
    }

    #[test]
    fn execute_removes_and_reports() {
        let t = Tmp::new("exec");
        let dir = node_dir(&t);
        let candidate = check(&project(&t.0, true), &dir, 10, &ctx(&t.0)).unwrap();
        let mut job = Job::new("clean", Some("p"), std::slice::from_ref(&candidate.path));
        let report = execute(&[candidate], &mut job, false, false).unwrap();
        assert_eq!(report.removed.len(), 1);
        assert!(!dir.exists());
        assert_eq!(report.allocated, 10);
        assert_eq!(job.state, JobState::Done);
    }
}
