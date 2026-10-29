//! 只读磁盘扫描：项目根下的构建目录归类、分配/逻辑尺寸、证据链与完整性。
//!
//! 这一步**只读**：不删除、不改配置。清理所需的门禁在 `clean` 里另做，
//! 但证据与完整性在这里先算出来，避免"先展示、后补证据"造成口径漂移。

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::{self, Config, ProjectRoot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    NodeModules,
    CargoTarget,
    PythonVenv,
    PyCache,
    PytestCache,
    MypyCache,
    RuffCache,
    Next,
    Nuxt,
    Turbo,
    ParcelCache,
    SvelteKit,
    Extra,
}

impl ArtifactKind {
    pub(crate) fn from_name(name: &str, extra: &[String]) -> Option<Self> {
        let kind = match name {
            "node_modules" => Self::NodeModules,
            "target" => Self::CargoTarget,
            ".venv" | "venv" => Self::PythonVenv,
            "__pycache__" => Self::PyCache,
            ".pytest_cache" => Self::PytestCache,
            ".mypy_cache" => Self::MypyCache,
            ".ruff_cache" => Self::RuffCache,
            ".next" => Self::Next,
            ".nuxt" => Self::Nuxt,
            ".turbo" => Self::Turbo,
            ".parcel-cache" => Self::ParcelCache,
            ".svelte-kit" => Self::SvelteKit,
            other if extra.iter().any(|e| e == other) => Self::Extra,
            _ => return None,
        };
        Some(kind)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NodeModules => "node_modules",
            Self::CargoTarget => "cargo target",
            Self::PythonVenv => "python venv",
            Self::PyCache => "__pycache__",
            Self::PytestCache => ".pytest_cache",
            Self::MypyCache => ".mypy_cache",
            Self::RuffCache => ".ruff_cache",
            Self::Next => ".next",
            Self::Nuxt => ".nuxt",
            Self::Turbo => ".turbo",
            Self::ParcelCache => ".parcel-cache",
            Self::SvelteKit => ".svelte-kit",
            Self::Extra => "extra",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    /// 构建归属文件（相对项目根）；None = 没有找到证据
    pub build_file: Option<String>,
    pub vcs_tracked: bool,
    pub pyvenv_cfg: bool,
    /// 候选内出现的挂载点（含 bind mount）
    pub mount_point: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub kind: ArtifactKind,
    /// 目录名（`node_modules` / `target` / 自定义）
    pub name: String,
    /// 相对项目根的路径
    pub path: String,
    pub allocated: u64,
    pub logical: u64,
    pub files: u64,
    pub deletable: bool,
    pub evidence: Evidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopEntry {
    pub path: String,
    pub allocated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectUsage {
    pub project: String,
    pub root: String,
    pub inferred: bool,
    pub cleaning_allowed: bool,
    pub cleaning_reason: Option<String>,
    pub notes: Vec<String>,
    pub artifacts: Vec<Artifact>,
    pub other_top: Vec<TopEntry>,
    pub artifacts_allocated: u64,
    pub unclassified_allocated: u64,
    pub total_allocated: u64,
    pub total_logical: u64,
    pub deletable_allocated: u64,
    pub scanned_at: u64,
    pub duration_ms: u64,
    pub skipped: u64,
    pub mount_skipped: Vec<String>,
    pub complete: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct ScanOptions {
    /// 用户额外允许扫描/清理的目录名（不改变证据要求）
    pub extra_dirs: Vec<String>,
}

struct Walk<'a> {
    root: &'a Path,
    extra: &'a [String],
    root_dev: u64,
    seen: HashSet<(u64, u64)>,
    mounts: HashSet<PathBuf>,
    mounts_ok: bool,
    artifacts: Vec<Artifact>,
    unclassified: u64,
    unclassified_logical: u64,
    top: BTreeMap<String, u64>,
    skipped: u64,
    mount_skipped: Vec<PathBuf>,
    warnings: Vec<String>,
}

pub fn scan(project: &ProjectRoot, opts: &ScanOptions) -> ProjectUsage {
    let started = Instant::now();
    let mount_result = read_mounts();
    let (mounts, mounts_ok) = match &mount_result {
        Ok(m) => (m.clone(), true),
        Err(e) => {
            let mut set = HashSet::new();
            set.insert(PathBuf::from("<mountinfo 不可读>"));
            let _ = e;
            (set, false)
        }
    };
    let root_dev = fs::symlink_metadata(&project.root)
        .map(|m| m.dev())
        .unwrap_or(0);
    let mut walk = Walk {
        root: &project.root,
        extra: &opts.extra_dirs,
        root_dev,
        seen: HashSet::new(),
        mounts,
        mounts_ok,
        artifacts: Vec::new(),
        unclassified: 0,
        unclassified_logical: 0,
        top: BTreeMap::new(),
        skipped: 0,
        mount_skipped: Vec::new(),
        warnings: Vec::new(),
    };
    if !mounts_ok {
        walk.warnings
            .push("读不到 /proc/self/mountinfo：清理将被禁用".into());
    }
    walk.walk(&project.root);
    walk.artifacts
        .sort_by_key(|a| std::cmp::Reverse(a.allocated));

    let artifacts_allocated: u64 = walk.artifacts.iter().map(|a| a.allocated).sum();
    let artifacts_logical: u64 = walk.artifacts.iter().map(|a| a.logical).sum();
    let deletable_allocated = walk
        .artifacts
        .iter()
        .filter(|a| a.deletable)
        .map(|a| a.allocated)
        .sum();
    let complete = walk.skipped == 0 && walk.mount_skipped.is_empty();
    let mut cleaning_allowed = project.cleaning_allowed && complete && mounts_ok;
    let mut cleaning_reason = project.reason.clone();
    if cleaning_allowed {
        cleaning_reason = None;
    } else if cleaning_reason.is_none() {
        cleaning_reason = Some(if !mounts_ok {
            "无法确认挂载边界".into()
        } else if !walk.mount_skipped.is_empty() {
            "项目根下存在挂载点，清理已禁用".into()
        } else {
            "扫描不完整（有目录无法读取），清理已禁用".into()
        });
    }
    if !complete && cleaning_allowed {
        cleaning_allowed = false;
    }

    let mut other_top: Vec<TopEntry> = walk
        .top
        .into_iter()
        .map(|(path, allocated)| TopEntry { path, allocated })
        .collect();
    other_top.sort_by_key(|t| std::cmp::Reverse(t.allocated));
    other_top.truncate(5);

    ProjectUsage {
        project: project.project.clone(),
        root: project.root.display().to_string(),
        inferred: project.inferred,
        cleaning_allowed,
        cleaning_reason,
        notes: project.notes.clone(),
        artifacts: walk.artifacts,
        other_top,
        artifacts_allocated,
        unclassified_allocated: walk.unclassified,
        total_allocated: artifacts_allocated + walk.unclassified,
        total_logical: artifacts_logical + walk.unclassified_logical,
        deletable_allocated,
        scanned_at: now_secs(),
        duration_ms: started.elapsed().as_millis() as u64,
        skipped: walk.skipped,
        mount_skipped: walk
            .mount_skipped
            .iter()
            .map(|p| display_rel(&project.root, p))
            .collect(),
        complete,
        warnings: walk.warnings,
    }
}

impl Walk<'_> {
    fn walk(&mut self, dir: &Path) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                self.skipped += 1;
                self.warn(format!("无法读取 {}：{e}", dir.display()));
                return;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                self.skipped += 1;
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if matches!(name.as_str(), ".git" | ".hg" | ".svn") {
                    continue;
                }
                if let Some(kind) = ArtifactKind::from_name(&name, self.extra) {
                    self.record_artifact(&path, name, kind);
                    continue;
                }
                if self.mounts.contains(&path) {
                    self.mount_skipped.push(path.clone());
                    self.warn(format!("跳过挂载点 {}", path.display()));
                    continue;
                }
                let dev = fs::symlink_metadata(&path).map(|m| m.dev()).unwrap_or(0);
                if dev != self.root_dev {
                    self.mount_skipped.push(path.clone());
                    self.warn(format!("跳过跨文件系统目录 {}", path.display()));
                    continue;
                }
                self.walk(&path);
            } else if ft.is_file() {
                let Ok(md) = fs::symlink_metadata(&path) else {
                    self.skipped += 1;
                    continue;
                };
                if !self.seen.insert((md.dev(), md.ino())) {
                    continue;
                }
                let allocated = allocated_bytes(&md);
                self.unclassified += allocated;
                self.unclassified_logical += md.size();
                if let Ok(rel) = path.strip_prefix(self.root)
                    && rel.components().count() > 1
                    && let Some(first) = rel.components().next()
                {
                    *self
                        .top
                        .entry(first.as_os_str().to_string_lossy().to_string())
                        .or_default() += allocated;
                }
            }
        }
    }

    fn record_artifact(&mut self, path: &Path, name: String, kind: ArtifactKind) {
        let mut measure = Measure::default();
        self.measure(path, &mut measure);
        for mount in &measure.mounts {
            self.mount_skipped.push(mount.clone());
        }
        let evidence = self.evidence(path, kind);
        let deletable = evidence.build_file.is_some()
            && !evidence.vcs_tracked
            && evidence.mount_point.is_none()
            && measure.mounts.is_empty()
            && self.mounts_ok;
        self.artifacts.push(Artifact {
            kind,
            path: display_rel(self.root, path),
            name,
            allocated: measure.allocated,
            logical: measure.logical,
            files: measure.files,
            deletable,
            evidence,
        });
    }

    fn measure(&mut self, dir: &Path, out: &mut Measure) {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            if self.mounts.contains(&current) {
                out.mounts.push(current);
                continue;
            }
            let entries = match fs::read_dir(&current) {
                Ok(e) => e,
                Err(e) => {
                    self.skipped += 1;
                    self.warn(format!("无法读取 {}：{e}", current.display()));
                    continue;
                }
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(ft) = entry.file_type() else {
                    self.skipped += 1;
                    continue;
                };
                if ft.is_symlink() {
                    continue;
                }
                if ft.is_dir() {
                    if self.mounts.contains(&path) {
                        out.mounts.push(path);
                        continue;
                    }
                    let dev = fs::symlink_metadata(&path).map(|m| m.dev()).unwrap_or(0);
                    if dev != self.root_dev {
                        out.mounts.push(path);
                        continue;
                    }
                    stack.push(path);
                } else if ft.is_file() {
                    let Ok(md) = fs::symlink_metadata(&path) else {
                        self.skipped += 1;
                        continue;
                    };
                    if !self.seen.insert((md.dev(), md.ino())) {
                        continue;
                    }
                    out.allocated += allocated_bytes(&md);
                    out.logical += md.size();
                    out.files += 1;
                }
            }
        }
    }

    fn evidence(&self, path: &Path, kind: ArtifactKind) -> Evidence {
        evidence_for(self.root, path, kind, &self.mounts)
    }

    fn warn(&mut self, message: String) {
        if self.warnings.len() < 20 {
            self.warnings.push(message);
        }
    }
}

/// 只验证"这个目录有没有资格被删"，不统计尺寸。清理前的复检也用它。
pub(crate) fn evidence_for(
    root: &Path,
    path: &Path,
    kind: ArtifactKind,
    mounts: &HashSet<PathBuf>,
) -> Evidence {
    let mut notes = Vec::new();
    let mut evidence = Evidence {
        build_file: None,
        vcs_tracked: vcs_tracked(root, path),
        pyvenv_cfg: path.join("pyvenv.cfg").is_file(),
        mount_point: None,
        notes: Vec::new(),
    };
    let parent = path.parent().unwrap_or(path);
    let manifest = |name: &str| -> Option<String> {
        find_upwards(path, root, name).map(|p| display_rel(root, &p))
    };
    match kind {
        ArtifactKind::NodeModules => {
            evidence.build_file = manifest("package.json");
            if evidence.build_file.is_some()
                && !has_any(
                    parent,
                    &[
                        "bun.lock",
                        "bun.lockb",
                        "pnpm-lock.yaml",
                        "yarn.lock",
                        "package-lock.json",
                    ],
                )
            {
                notes.push("缺少 lockfile，无法证明是依赖目录".into());
                evidence.build_file = None;
            }
        }
        ArtifactKind::CargoTarget => {
            evidence.build_file = manifest("Cargo.toml");
            if evidence.build_file.is_some() && !target_marker(path) {
                notes.push("没有 CACHEDIR.TAG / debug / release 等构建痕迹".into());
                evidence.build_file = None;
            }
        }
        ArtifactKind::PythonVenv => {
            if !evidence.pyvenv_cfg {
                notes.push("缺少 pyvenv.cfg，不是可重建的 venv".into());
            }
            let has_rebuild = has_any(parent, &["uv.lock", "requirements.txt"]);
            if !has_rebuild {
                notes
                    .push("缺少 uv.lock / requirements.txt：只有 pyproject.toml 时拒绝删除".into());
            }
            if evidence.pyvenv_cfg && has_rebuild {
                evidence.build_file = Some(display_rel(
                    root,
                    &parent.join(if parent.join("uv.lock").is_file() {
                        "uv.lock"
                    } else {
                        "requirements.txt"
                    }),
                ));
            }
        }
        ArtifactKind::PyCache => {
            if !contains_extension(path, "pyc") {
                notes.push("目录里没有 .pyc".into());
            } else {
                evidence.build_file = Some(display_rel(root, path));
            }
        }
        ArtifactKind::PytestCache | ArtifactKind::MypyCache | ArtifactKind::RuffCache => {
            // 工具缓存目录：有内容就说明是该工具写的，不强制要求 package.json
            if dir_has_entries(path) {
                evidence.build_file = Some(display_rel(root, path));
            } else {
                notes.push("缓存目录为空".into());
            }
        }
        ArtifactKind::Next
        | ArtifactKind::Nuxt
        | ArtifactKind::Turbo
        | ArtifactKind::ParcelCache
        | ArtifactKind::SvelteKit => {
            evidence.build_file = manifest("package.json");
        }
        ArtifactKind::Extra => {
            evidence.build_file = manifest("package.json").or_else(|| manifest("Cargo.toml"));
        }
    }
    if evidence.vcs_tracked {
        notes.push("目录内有被 git 跟踪的文件，禁止删除".into());
    }
    evidence.mount_point = mounts
        .iter()
        .find(|m| m.starts_with(path))
        .map(|m| m.display().to_string());
    evidence.notes = notes;
    evidence
}

#[derive(Default)]
struct Measure {
    allocated: u64,
    logical: u64,
    files: u64,
    mounts: Vec<PathBuf>,
}

fn allocated_bytes(md: &fs::Metadata) -> u64 {
    md.blocks().saturating_mul(512)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn display_rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn has_any(dir: &Path, names: &[&str]) -> bool {
    names.iter().any(|n| dir.join(n).is_file())
}

fn dir_has_entries(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

fn contains_extension(dir: &Path, ext: &str) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    let mut seen = 0usize;
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 10_000 {
                return false;
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(entry.path());
            } else if entry.path().extension().map(|e| e == ext).unwrap_or(false) {
                return true;
            }
        }
    }
    false
}

fn target_marker(dir: &Path) -> bool {
    if dir.join("CACHEDIR.TAG").is_file()
        || dir.join("debug").is_dir()
        || dir.join("release").is_dir()
    {
        return true;
    }
    fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    && (name.starts_with("x86_64")
                        || name.starts_with("aarch64")
                        || name.starts_with("i686"))
            })
        })
        .unwrap_or(false)
}

fn find_upwards(path: &Path, root: &Path, name: &str) -> Option<PathBuf> {
    let mut current = path.parent();
    while let Some(dir) = current {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if dir == root {
            break;
        }
        current = dir.parent();
    }
    None
}

fn vcs_tracked(root: &Path, path: &Path) -> bool {
    if !root.join(".git").exists() {
        return false;
    }
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--"])
        .arg(rel)
        .output();
    matches!(out, Ok(o) if o.status.success() && !o.stdout.is_empty())
}

pub(crate) fn read_mounts() -> Result<HashSet<PathBuf>, String> {
    let text = fs::read_to_string("/proc/self/mountinfo")
        .map_err(|e| format!("读 /proc/self/mountinfo 失败：{e}"))?;
    Ok(parse_mountinfo(&text))
}

fn parse_mountinfo(text: &str) -> HashSet<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let _id = fields.next()?;
            let _parent = fields.next()?;
            let _device = fields.next()?;
            let _root = fields.next()?;
            let mount_point = fields.next()?;
            Some(PathBuf::from(unescape_mount_field(mount_point)))
        })
        .collect()
}

/// mountinfo 里的空格/制表/换行/反斜杠用八进制转义
fn unescape_mount_field(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let oct = &field[i + 1..i + 4];
            if oct.bytes().all(|b| (b'0'..=b'7').contains(&b))
                && let Ok(value) = u8::from_str_radix(oct, 8)
            {
                out.push(value);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn cache_dir() -> PathBuf {
    config::state_dir().join("cache").join("disk")
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "project".into()
    } else {
        cleaned
    }
}

fn cache_file(project: &str) -> PathBuf {
    cache_dir().join(format!("{}.json", safe_name(project)))
}

/// 缓存命中直接返回；TTL 内不重复扫。`ttl = 0` 表示不缓存。
pub fn scan_cached(
    project: &ProjectRoot,
    opts: &ScanOptions,
    refresh: bool,
    ttl: u64,
) -> ProjectUsage {
    if !refresh
        && ttl > 0
        && let Some(cached) = read_cached(&project.project)
        && cached.root == project.root.display().to_string()
        && now_secs().saturating_sub(cached.scanned_at) <= ttl
    {
        return cached;
    }
    let usage = scan(project, opts);
    let _ = write_cached(&usage);
    usage
}

pub fn read_cached(project: &str) -> Option<ProjectUsage> {
    let text = fs::read_to_string(cache_file(project)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn cached_all() -> Vec<ProjectUsage> {
    let Ok(entries) = fs::read_dir(cache_dir()) else {
        return Vec::new();
    };
    let mut out: Vec<ProjectUsage> = entries
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect();
    out.sort_by_key(|u| std::cmp::Reverse(u.total_allocated));
    out
}

fn write_cached(usage: &ProjectUsage) -> Result<(), String> {
    let dir = cache_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("创建扫描缓存目录失败：{e}"))?;
    let path = cache_file(&usage.project);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string(usage).map_err(|e| e.to_string())?;
    fs::write(&tmp, text).map_err(|e| format!("写扫描缓存失败：{e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("提交扫描缓存失败：{e}"))
}

pub fn invalidate(project: &str) {
    let _ = fs::remove_file(cache_file(project));
}

pub fn bytes_human(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn config_roots() -> Result<(Config, Vec<ProjectRoot>), String> {
    let path = std::env::var("DEVTRAY_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| config::default_config_path());
    let cfg = Config::load(&path)?;
    let roots = cfg.project_roots()?;
    Ok((cfg, roots))
}

fn select<'a>(
    roots: &'a [ProjectRoot],
    name: Option<&str>,
) -> Result<Vec<&'a ProjectRoot>, String> {
    match name {
        None => Ok(roots.iter().collect()),
        Some(name) => roots
            .iter()
            .find(|r| r.project == name)
            .map(|r| vec![r])
            .ok_or_else(|| {
                format!(
                    "找不到项目 {name:?}；可用：{}",
                    roots
                        .iter()
                        .map(|r| r.project.as_str())
                        .collect::<Vec<_>>()
                        .join("、")
                )
            }),
    }
}

pub fn command(args: &[String]) -> ExitCode {
    if args.first().map(String::as_str) == Some("clean") {
        return crate::clean::command(&args[1..]);
    }
    match run(args) {
        Ok(value) => {
            let mut out = std::io::stdout();
            let _ = writeln!(out, "{value}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<String, String> {
    if args.first().map(String::as_str) == Some("open") {
        return open_dir(
            args.get(1)
                .map(String::as_str)
                .ok_or("用法：devtray disk open <项目|项目/服务>")?,
        );
    }
    let mut json = false;
    let mut refresh = false;
    let mut project = None;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "--refresh" => refresh = true,
            other if other.starts_with('-') => return Err(format!("未知选项：{other}")),
            other if project.is_none() => project = Some(other.to_string()),
            other => return Err(format!("多余的参数：{other}")),
        }
    }
    let (cfg, roots) = config_roots()?;
    let selected = select(&roots, project.as_deref())?;
    let opts = ScanOptions {
        extra_dirs: cfg.settings.clean_extra_dirs.clone(),
    };
    let ttl = cfg.settings.disk_cache_ttl_s;
    let usages: Vec<ProjectUsage> = selected
        .iter()
        .map(|p| scan_cached(p, &opts, refresh, ttl))
        .collect();
    if json {
        return serde_json::to_string_pretty(&usages).map_err(|e| e.to_string());
    }
    Ok(render(&usages))
}

pub fn scan_only(args: &[String]) -> ExitCode {
    let result = (|| {
        let mut name = None;
        let mut refresh = false;
        for arg in args {
            match arg.as_str() {
                "--refresh" => refresh = true,
                other if other.starts_with('-') => return Err(format!("未知选项：{other}")),
                other if name.is_none() => name = Some(other.to_string()),
                other => return Err(format!("多余的参数：{other}")),
            }
        }
        let name = name.ok_or("用法：devtray _disk-scan <项目> [--refresh]")?;
        let (cfg, roots) = config_roots()?;
        let selected = select(&roots, Some(&name))?;
        let usage = scan_cached(
            selected[0],
            &ScanOptions {
                extra_dirs: cfg.settings.clean_extra_dirs.clone(),
            },
            refresh,
            cfg.settings.disk_cache_ttl_s,
        );
        serde_json::to_string(&usage).map_err(|e| e.to_string())
    })();
    match result {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// 只读缓存（资源页首屏 / 总览摘要用），不触发扫描。
pub fn cache_command() -> ExitCode {
    match serde_json::to_string_pretty(&cached_all()) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn render(usages: &[ProjectUsage]) -> String {
    let mut out = String::new();
    for u in usages {
        out.push_str(&format!(
            "项目 {}  根 {}（{}）\n",
            u.project,
            u.root,
            if u.inferred { "推断" } else { "显式" }
        ));
        if let Some(reason) = &u.cleaning_reason {
            out.push_str(&format!("  清理禁用：{reason}\n"));
        }
        out.push_str(&format!(
            "  分配 {} / 逻辑 {}；可清理候选 {}\n",
            bytes_human(u.total_allocated),
            bytes_human(u.total_logical),
            bytes_human(u.deletable_allocated)
        ));
        if u.artifacts.is_empty() {
            out.push_str("  （没有发现已知构建目录）\n");
        }
        for a in &u.artifacts {
            out.push_str(&format!(
                "  {:<16} {:<40} {:>9}  {:>5} 个文件  {}  {}",
                a.kind.label(),
                a.path,
                bytes_human(a.allocated),
                a.files,
                if a.deletable { "可清理" } else { "只读" },
                a.evidence
                    .build_file
                    .as_ref()
                    .map(|f| format!("证据：{f}"))
                    .unwrap_or_else(|| "证据：无".into())
            ));
            out.push('\n');
            for note in &a.evidence.notes {
                out.push_str(&format!("      注意：{note}\n"));
            }
        }
        if !u.other_top.is_empty() {
            out.push_str("  其他占用（未归类，前 5）：\n");
            for t in &u.other_top {
                out.push_str(&format!(
                    "    {:<40} {:>9}\n",
                    t.path,
                    bytes_human(t.allocated)
                ));
            }
        }
        if !u.complete {
            out.push_str(&format!(
                "  扫描不完整：跳过 {} 项，挂载/跨设备 {} 项\n",
                u.skipped,
                u.mount_skipped.len()
            ));
        }
        for warning in &u.warnings {
            out.push_str(&format!("  警告：{warning}\n"));
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn open_dir(spec: &str) -> Result<String, String> {
    let (cfg, roots) = config_roots()?;
    let (project_name, service_name) = match spec.split_once('/') {
        Some((p, s)) => (p, Some(s)),
        None => (spec, None),
    };
    let dir = match service_name {
        Some(service_name) => {
            let project = cfg
                .projects
                .iter()
                .find(|p| p.name == project_name)
                .ok_or_else(|| format!("找不到项目 {project_name:?}"))?;
            let service = project
                .services
                .iter()
                .find(|s| s.name == service_name)
                .ok_or_else(|| format!("找不到服务 {service_name:?}"))?;
            config::absolute(config::expand_home(&service.cwd))
        }
        None => roots
            .iter()
            .find(|r| r.project == project_name)
            .map(|r| r.root.clone())
            .ok_or_else(|| format!("找不到项目 {project_name:?}"))?,
    };
    open_path(&dir)
}

/// 用默认文件管理器打开任意已存在的目录；`disk open` 与 `skills open` 共用。
pub(crate) fn open_path(dir: &Path) -> Result<String, String> {
    if !dir.is_dir() {
        return Err(format!("目录不存在：{}", dir.display()));
    }
    let mut cmd = Command::new("xdg-open");
    cmd.arg(dir);
    if crate::proc::spawn_detached(&mut cmd).is_err() {
        let mut fallback = Command::new("gio");
        fallback.arg("open").arg(dir);
        crate::proc::spawn_detached(&mut fallback)
            .map_err(|e| format!("xdg-open 与 gio 都不可用：{e}"))?;
    }
    Ok(format!("已用默认文件管理器打开 {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::os::unix::fs::symlink;

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "devtray-disk-{tag}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn dir(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            fs::create_dir_all(&p).unwrap();
            p
        }

        fn file(&self, rel: &str, bytes: usize) -> PathBuf {
            let p = self.0.join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&p, vec![0u8; bytes]).unwrap();
            p
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn project(root: &Path) -> ProjectRoot {
        ProjectRoot {
            project: "p".into(),
            root: fs::canonicalize(root).unwrap(),
            inferred: true,
            cleaning_allowed: true,
            reason: None,
            notes: Vec::new(),
        }
    }

    #[test]
    fn sparse_file_reports_zero_allocated_but_full_logical_size() {
        let t = Tmp::new("sparse");
        let node = t.dir("app/node_modules");
        t.file("app/package.json", 2);
        t.file("app/bun.lock", 2);
        let f = File::create(node.join("sparse.bin")).unwrap();
        f.set_len(1024 * 1024 * 1024).unwrap();
        drop(f);

        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage
            .artifacts
            .iter()
            .find(|a| a.name == "node_modules")
            .unwrap();
        assert_eq!(artifact.allocated, 0);
        assert!(artifact.logical >= 1024 * 1024 * 1024);
        assert!(artifact.deletable);
    }

    #[test]
    fn node_modules_needs_manifest_and_lockfile() {
        let t = Tmp::new("node");
        t.dir("app/node_modules/pkg");
        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage
            .artifacts
            .iter()
            .find(|a| a.name == "node_modules")
            .unwrap();
        assert!(!artifact.deletable, "没有 package.json 时不应可删");

        t.file("app/package.json", 2);
        t.dir("app/node_modules/pkg");
        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage
            .artifacts
            .iter()
            .find(|a| a.name == "node_modules")
            .unwrap();
        assert!(!artifact.deletable, "缺少 lockfile 不应可删");

        t.file("app/bun.lock", 2);
        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage
            .artifacts
            .iter()
            .find(|a| a.name == "node_modules")
            .unwrap();
        assert!(artifact.deletable);
    }

    #[test]
    fn target_needs_cargo_manifest_and_build_marker() {
        let t = Tmp::new("target");
        t.dir("repo/target/debug/deps");
        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage.artifacts.iter().find(|a| a.name == "target").unwrap();
        assert!(!artifact.deletable, "没有 Cargo.toml 不应可删");

        t.file("repo/Cargo.toml", 2);
        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage.artifacts.iter().find(|a| a.name == "target").unwrap();
        assert!(artifact.deletable);
    }

    #[test]
    fn venv_requires_pyvenv_cfg_and_rebuild_file() {
        let t = Tmp::new("venv");
        let venv = t.dir("app/.venv");
        fs::write(venv.join("pyvenv.cfg"), "home = /usr/bin\n").unwrap();
        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage.artifacts.iter().find(|a| a.name == ".venv").unwrap();
        assert!(
            !artifact.deletable,
            "只有 pyproject.toml/无重建依据时不应可删"
        );

        t.file("app/pyproject.toml", 2);
        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage.artifacts.iter().find(|a| a.name == ".venv").unwrap();
        assert!(!artifact.deletable, "只有 pyproject.toml 时拒绝");

        t.file("app/requirements.txt", 2);
        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage.artifacts.iter().find(|a| a.name == ".venv").unwrap();
        assert!(artifact.deletable);
    }

    #[test]
    fn pycache_needs_pyc() {
        let t = Tmp::new("pycache");
        t.dir("app/__pycache__");
        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage
            .artifacts
            .iter()
            .find(|a| a.name == "__pycache__")
            .unwrap();
        assert!(!artifact.deletable);

        t.file("app/__pycache__/m.pyc", 2);
        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage
            .artifacts
            .iter()
            .find(|a| a.name == "__pycache__")
            .unwrap();
        assert!(artifact.deletable);
    }

    #[test]
    fn hardlinks_are_counted_once() {
        let t = Tmp::new("inode");
        t.file("app/package.json", 2);
        t.file("app/bun.lock", 2);
        let a = t.file("app/node_modules/a.bin", 8192);
        fs::hard_link(&a, t.0.join("app/node_modules/b.bin")).unwrap();
        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage
            .artifacts
            .iter()
            .find(|a| a.name == "node_modules")
            .unwrap();
        let expected = fs::metadata(&a).unwrap().blocks() * 512;
        assert_eq!(artifact.files, 1);
        assert_eq!(artifact.allocated, expected);
    }

    #[test]
    fn symlinked_directory_is_not_followed() {
        let t = Tmp::new("symlink");
        t.file("app/package.json", 2);
        t.file("app/bun.lock", 2);
        t.dir("app/node_modules");
        let outside = t.dir("outside");
        fs::write(outside.join("big.bin"), vec![0u8; 4096]).unwrap();
        symlink(&outside, t.0.join("app/node_modules/linked")).unwrap();
        let usage = scan(&project(&t.0), &ScanOptions::default());
        let artifact = usage
            .artifacts
            .iter()
            .find(|a| a.name == "node_modules")
            .unwrap();
        assert_eq!(artifact.allocated, 0);
        assert_eq!(artifact.files, 0);
    }

    #[test]
    fn mountinfo_parsing_handles_escapes() {
        let text = "36 35 98:0 /mnt1 /mnt/with\\040space rw,noatime - ext4 /dev/sda1 rw\n\
                    37 35 98:0 / /home/ferychen rw - ext4 /dev/sda2 rw\n";
        let mounts = parse_mountinfo(text);
        assert!(mounts.contains(Path::new("/mnt/with space")));
        assert!(mounts.contains(Path::new("/home/ferychen")));
    }

    #[test]
    fn vcs_tracked_blocks_deletion() {
        let t = Tmp::new("git");
        t.file("repo/Cargo.toml", 2);
        t.dir("repo/target/debug");
        t.file("repo/target/debug/tracked.bin", 16);
        let repo = t.0.join("repo");
        let init = Command::new("git")
            .args(["-C", repo.to_str().unwrap(), "init", "-q"])
            .status();
        if !matches!(init, Ok(s) if s.success()) {
            return; // 没有 git 就跳过这条
        }
        let add = Command::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "add",
                "target/debug/tracked.bin",
            ])
            .status();
        if !matches!(add, Ok(s) if s.success()) {
            return;
        }
        let usage = scan(&project(&repo), &ScanOptions::default());
        let artifact = usage.artifacts.iter().find(|a| a.name == "target").unwrap();
        assert!(!artifact.deletable);
        assert!(artifact.evidence.vcs_tracked);
    }

    #[test]
    fn duplicate_scan_is_deterministic() {
        let t = Tmp::new("stable");
        t.file("app/package.json", 2);
        t.file("app/bun.lock", 2);
        t.file("app/node_modules/x.bin", 4096);
        let a = scan(&project(&t.0), &ScanOptions::default());
        let b = scan(&project(&t.0), &ScanOptions::default());
        assert_eq!(a.total_allocated, b.total_allocated);
        assert_eq!(a.artifacts.len(), b.artifacts.len());
    }
}
