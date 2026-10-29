//! Skills 清单（只读）：`~/.agents` 主存储 + `.skill-lock.json` 对账 + agent 副本漂移。
//!
//! 这一版不联网检查更新、不升级/删除/安装；先把"装了什么、来源是什么、哪份副本不一致"
//! 如实列出来。写操作要等 Skills 写门禁上线。

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use serde::{Deserialize, Serialize};

use crate::jobs::Job;

const MIRRORS: &[(&str, &str)] = &[
    ("claude-code", ".claude/skills"),
    ("codex", ".codex/skills"),
    ("gemini-cli", ".gemini/skills"),
    ("opencode", ".config/opencode/skills"),
];

#[derive(Debug, Deserialize, Default)]
struct LockFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    skills: BTreeMap<String, LockEntry>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LockEntry {
    #[serde(default)]
    source: Option<String>,
    #[serde(default, rename = "sourceType")]
    source_type: Option<String>,
    #[serde(default, rename = "sourceUrl")]
    source_url: Option<String>,
    #[serde(default, rename = "skillPath")]
    skill_path: Option<String>,
    #[serde(default, rename = "skillFolderHash")]
    tree_sha: Option<String>,
    #[serde(default, rename = "installedAt")]
    installed_at: Option<String>,
    #[serde(default, rename = "updatedAt")]
    updated_at: Option<String>,
    #[serde(default, rename = "ref")]
    git_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mirror {
    pub agent: String,
    pub path: String,
    pub kind: String,
    /// same / drift / unknown
    pub status: String,
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub display_name: Option<String>,
    pub dir: String,
    pub dir_exists: bool,
    pub managed: bool,
    pub source: Option<String>,
    pub source_type: Option<String>,
    pub source_url: Option<String>,
    pub git_ref: Option<String>,
    pub skill_path: Option<String>,
    pub tree_sha: Option<String>,
    pub installed_at: Option<String>,
    pub updated_at: Option<String>,
    /// unchecked / current / available / unknown
    pub update: String,
    pub latest_sha: Option<String>,
    pub update_checked_at: Option<u64>,
    pub update_detail: Option<String>,
    pub mirrors: Vec<Mirror>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub total: usize,
    pub managed: usize,
    pub unmanaged: usize,
    pub missing_dir: usize,
    pub drift_mirrors: usize,
    pub mirror_count: usize,
    pub lock_path: String,
    pub lock_ok: bool,
    pub lock_version: u32,
    pub skills_dir: String,
    pub update_available: usize,
    pub update_current: usize,
    pub update_unknown: usize,
    pub update_unchecked: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateInfo {
    /// current / available / unknown
    pub status: String,
    pub latest_sha: Option<String>,
    pub checked_at: u64,
    pub detail: Option<String>,
}

type UpdateCache = BTreeMap<String, UpdateInfo>;

#[derive(Debug, Clone, Serialize)]
pub struct Inventory {
    pub skills: Vec<SkillInfo>,
    pub summary: Summary,
}

pub fn inventory() -> Inventory {
    inventory_at(&crate::config::home_dir(), &load_updates())
}

fn inventory_at(home: &Path, updates: &UpdateCache) -> Inventory {
    let agents_dir = home.join(".agents");
    let skills_dir = agents_dir.join("skills");
    let lock_path = agents_dir.join(".skill-lock.json");
    let lock = fs::read_to_string(&lock_path)
        .ok()
        .and_then(|text| serde_json::from_str::<LockFile>(&text).ok());
    let lock_ok = lock.is_some();
    let lock_version = lock.as_ref().map(|l| l.version).unwrap_or(0);
    let lock_skills = lock.map(|l| l.skills).unwrap_or_default();

    let mut names: Vec<String> = lock_skills.keys().cloned().collect();
    if let Ok(entries) = fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    names.sort();
    names.dedup();

    let mirrors: Vec<(&str, PathBuf)> = MIRRORS
        .iter()
        .map(|(agent, rel)| (*agent, home.join(rel)))
        .filter(|(_, dir)| dir.is_dir())
        .collect();

    let mut skills = Vec::new();
    for name in names {
        let dir = skills_dir.join(&name);
        let dir_exists = dir.is_dir();
        let locked = lock_skills.get(&name);
        let display_name = if dir_exists {
            frontmatter_name(&dir)
        } else {
            None
        };
        let mut notes = Vec::new();
        if locked.is_none() {
            notes.push("来源未知：lock 里没有这条记录".into());
        }
        if !dir_exists {
            notes.push("lock 有记录，但主存储目录不存在".into());
        }
        if dir_exists && display_name.is_none() {
            notes.push("SKILL.md 缺少 frontmatter name".into());
        }
        if let (Some(display), true) = (&display_name, dir_exists)
            && display != &name
        {
            notes.push(format!("frontmatter name = {display}，安装目录 = {name}"));
        }
        let mirrors = if dir_exists {
            mirror_status(&dir, &name, &mirrors)
        } else {
            Vec::new()
        };
        let drift = mirrors.iter().any(|m| m.status == "drift");
        let mut skill = SkillInfo {
            name,
            display_name,
            dir: dir.display().to_string(),
            dir_exists,
            managed: locked.is_some(),
            source: locked.and_then(|l| l.source.clone()),
            source_type: locked.and_then(|l| l.source_type.clone()),
            source_url: locked.and_then(|l| l.source_url.clone()),
            git_ref: locked.and_then(|l| l.git_ref.clone()),
            skill_path: locked.and_then(|l| l.skill_path.clone()),
            tree_sha: locked.and_then(|l| l.tree_sha.clone()),
            installed_at: locked.and_then(|l| l.installed_at.clone()),
            updated_at: locked.and_then(|l| l.updated_at.clone()),
            update: "unchecked".into(),
            latest_sha: None,
            update_checked_at: None,
            update_detail: None,
            mirrors,
            notes: if drift {
                notes.push("至少一份 agent 副本与主存储不一致".into());
                notes
            } else {
                notes
            },
        };
        if let Some(info) = updates.get(&skill.name) {
            skill.update = info.status.clone();
            skill.latest_sha = info.latest_sha.clone();
            skill.update_checked_at = Some(info.checked_at);
            skill.update_detail = info.detail.clone();
        }
        skills.push(skill);
    }
    skills.sort_by(|a, b| {
        let key = |s: &SkillInfo| (!s.managed, s.name.clone());
        key(a).cmp(&key(b))
    });

    let managed = skills.iter().filter(|s| s.managed).count();
    let missing_dir = skills.iter().filter(|s| !s.dir_exists).count();
    let drift_mirrors = skills
        .iter()
        .map(|s| s.mirrors.iter().filter(|m| m.status == "drift").count())
        .sum();
    let mirror_count = skills.iter().map(|s| s.mirrors.len()).sum();
    Inventory {
        summary: Summary {
            total: skills.len(),
            managed,
            unmanaged: skills.len() - managed,
            missing_dir,
            drift_mirrors,
            mirror_count,
            lock_path: lock_path.display().to_string(),
            lock_ok,
            lock_version,
            skills_dir: skills_dir.display().to_string(),
            update_available: skills.iter().filter(|s| s.update == "available").count(),
            update_current: skills.iter().filter(|s| s.update == "current").count(),
            update_unknown: skills.iter().filter(|s| s.update == "unknown").count(),
            update_unchecked: skills.iter().filter(|s| s.update == "unchecked").count(),
        },
        skills,
    }
}

fn frontmatter_name(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix("name:") {
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn mirror_status(skill_dir: &Path, name: &str, mirrors: &[(&str, PathBuf)]) -> Vec<Mirror> {
    let mut out = Vec::new();
    for (agent, base) in mirrors {
        let path = base.join(name);
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            let target = fs::read_link(&path).ok();
            let status = match target
                .as_ref()
                .map(|t| fs::canonicalize(wrap(t, base)).ok())
            {
                Some(Some(real)) => {
                    let main = fs::canonicalize(skill_dir).unwrap_or_else(|_| skill_dir.into());
                    if real == main { "same" } else { "drift" }
                }
                _ => "unknown",
            };
            out.push(Mirror {
                agent: agent.to_string(),
                path: path.display().to_string(),
                kind: "symlink".into(),
                status: status.into(),
                target: target.map(|t| t.display().to_string()),
            });
        } else if meta.is_dir() {
            out.push(Mirror {
                agent: agent.to_string(),
                path: path.display().to_string(),
                kind: "copy".into(),
                status: diff_status(skill_dir, &path).into(),
                target: None,
            });
        }
    }
    out
}

/// 相对符号链接按所在目录解析；绝对路径原样返回。
fn wrap(target: &Path, base: &Path) -> PathBuf {
    if target.is_absolute() {
        target.to_path_buf()
    } else {
        base.join(target)
    }
}

fn diff_status(main: &Path, copy: &Path) -> &'static str {
    let out = Command::new("diff")
        .args(["-rq", "--exclude=.git", "--exclude=node_modules"])
        .arg(main)
        .arg(copy)
        .output();
    match out {
        Ok(o) if o.status.success() => "same",
        Ok(o) if o.status.code() == Some(1) => "drift",
        _ => "unknown",
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn update_cache_path() -> PathBuf {
    crate::config::state_dir()
        .join("cache")
        .join("skills-updates.json")
}

fn load_updates() -> UpdateCache {
    fs::read_to_string(update_cache_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_updates(cache: &UpdateCache) -> Result<(), String> {
    let path = update_cache_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("创建 Skills 缓存目录失败：{e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(cache).map_err(|e| e.to_string())?;
    fs::write(&tmp, text).map_err(|e| format!("写 Skills 缓存失败：{e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("提交 Skills 缓存失败：{e}"))
}

fn check_ttl() -> u64 {
    let path = std::env::var("DEVTRAY_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate::config::default_config_path());
    crate::config::Config::load(&path)
        .map(|c| c.settings.skill_check_ttl_s)
        .unwrap_or(21600)
}

#[derive(Debug, Deserialize)]
struct TreeResponse {
    sha: String,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    tree: Vec<TreeEntry>,
}

#[derive(Debug, Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
}

#[derive(Debug, Deserialize)]
struct RepoInfo {
    default_branch: String,
}

fn repo_of(skill: &SkillInfo) -> Option<String> {
    if let Some(source) = &skill.source {
        let source = source.trim_end_matches(".git");
        if !source.contains("://") && !source.contains('@') && source.matches('/').count() == 1 {
            return Some(source.to_string());
        }
    }
    let url = skill.source_url.as_deref()?.trim_end_matches(".git");
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))?;
    let parts: Vec<&str> = rest.split('/').collect();
    (parts.len() >= 2).then(|| format!("{}/{}", parts[0], parts[1]))
}

fn folder_sha(tree: &TreeResponse, skill_path: &str) -> Option<String> {
    let folder = Path::new(skill_path)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or("")
        .trim_end_matches('/');
    if folder.is_empty() {
        return Some(tree.sha.clone());
    }
    tree.tree
        .iter()
        .find(|entry| entry.kind == "tree" && entry.path == folder)
        .map(|entry| entry.sha.clone())
}

fn api_get(path: &str) -> Result<String, String> {
    let mut gh_error = None;
    if crate::terminal::which("gh")
        && let Ok(out) = Command::new("gh").args(["api", path]).output()
    {
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        gh_error = Some(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let url = format!("https://api.github.com/{path}");
    let out = Command::new("curl")
        .args([
            "-sS",
            "-f",
            "--max-time",
            "20",
            "-H",
            "Accept: application/vnd.github+json",
            &url,
        ])
        .output()
        .map_err(|e| {
            let previous = gh_error
                .as_deref()
                .map(|g| format!("gh 失败：{g}；"))
                .unwrap_or_default();
            format!("{previous}curl 不可用：{e}")
        })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if let Some(gh) = gh_error {
            format!("gh 失败：{gh}；curl 失败：{err}")
        } else {
            format!("GitHub API 失败：{err}")
        })
    }
}

fn default_branch(repo: &str) -> Result<String, String> {
    let text = api_get(&format!("repos/{repo}"))?;
    let info: RepoInfo =
        serde_json::from_str(&text).map_err(|e| format!("仓库信息解析失败：{e}"))?;
    Ok(info.default_branch)
}

fn fetch_tree(repo: &str, git_ref: &str) -> Result<TreeResponse, String> {
    let text = api_get(&format!("repos/{repo}/git/trees/{git_ref}?recursive=1"))?;
    serde_json::from_str(&text).map_err(|e| format!("tree 解析失败：{e}"))
}

/// 联网检查上游 tree sha；结果写缓存，TTL 内不重复请求。
pub fn check_updates(refresh: bool) -> Result<Inventory, String> {
    let home = crate::config::home_dir();
    let mut updates = load_updates();
    let base = inventory_at(&home, &updates);
    let ttl = check_ttl();
    let now = now_secs();
    let mut branches: HashMap<String, Result<String, String>> = HashMap::new();
    let mut trees: HashMap<String, Result<TreeResponse, String>> = HashMap::new();

    for skill in &base.skills {
        if !skill.managed
            || skill.source_type.as_deref() != Some("github")
            || skill.skill_path.is_none()
        {
            continue;
        }
        if !refresh
            && let Some(info) = updates.get(&skill.name)
            && now.saturating_sub(info.checked_at) < ttl
        {
            continue;
        }
        let Some(repo) = repo_of(skill) else {
            updates.insert(
                skill.name.clone(),
                UpdateInfo {
                    status: "unknown".into(),
                    latest_sha: None,
                    checked_at: now,
                    detail: Some("无法从 source / sourceUrl 解析 GitHub 仓库".into()),
                },
            );
            continue;
        };
        let git_ref = match &skill.git_ref {
            Some(r) if !r.trim().is_empty() => r.clone(),
            _ => {
                let entry = branches
                    .entry(repo.clone())
                    .or_insert_with(|| default_branch(&repo));
                match entry {
                    Ok(r) => r.clone(),
                    Err(e) => {
                        updates.insert(
                            skill.name.clone(),
                            UpdateInfo {
                                status: "unknown".into(),
                                latest_sha: None,
                                checked_at: now,
                                detail: Some(format!("取默认分支失败：{e}")),
                            },
                        );
                        continue;
                    }
                }
            }
        };
        let key = format!("{repo}@{git_ref}");
        let tree = trees
            .entry(key)
            .or_insert_with(|| fetch_tree(&repo, &git_ref));
        let (status, latest, detail) = match tree {
            Ok(tree) if tree.truncated => (
                "unknown",
                None,
                Some("GitHub tree 被截断，无法比较".to_string()),
            ),
            Ok(tree) => match folder_sha(tree, skill.skill_path.as_deref().unwrap_or("")) {
                Some(sha) => {
                    let status = if Some(&sha) == skill.tree_sha.as_ref() {
                        "current"
                    } else {
                        "available"
                    };
                    (status, Some(sha), Some(format!("{repo}@{git_ref}")))
                }
                None => (
                    "unknown",
                    None,
                    Some(format!("上游找不到技能目录（{repo}@{git_ref}）")),
                ),
            },
            Err(e) => ("unknown", None, Some(e.clone())),
        };
        updates.insert(
            skill.name.clone(),
            UpdateInfo {
                status: status.into(),
                latest_sha: latest,
                checked_at: now,
                detail,
            },
        );
    }

    save_updates(&updates)?;
    Ok(inventory_at(&home, &updates))
}

const SKILLS_CLI: &str = "npx";
const SKILLS_PACKAGE: &str = "skills@1.7.4";

#[derive(Debug, Clone, Serialize)]
pub struct SkillPlan {
    pub action: String,
    pub name: String,
    pub display_name: Option<String>,
    pub source: Option<String>,
    pub dir: String,
    pub paths: Vec<String>,
    pub mirrors: Vec<Mirror>,
    pub drift: bool,
    pub allow_drift: bool,
    pub command: String,
    pub lock_path: String,
}

fn cli_action(action: &str) -> &str {
    if action == "upgrade" {
        "update"
    } else {
        action
    }
}

fn cli_args(action: &str, name: &str) -> Vec<String> {
    vec![
        "--yes".into(),
        SKILLS_PACKAGE.into(),
        cli_action(action).into(),
        name.into(),
        "-g".into(),
        "-y".into(),
    ]
}

fn display_command(action: &str, name: &str) -> String {
    format!(
        "{SKILLS_CLI} --yes {SKILLS_PACKAGE} {} {name} -g -y",
        cli_action(action)
    )
}

pub fn plan_action(action: &str, name: &str, allow_drift: bool) -> Result<SkillPlan, String> {
    if !matches!(action, "upgrade" | "remove") {
        return Err(format!("未知技能操作：{action}"));
    }
    let inv = inventory();
    let skill = inv
        .skills
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("找不到技能 {name:?}"))?;
    if !skill.managed {
        return Err(format!(
            "{name} 没有 lock 记录（来源未知），不能直接{action}；请先用 npx skills add 重新纳入管理"
        ));
    }
    if !skill.dir_exists {
        return Err(format!("{name} 的主存储目录不存在：{}", skill.dir));
    }
    let drift = skill.mirrors.iter().any(|m| m.status == "drift");
    if drift && !allow_drift {
        let paths: Vec<&str> = skill
            .mirrors
            .iter()
            .filter(|m| m.status == "drift")
            .map(|m| m.path.as_str())
            .collect();
        return Err(format!(
            "{name} 存在内容不一致的 agent 副本，默认拒绝写入：{}\n确认要覆盖时加 --allow-drift",
            paths.join("、")
        ));
    }
    let mut paths = vec![skill.dir.clone()];
    paths.extend(skill.mirrors.iter().map(|m| m.path.clone()));
    Ok(SkillPlan {
        action: action.into(),
        name: name.into(),
        display_name: skill.display_name.clone(),
        source: skill.source.clone(),
        dir: skill.dir.clone(),
        paths,
        mirrors: skill.mirrors.clone(),
        drift,
        allow_drift,
        command: display_command(action, name),
        lock_path: inv.summary.lock_path,
    })
}

fn safe_component(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "skill".into()
    } else {
        cleaned
    }
}

fn backup_root(name: &str) -> PathBuf {
    crate::config::state_dir()
        .join("skills-backup")
        .join(safe_component(name))
}

fn snapshot(plan: &SkillPlan) -> Result<PathBuf, String> {
    let root = backup_root(&plan.name);
    let stamp = format!(
        "{}-{}",
        now_secs(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    );
    let dest = root.join(&stamp);
    fs::create_dir_all(&dest).map_err(|e| format!("创建技能快照目录失败：{e}"))?;

    let main = PathBuf::from(&plan.dir);
    copy_tree(&main, &dest.join("canonical"))?;
    for mirror in &plan.mirrors {
        if mirror.kind == "copy" {
            let src = PathBuf::from(&mirror.path);
            if src.is_dir() {
                copy_tree(
                    &src,
                    &dest.join("agents").join(safe_component(&mirror.agent)),
                )?;
            }
        }
    }

    let lock_entry = fs::read_to_string(&plan.lock_path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v.get("skills")?.get(&plan.name).cloned());
    if let Some(entry) = &lock_entry {
        fs::write(
            dest.join("lock-entry.json"),
            serde_json::to_string_pretty(entry).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("写 lock 快照失败：{e}"))?;
    }
    let manifest = serde_json::json!({
        "name": plan.name,
        "action": plan.action,
        "source": plan.source,
        "dir": plan.dir,
        "paths": plan.paths,
        "mirrors": plan.mirrors,
        "allow_drift": plan.allow_drift,
        "command": plan.command,
        "created_at": now_secs(),
        "lock_entry": lock_entry,
    });
    fs::write(
        dest.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("写快照 manifest 失败：{e}"))?;

    // 每个技能只保留最近 3 份
    if let Ok(entries) = fs::read_dir(&root) {
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        while dirs.len() > 3 {
            let old = dirs.remove(0);
            let _ = fs::remove_dir_all(old);
        }
    }
    Ok(dest)
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("创建 {} 失败：{e}", dst.display()))?;
    let entries = fs::read_dir(src).map_err(|e| format!("读取 {} 失败：{e}", src.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == ".git" || name == "node_modules" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        let meta = fs::symlink_metadata(&from).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            let target = fs::read_link(&from).map_err(|e| e.to_string())?;
            std::os::unix::fs::symlink(&target, &to).map_err(|e| e.to_string())?;
        } else if meta.is_dir() {
            copy_tree(&from, &to)?;
        } else if meta.is_file() {
            fs::copy(&from, &to).map_err(|e| format!("复制 {} 失败：{e}", from.display()))?;
        }
    }
    Ok(())
}

fn job_log_path(id: &str) -> PathBuf {
    crate::config::state_dir()
        .join("jobs")
        .join(format!("{}.log", safe_component(id)))
}

fn invalidate_update_cache(name: &str) {
    let mut updates = load_updates();
    if updates.remove(name).is_some() {
        let _ = save_updates(&updates);
    }
}

fn verify(plan: &SkillPlan, inventory: &Inventory) -> Result<(), String> {
    let skill = inventory.skills.iter().find(|s| s.name == plan.name);
    match plan.action.as_str() {
        "upgrade" => {
            let Some(skill) = skill else {
                return Err("升级后找不到技能记录".into());
            };
            if !skill.dir_exists || !Path::new(&skill.dir).join("SKILL.md").is_file() {
                return Err("升级后主存储或 SKILL.md 缺失".into());
            }
            if !skill.managed {
                return Err("升级后 lock 记录消失".into());
            }
        }
        "remove" => {
            if let Some(skill) = skill {
                if skill.dir_exists {
                    return Err(format!("删除后主存储仍存在：{}", skill.dir));
                }
                if skill.managed {
                    return Err("删除后 lock 记录仍在".into());
                }
                if let Some(mirror) = skill.mirrors.iter().find(|m| Path::new(&m.path).exists()) {
                    return Err(format!("删除后 agent 副本仍存在：{}", mirror.path));
                }
            }
            for path in &plan.paths {
                if Path::new(path).exists() {
                    return Err(format!("删除后路径仍存在：{path}"));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn execute_job(job: &mut Job) -> Result<String, String> {
    let action = job
        .action
        .strip_prefix("skill-")
        .ok_or("任务不是技能操作")?
        .to_string();
    let name = job.target.clone().ok_or("任务缺少技能名")?;
    let allow_drift = job
        .params
        .as_ref()
        .and_then(|p| p.get("allow_drift"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let _guard = crate::ops::acquire(std::time::Duration::from_secs(5))?;
    // 拿锁后重新枚举，避免确认期间新增副本/改 lock
    let plan = plan_action(&action, &name, allow_drift)?;
    let backup = snapshot(&plan)?;
    job.mark_running(Some("执行 skills CLI"));
    job.save()?;

    if !crate::terminal::which(SKILLS_CLI) {
        return Err("找不到 npx；需要 Node.js 才能执行 skills".into());
    }
    let log_path = job_log_path(&job.id);
    if let Some(dir) = log_path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("创建任务日志目录失败：{e}"))?;
    }
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("打开任务日志失败：{e}"))?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;
    let status = Command::new(SKILLS_CLI)
        .args(cli_args(&action, &name))
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .status()
        .map_err(|e| format!("执行 skills CLI 失败：{e}"))?;
    if !status.success() {
        return Err(format!(
            "skills {action} 失败（退出码 {:?}）；日志：{}",
            status.code(),
            log_path.display()
        ));
    }

    let inventory = inventory();
    verify(&plan, &inventory)?;
    invalidate_update_cache(&name);
    let _ = crate::clean::append_audit(job, "ok", &plan.paths, &[], 0);
    Ok(format!(
        "{} {} 完成；快照：{}；日志：{}",
        action,
        name,
        backup.display(),
        log_path.display()
    ))
}

pub fn run_job(id: Option<&String>) -> std::process::ExitCode {
    let result = (|| -> Result<String, String> {
        let id = id.ok_or("用法：devtray _skills-run <job-id>")?;
        let mut job = Job::load(id)?;
        if !matches!(
            job.state,
            crate::jobs::JobState::PendingApproval | crate::jobs::JobState::Running
        ) {
            return Err(format!("任务 {} 已是 {:?}，不重复执行", job.id, job.state));
        }
        let message = if job.action == "skill-install" {
            execute_install(&mut job)?
        } else {
            execute_job(&mut job)?
        };
        job.finish_ok(serde_json::json!({"message": message}));
        job.save()?;
        Ok(message)
    })();
    match result {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            if let Some(id) = id
                && let Ok(mut job) = Job::load(id)
            {
                let _ = crate::clean::append_audit(&job, "error", &[], &[], 0);
                job.finish_error(e.clone());
                let _ = job.save();
            }
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

pub fn plan_command(args: &[String]) -> ExitCode {
    let parsed = parse_write_args(args);
    let parsed = match parsed {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match plan_action(&parsed.action, &parsed.name, parsed.allow_drift) {
        Ok(plan) => match serde_json::to_string_pretty(&plan) {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

struct WriteArgs {
    action: String,
    name: String,
    allow_drift: bool,
}

fn parse_write_args(args: &[String]) -> Result<WriteArgs, String> {
    let mut action = None;
    let mut name = None;
    let mut allow_drift = false;
    for arg in args {
        match arg.as_str() {
            "--allow-drift" => allow_drift = true,
            "upgrade" | "remove" if action.is_none() => action = Some(arg.clone()),
            other if other.starts_with('-') => return Err(format!("未知选项：{other}")),
            other if name.is_none() => name = Some(other.to_string()),
            other => return Err(format!("多余的参数：{other}")),
        }
    }
    Ok(WriteArgs {
        action: action.ok_or("缺少操作：upgrade 或 remove")?,
        name: name.ok_or("缺少技能名")?,
        allow_drift,
    })
}

pub fn start_command(args: &[String]) -> ExitCode {
    let parsed = match parse_write_args(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let result = (|| -> Result<String, String> {
        let plan = plan_action(&parsed.action, &parsed.name, parsed.allow_drift)?;
        let mut job = Job::new(&format!("skill-{}", parsed.action), None, &plan.paths);
        job.target = Some(parsed.name.clone());
        job.params = Some(serde_json::json!({"allow_drift": parsed.allow_drift}));
        job.source = "panel".into();
        job.save()?;
        crate::clean::spawn_worker("_skills-run", &job.id)?;
        serde_json::to_string(&serde_json::json!({"job_id": job.id, "plan": plan}))
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub name: String,
    pub action: String,
    #[serde(default)]
    pub mirrors: Vec<Mirror>,
    #[serde(default)]
    pub lock_entry: Option<serde_json::Value>,
    #[serde(default)]
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    pub id: String,
    pub action: String,
    pub created_at: u64,
    pub has_lock: bool,
    pub mirrors: usize,
    pub path: String,
}

fn list_backups(name: &str) -> Vec<BackupInfo> {
    let root = backup_root(name);
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out: Vec<BackupInfo> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            let text = fs::read_to_string(e.path().join("manifest.json")).ok()?;
            let manifest: BackupManifest = serde_json::from_str(&text).ok()?;
            Some(BackupInfo {
                id,
                action: manifest.action,
                created_at: manifest.created_at,
                has_lock: manifest.lock_entry.is_some(),
                mirrors: manifest.mirrors.len(),
                path: e.path().display().to_string(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

fn expected_not_exist(path: &Path) -> bool {
    !path.exists() && fs::symlink_metadata(path).is_err()
}

fn replace_with_dir(src: &Path, dst: &Path) -> Result<(), String> {
    if dst.exists() {
        fs::remove_dir_all(dst).map_err(|e| format!("清除 {} 失败：{e}", dst.display()))?;
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 {} 失败：{e}", parent.display()))?;
    }
    copy_tree(src, dst)
}

fn restore(name: &str, snapshot: Option<&str>, restore_lock: bool) -> Result<String, String> {
    let backups = list_backups(name);
    if backups.is_empty() {
        return Err(format!("{name} 没有可用快照"));
    }
    let chosen = match snapshot {
        Some(id) => backups
            .iter()
            .find(|b| b.id == id)
            .ok_or_else(|| format!("找不到快照 {id}"))?
            .clone(),
        None => backups[0].clone(),
    };
    let base = PathBuf::from(&chosen.path);
    let manifest: BackupManifest = serde_json::from_str(
        &fs::read_to_string(base.join("manifest.json"))
            .map_err(|e| format!("读取快照 manifest 失败：{e}"))?,
    )
    .map_err(|e| format!("快照 manifest 损坏：{e}"))?;
    if manifest.name != name {
        return Err(format!(
            "快照属于 {}，与请求的 {name} 不一致",
            manifest.name
        ));
    }

    let home = crate::config::home_dir();
    let skills_dir = home.join(".agents").join("skills");
    let main = skills_dir.join(name);
    let allowed_mirror = |path: &str| -> bool {
        MIRRORS
            .iter()
            .any(|(_, rel)| home.join(rel).join(name).display().to_string() == path)
    };

    let _guard = crate::ops::acquire(std::time::Duration::from_secs(5))?;
    replace_with_dir(&base.join("canonical"), &main)?;

    for mirror in &manifest.mirrors {
        if !allowed_mirror(&mirror.path) {
            return Err(format!("快照里的副本路径不在允许范围：{}", mirror.path));
        }
        let target = PathBuf::from(&mirror.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建 {} 失败：{e}", parent.display()))?;
        }
        if expected_not_exist(&target) {
            // 不存在就按快照重建
        } else if target.is_dir() && !target.is_symlink() {
            fs::remove_dir_all(&target)
                .map_err(|e| format!("清除副本 {} 失败：{e}", target.display()))?;
        } else {
            fs::remove_file(&target)
                .map_err(|e| format!("清除副本 {} 失败：{e}", target.display()))?;
        }
        match mirror.kind.as_str() {
            "symlink" => {
                let link_target = mirror
                    .target
                    .as_deref()
                    .ok_or_else(|| format!("快照副本 {} 缺少链接目标", mirror.path))?;
                std::os::unix::fs::symlink(link_target, &target)
                    .map_err(|e| format!("重建符号链接 {} 失败：{e}", target.display()))?;
            }
            "copy" => {
                let src = base.join("agents").join(safe_component(&mirror.agent));
                if !src.is_dir() {
                    return Err(format!("快照缺少副本内容：{}", src.display()));
                }
                copy_tree(&src, &target)?;
            }
            other => return Err(format!("未知副本类型：{other}")),
        }
    }

    if restore_lock {
        let entry = manifest
            .lock_entry
            .clone()
            .ok_or("该快照没有 lock 记录，无法恢复版本元数据")?;
        let lock_path = home.join(".agents").join(".skill-lock.json");
        let mut lock: serde_json::Value = fs::read_to_string(&lock_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_else(|| serde_json::json!({"version": 3, "skills": {}}));
        let before = lock["skills"]
            .as_object()
            .cloned()
            .ok_or("lock 文件结构异常")?;
        lock["skills"]
            .as_object_mut()
            .ok_or("lock 文件结构异常")?
            .insert(name.to_string(), entry);
        for (key, value) in &before {
            if key != name && lock["skills"].get(key) != Some(value) {
                return Err(format!("恢复 lock 时其他条目发生变化：{key}"));
            }
        }
        let tmp = lock_path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_string_pretty(&lock).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("写 lock 快照失败：{e}"))?;
        fs::rename(&tmp, &lock_path).map_err(|e| format!("提交 lock 失败：{e}"))?;
    }

    if !main.join("SKILL.md").is_file() {
        return Err("恢复后主存储缺少 SKILL.md".into());
    }
    let mut job = Job::new("skill-restore", None, &[main.display().to_string()]);
    job.target = Some(name.to_string());
    job.source = "cli".into();
    job.save()?;
    job.mark_running(Some("恢复快照"));
    job.save()?;
    let _ = crate::clean::append_audit(&job, "ok", &[main.display().to_string()], &[], 0);
    job.finish_ok(serde_json::json!({
        "snapshot": chosen.id,
        "restored_lock": restore_lock,
        "path": main.display().to_string(),
    }));
    job.save()?;
    Ok(format!(
        "已恢复 {name} 的快照 {}（lock：{}）",
        chosen.id,
        if restore_lock {
            "已恢复"
        } else {
            "未修改"
        }
    ))
}

fn render_install_plan(plan: &InstallPlan) -> String {
    format!(
        "将安装 {}（{}）\n候选：{}\n目标 agent：{}\n命令：{}\n安装后会校验 lock、主存储与每个 agent 副本。",
        plan.skill,
        plan.repo,
        plan.candidates.join("、"),
        plan.agents.join("、"),
        plan.command
    )
}

fn render_backups(name: &str, backups: &[BackupInfo]) -> String {
    if backups.is_empty() {
        return format!("{name} 没有可用快照");
    }
    let mut out = format!("{name} 的可用快照（新 → 旧）：\n");
    for backup in backups {
        out.push_str(&format!(
            "  {}  {}  {}  lock={}\n",
            backup.id,
            backup.action,
            backup.mirrors,
            if backup.has_lock { "有" } else { "无" }
        ));
    }
    out.trim_end().to_string()
}

pub fn backups_command(args: &[String]) -> ExitCode {
    let Some(name) = args.first() else {
        eprintln!("用法：devtray _skills-backups <技能名>");
        return ExitCode::FAILURE;
    };
    match serde_json::to_string_pretty(&list_backups(name)) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

pub fn restore_command(args: &[String]) -> ExitCode {
    let mut name = None;
    let mut snapshot = None;
    let mut restore_lock = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--snapshot" => snapshot = it.next().cloned(),
            "--restore-lock" => restore_lock = true,
            other if other.starts_with('-') => {
                eprintln!("未知选项：{other}");
                return ExitCode::FAILURE;
            }
            other if name.is_none() => name = Some(other.to_string()),
            other => {
                eprintln!("多余的参数：{other}");
                return ExitCode::FAILURE;
            }
        }
    }
    let Some(name) = name else {
        eprintln!("缺少技能名");
        return ExitCode::FAILURE;
    };
    match restore(&name, snapshot.as_deref(), restore_lock) {
        Ok(message) => {
            println!("{}", serde_json::json!({"message": message}));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallPlan {
    pub input: String,
    pub repo: String,
    pub git_ref: Option<String>,
    pub path: Option<String>,
    pub skill: String,
    pub candidates: Vec<String>,
    pub agents: Vec<String>,
    pub command: String,
    pub allow_existing: bool,
    #[serde(default)]
    pub argv: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    skills: Vec<SearchItem>,
}

#[derive(Debug, Deserialize)]
struct SearchItem {
    #[serde(default, rename = "skillId")]
    skill_id: String,
    #[serde(default)]
    source: String,
}

fn search_skills(query: &str) -> Result<(String, String), String> {
    let out = Command::new("curl")
        .args([
            "-sS",
            "-f",
            "--max-time",
            "20",
            "--get",
            "--data-urlencode",
            &format!("q={query}"),
            "https://skills.sh/api/search",
        ])
        .output()
        .map_err(|e| format!("搜索技能失败：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "skills.sh 搜索失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let body: SearchResponse =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("skills.sh 返回无法解析：{e}"))?;
    let hits: Vec<(String, String)> = body
        .skills
        .into_iter()
        .filter(|s| !s.skill_id.is_empty() && s.source.matches('/').count() == 1)
        .map(|s| (s.source, s.skill_id))
        .collect();
    match hits.len() {
        0 => Err(format!("没有找到名为 {query:?} 的技能")),
        1 => Ok(hits.into_iter().next().unwrap()),
        _ => {
            let list = hits
                .iter()
                .take(5)
                .map(|(repo, skill)| format!("{repo}@{skill}"))
                .collect::<Vec<_>>()
                .join("、");
            Err(format!(
                "{query:?} 有多个候选：{list}；请改用 owner/repo@skill 指定"
            ))
        }
    }
}

struct SourceSpec {
    repo: String,
    git_ref: Option<String>,
    path: Option<String>,
    skill: Option<String>,
}

fn parse_source(input: &str) -> Result<SourceSpec, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("缺少技能来源".into());
    }
    if !input.contains('/') && !input.contains(':') && !input.contains('@') {
        let (repo, skill) = search_skills(input)?;
        return Ok(SourceSpec {
            repo,
            git_ref: None,
            path: None,
            skill: Some(skill),
        });
    }
    let mut rest = input.trim_end_matches(".git");
    if let Some((base, skill)) = rest.rsplit_once('@')
        && !base.contains("://")
        && !base.contains(':')
    {
        rest = base;
        return Ok(SourceSpec {
            repo: normalize_repo(rest)?,
            git_ref: None,
            path: None,
            skill: Some(skill.to_string()),
        });
    }
    if let Some(after) = rest
        .strip_prefix("https://github.com/")
        .or_else(|| rest.strip_prefix("http://github.com/"))
    {
        let parts: Vec<&str> = after.split('/').collect();
        if parts.len() < 2 {
            return Err("GitHub 地址缺少 owner/repo".into());
        }
        let repo = format!("{}/{}", parts[0], parts[1]);
        let (git_ref, path) = if parts.len() > 3 && parts[2] == "tree" {
            let git_ref = Some(parts[3].to_string());
            let path = (parts.len() > 4).then(|| parts[4..].join("/"));
            (git_ref, path)
        } else {
            (None, None)
        };
        return Ok(SourceSpec {
            repo,
            git_ref,
            path,
            skill: None,
        });
    }
    if let Some(after) = rest.strip_prefix("git@github.com:") {
        return Ok(SourceSpec {
            repo: normalize_repo(after)?,
            git_ref: None,
            path: None,
            skill: None,
        });
    }
    Ok(SourceSpec {
        repo: normalize_repo(rest)?,
        git_ref: None,
        path: None,
        skill: None,
    })
}

fn normalize_repo(repo: &str) -> Result<String, String> {
    let repo = repo.trim().trim_end_matches(".git");
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
        return Err(format!("仓库应写成 owner/repo，收到：{repo}"));
    }
    Ok(repo.to_string())
}

fn install_candidates(tree: &TreeResponse, subpath: Option<&str>) -> Vec<(String, String)> {
    let prefix = subpath.map(|p| p.trim_end_matches('/').to_string());
    tree.tree
        .iter()
        .filter(|entry| entry.kind == "blob" && entry.path.ends_with("SKILL.md"))
        .filter(|entry| {
            prefix
                .as_ref()
                .map(|p| entry.path.starts_with(p))
                .unwrap_or(true)
        })
        .filter_map(|entry| {
            let path = Path::new(&entry.path);
            let folder = path.parent()?;
            let name = if folder.as_os_str().is_empty() {
                return None;
            } else {
                folder.file_name()?.to_string_lossy().into_owned()
            };
            Some((name, folder.display().to_string()))
        })
        .collect()
}

pub fn plan_install(
    input: &str,
    skill_override: Option<&str>,
    agents_override: &[String],
    allow_existing: bool,
) -> Result<InstallPlan, String> {
    let spec = parse_source(input)?;
    let git_ref = spec.git_ref.clone();
    let r = git_ref.as_deref().unwrap_or("HEAD");
    let tree = fetch_tree(&spec.repo, r).or_else(|_| {
        let branch = default_branch(&spec.repo)?;
        fetch_tree(&spec.repo, &branch)
    })?;
    if tree.truncated {
        return Err("GitHub tree 被截断，无法预检技能列表".into());
    }
    let candidates = install_candidates(&tree, spec.path.as_deref());
    if candidates.is_empty() {
        return Err(format!("{} 里没有找到 SKILL.md", spec.repo));
    }
    let wanted = skill_override.or(spec.skill.as_deref());
    let chosen = match wanted {
        Some(name) => candidates.iter().find(|(n, _)| n == name).ok_or_else(|| {
            format!(
                "{} 里没有技能 {name:?}；可用：{}",
                spec.repo,
                candidates
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            )
        })?,
        None if candidates.len() == 1 => &candidates[0],
        None => {
            return Err(format!(
                "{} 有多个技能：{}；请用 --skill 指定",
                spec.repo,
                candidates
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            ));
        }
    };

    let inv = inventory();
    if !allow_existing && let Some(existing) = inv.skills.iter().find(|s| s.name == chosen.0) {
        return Err(format!(
            "技能 {} 已经存在（{}）；请用 skills upgrade，或加 --allow-existing",
            existing.name,
            existing.source.as_deref().unwrap_or("来源未知")
        ));
    }

    let mut agents: Vec<String> = if agents_override.is_empty() {
        // Only select targets whose written copies we can verify afterwards.
        crate::agents::detect()
            .into_iter()
            .filter(|agent| MIRRORS.iter().any(|(id, _)| *id == agent.id))
            .map(|a| a.id)
            .collect()
    } else {
        agents_override.to_vec()
    };
    agents.sort();
    agents.dedup();
    if agents.is_empty() {
        return Err("没有可用 agent：请用 --agent 指定（如 claude-code、codex）".into());
    }

    let source = match (&git_ref, &spec.path) {
        (Some(r), Some(p)) => format!("https://github.com/{}/tree/{r}/{p}", spec.repo),
        (Some(r), None) => format!("https://github.com/{}/tree/{r}", spec.repo),
        _ => spec.repo.clone(),
    };
    let mut argv = vec![
        "--yes".into(),
        SKILLS_PACKAGE.into(),
        "add".into(),
        source.clone(),
        "--skill".into(),
        chosen.0.clone(),
    ];
    for agent in &agents {
        argv.push("-a".into());
        argv.push(agent.clone());
    }
    argv.extend(["-g".into(), "-y".into(), "--json".into()]);
    let command = format!(
        "{SKILLS_CLI} --yes {SKILLS_PACKAGE} add {source} --skill {} {} -g -y --json",
        chosen.0,
        agents
            .iter()
            .map(|a| format!("-a {a}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    Ok(InstallPlan {
        input: input.into(),
        repo: spec.repo,
        git_ref,
        path: spec.path,
        skill: chosen.0.clone(),
        candidates: candidates.into_iter().map(|(n, _)| n).collect(),
        agents,
        command,
        allow_existing,
        argv,
    })
}

fn execute_install(job: &mut Job) -> Result<String, String> {
    let params = job.params.clone().ok_or("任务缺少安装参数")?;
    let source = params
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or("任务缺少 source")?;
    let skill = params.get("skill").and_then(|v| v.as_str());
    let agents: Vec<String> = params
        .get("agents")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let allow_existing = params
        .get("allow_existing")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let plan = plan_install(source, skill, &agents, allow_existing)?;

    let _guard = crate::ops::acquire(std::time::Duration::from_secs(5))?;
    if !crate::terminal::which(SKILLS_CLI) {
        return Err("找不到 npx；需要 Node.js 才能执行 skills".into());
    }
    let log_path = job_log_path(&job.id);
    if let Some(dir) = log_path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("创建任务日志目录失败：{e}"))?;
    }
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("打开任务日志失败：{e}"))?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;
    let status = Command::new(SKILLS_CLI)
        .args(&plan.argv)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .status()
        .map_err(|e| format!("执行 skills add 失败：{e}"))?;
    if !status.success() {
        return Err(format!(
            "skills add 失败（退出码 {:?}）；日志：{}",
            status.code(),
            log_path.display()
        ));
    }
    let inv = inventory();
    let installed = inv
        .skills
        .iter()
        .find(|s| s.name == plan.skill)
        .ok_or_else(|| format!("安装后找不到技能 {}", plan.skill))?;
    if !installed.managed || !installed.dir_exists {
        return Err("安装后 lock 或主存储缺失".into());
    }
    for agent in &plan.agents {
        let Some(mirror) = installed.mirrors.iter().find(|m| &m.agent == agent) else {
            return Err(format!("安装后没有找到 {agent} 的副本"));
        };
        if !Path::new(&mirror.path).exists() {
            return Err(format!("安装后副本不存在：{}", mirror.path));
        }
    }
    invalidate_update_cache(&plan.skill);
    let paths: Vec<String> = vec![installed.dir.clone()];
    let _ = crate::clean::append_audit(job, "ok", &paths, &[], 0);
    Ok(format!(
        "安装 {} 完成（{}）；日志：{}",
        plan.skill,
        plan.agents.join("、"),
        log_path.display()
    ))
}

fn install_args(args: &[String]) -> Result<(String, Option<String>, Vec<String>, bool), String> {
    let mut source = None;
    let mut skill = None;
    let mut agents = Vec::new();
    let mut allow_existing = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--skill" => skill = it.next().cloned(),
            "--agent" => agents.push(it.next().ok_or("--agent 缺少值")?.clone()),
            "--allow-existing" => allow_existing = true,
            other if other.starts_with('-') => return Err(format!("未知选项：{other}")),
            other if source.is_none() => source = Some(other.to_string()),
            other => return Err(format!("多余的参数：{other}")),
        }
    }
    Ok((source.ok_or("缺少技能来源")?, skill, agents, allow_existing))
}

pub fn install_plan_command(args: &[String]) -> ExitCode {
    match install_args(args).and_then(|(source, skill, agents, allow)| {
        plan_install(&source, skill.as_deref(), &agents, allow)
    }) {
        Ok(plan) => match serde_json::to_string_pretty(&plan) {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

pub fn install_start_command(args: &[String]) -> ExitCode {
    let result = (|| -> Result<String, String> {
        let (source, skill, agents, allow) = install_args(args)?;
        let plan = plan_install(&source, skill.as_deref(), &agents, allow)?;
        let mut job = Job::new("skill-install", None, &[]);
        job.target = Some(plan.skill.clone());
        job.params = Some(serde_json::json!({
            "source": source,
            "skill": plan.skill,
            "agents": plan.agents,
            "allow_existing": allow,
        }));
        job.source = "panel".into();
        job.save()?;
        crate::clean::spawn_worker("_skills-run", &job.id)?;
        serde_json::to_string(&serde_json::json!({"job_id": job.id, "plan": plan}))
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn render_plan(plan: &SkillPlan) -> String {
    let mut out = format!(
        "将{} {}（{}）\n影响路径 {} 个：\n",
        if plan.action == "upgrade" {
            "升级"
        } else {
            "删除"
        },
        plan.name,
        plan.source.as_deref().unwrap_or("来源未知"),
        plan.paths.len()
    );
    for path in &plan.paths {
        out.push_str(&format!("  • {path}\n"));
    }
    out.push_str(&format!(
        "副本漂移：{}\n命令：{}\n执行前会快照到 ~/.local/state/devtray/skills-backup/\n",
        if plan.drift {
            "是（已显式允许覆盖）"
        } else {
            "否"
        },
        plan.command
    ));
    out.trim_end().to_string()
}

fn confirm_skill(name: &str) -> Result<bool, String> {
    print!("输入技能名 {name} 确认（其他任意输入取消）：");
    std::io::Write::flush(&mut std::io::stdout()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("读取确认失败：{e}"))?;
    Ok(line.trim() == name)
}

fn write_command(
    action: &str,
    name: &str,
    allow_drift: bool,
    json_flag: bool,
    dry_run: bool,
    yes: bool,
) -> ExitCode {
    let result = (|| -> Result<String, String> {
        let plan = plan_action(action, name, allow_drift)?;
        if dry_run {
            return if json_flag {
                serde_json::to_string_pretty(&plan).map_err(|e| e.to_string())
            } else {
                Ok(render_plan(&plan))
            };
        }
        if !json_flag {
            println!("{}", render_plan(&plan));
        }
        if !yes && !confirm_skill(name)? {
            return Err("已取消".into());
        }
        let mut job = Job::new(&format!("skill-{action}"), None, &plan.paths);
        job.target = Some(name.to_string());
        job.params = Some(serde_json::json!({"allow_drift": allow_drift}));
        job.source = "cli".into();
        job.save()?;
        match execute_job(&mut job) {
            Ok(message) => {
                job.finish_ok(serde_json::json!({"message": message}));
                job.save()?;
                if json_flag {
                    serde_json::to_string_pretty(&serde_json::json!({
                        "job_id": job.id,
                        "message": message,
                        "plan": plan,
                    }))
                    .map_err(|e| e.to_string())
                } else {
                    Ok(message)
                }
            }
            Err(e) => {
                job.finish_error(e.clone());
                let _ = job.save();
                Err(e)
            }
        }
    })();
    match result {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

pub fn command(args: &[String]) -> ExitCode {
    if args.first().map(String::as_str) == Some("open") {
        let dir = crate::config::home_dir().join(".agents").join("skills");
        return match crate::disk::open_path(&dir) {
            Ok(message) => {
                println!("{message}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    let mut json_flag = false;
    let mut check = false;
    let mut refresh = false;
    let mut yes = false;
    let mut dry_run = false;
    let mut allow_drift = false;
    let mut restore_lock = false;
    let mut snapshot = None;
    let mut install_skill = None;
    let mut install_agents: Vec<String> = Vec::new();
    let mut allow_existing = false;
    let mut action = None;
    let mut name = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" => json_flag = true,
            "check" => {
                check = true;
                action = Some("check".to_string());
            }
            "upgrade" | "remove" | "restore" | "install" if action.is_none() => {
                action = Some(arg.clone())
            }
            "--skill" => install_skill = it.next().cloned(),
            "--agent" => {
                let Some(agent) = it.next() else {
                    eprintln!("--agent 缺少值");
                    return ExitCode::FAILURE;
                };
                install_agents.push(agent.clone());
            }
            "--allow-existing" => allow_existing = true,
            "--snapshot" => snapshot = it.next().cloned(),
            "--restore-lock" => restore_lock = true,
            "--refresh" => refresh = true,
            "--yes" | "-y" => yes = true,
            "--dry-run" => dry_run = true,
            "--allow-drift" => allow_drift = true,
            other if other.starts_with('-') => {
                eprintln!("未知选项：{other}");
                return ExitCode::FAILURE;
            }
            other if name.is_none() => name = Some(other.to_string()),
            other => {
                eprintln!("多余的参数：{other}");
                return ExitCode::FAILURE;
            }
        }
    }
    if action.as_deref() == Some("install") {
        let Some(source) = name else {
            eprintln!("缺少技能来源");
            return ExitCode::FAILURE;
        };
        let result = (|| -> Result<String, String> {
            let plan = plan_install(
                &source,
                install_skill.as_deref(),
                &install_agents,
                allow_existing,
            )?;
            if dry_run {
                return if json_flag {
                    serde_json::to_string_pretty(&plan).map_err(|e| e.to_string())
                } else {
                    Ok(render_install_plan(&plan))
                };
            }
            if !json_flag {
                println!("{}", render_install_plan(&plan));
            }
            if !yes && !confirm_skill(&plan.skill)? {
                return Err("已取消".into());
            }
            let mut job = Job::new("skill-install", None, &[]);
            job.target = Some(plan.skill.clone());
            job.params = Some(serde_json::json!({
                "source": source,
                "skill": plan.skill,
                "agents": plan.agents,
                "allow_existing": allow_existing,
            }));
            job.source = "cli".into();
            job.save()?;
            match execute_install(&mut job) {
                Ok(message) => {
                    job.finish_ok(serde_json::json!({"message": message}));
                    job.save()?;
                    if json_flag {
                        serde_json::to_string_pretty(&serde_json::json!({
                            "job_id": job.id,
                            "message": message,
                            "plan": plan,
                        }))
                        .map_err(|e| e.to_string())
                    } else {
                        Ok(message)
                    }
                }
                Err(e) => {
                    job.finish_error(e.clone());
                    let _ = job.save();
                    Err(e)
                }
            }
        })();
        return match result {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    if action.as_deref() == Some("restore") {
        let Some(name) = name else {
            eprintln!("缺少技能名");
            return ExitCode::FAILURE;
        };
        let backups = list_backups(&name);
        if dry_run {
            let text = if json_flag {
                match serde_json::to_string_pretty(&backups) {
                    Ok(text) => text,
                    Err(e) => {
                        eprintln!("{e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                render_backups(&name, &backups)
            };
            println!("{text}");
            return ExitCode::SUCCESS;
        }
        if backups.is_empty() {
            eprintln!("{name} 没有可用快照");
            return ExitCode::FAILURE;
        }
        if !json_flag {
            println!("{}", render_backups(&name, &backups));
        }
        if !yes && !matches!(confirm_skill(&name), Ok(true)) {
            eprintln!("已取消");
            return ExitCode::FAILURE;
        }
        return match restore(&name, snapshot.as_deref(), restore_lock) {
            Ok(message) => {
                println!(
                    "{}",
                    if json_flag {
                        serde_json::json!({"message": message}).to_string()
                    } else {
                        message
                    }
                );
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(action) = action
        && matches!(action.as_str(), "upgrade" | "remove")
    {
        let Some(name) = name else {
            eprintln!("缺少技能名");
            return ExitCode::FAILURE;
        };
        return write_command(&action, &name, allow_drift, json_flag, dry_run, yes);
    }
    if name.is_some() {
        eprintln!("未知操作；可用：check / upgrade / remove / open");
        return ExitCode::FAILURE;
    }
    let inv = if check || refresh {
        match check_updates(refresh) {
            Ok(inv) => inv,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        inventory()
    };
    let text = if json_flag {
        match serde_json::to_string_pretty(&inv) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        render(&inv)
    };
    println!("{text}");
    ExitCode::SUCCESS
}

fn render(inv: &Inventory) -> String {
    let s = &inv.summary;
    let mut out = format!(
        "Skills {} 个（纳管 {} / 来源未知 {} / 目录缺失 {}）\n主存储 {}\nlock {}{}\n副本漂移 {} · 可更新 {}\n\n",
        s.total,
        s.managed,
        s.unmanaged,
        s.missing_dir,
        s.skills_dir,
        s.lock_path,
        if s.lock_ok { "" } else { "（读取失败）" },
        s.drift_mirrors,
        s.update_available
    );
    for skill in &inv.skills {
        let drift = skill.mirrors.iter().any(|m| m.status == "drift");
        out.push_str(&format!(
            "{}  {}\n  来源 {}   版本 {}   安装 {}\n",
            if drift { "⚠" } else { "✓" },
            skill.name,
            skill.source.as_deref().unwrap_or("未知"),
            skill
                .tree_sha
                .as_deref()
                .map(|s| &s[..s.len().min(7)])
                .unwrap_or("—"),
            skill.installed_at.as_deref().unwrap_or("—")
        ));
        out.push_str(&format!(
            "  更新 {}\n",
            match skill.update.as_str() {
                "available" => "有更新",
                "current" => "已是最新",
                "unknown" => "无法检查",
                _ => "未检查",
            }
        ));
        for mirror in &skill.mirrors {
            if mirror.status == "drift" {
                out.push_str(&format!("  漂移：{} ({})\n", mirror.path, mirror.kind));
            }
        }
        for note in &skill.notes {
            out.push_str(&format!("  注意：{note}\n"));
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "devtray-skills-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
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

        fn file(&self, rel: &str, text: &str) {
            let p = self.0.join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(p, text).unwrap();
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn inventory_marks_unmanaged_and_drift() {
        let t = Tmp::new("inv");
        t.file(
            ".agents/skills/alpha/SKILL.md",
            "---\nname: alpha\n---\n# alpha\n",
        );
        t.file(
            ".agents/skills/beta/SKILL.md",
            "---\nname: beta\n---\n# beta\n",
        );
        t.file(
            ".agents/.skill-lock.json",
            r#"{"version":3,"skills":{"alpha":{"source":"o/r","sourceUrl":"https://github.com/o/r","skillPath":"skills/alpha/SKILL.md","skillFolderHash":"0123456789012345678901234567890123456789","installedAt":"2026-01-01T00:00:00Z"}}}"#,
        );
        t.file(
            ".claude/skills/alpha/SKILL.md",
            "---\nname: alpha\n---\n# alpha\n",
        );
        t.file(
            ".claude/skills/beta/SKILL.md",
            "---\nname: beta\n---\n# changed\n",
        );

        let inv = inventory_at(&t.0, &UpdateCache::new());
        assert_eq!(inv.summary.total, 2);
        assert_eq!(inv.summary.managed, 1);
        assert_eq!(inv.summary.unmanaged, 1);
        let alpha = inv.skills.iter().find(|s| s.name == "alpha").unwrap();
        assert!(alpha.managed);
        assert_eq!(alpha.display_name.as_deref(), Some("alpha"));
        assert!(!alpha.mirrors.is_empty());
        assert_ne!(alpha.mirrors[0].status, "drift");
        let beta = inv.skills.iter().find(|s| s.name == "beta").unwrap();
        assert!(!beta.managed);
        if Command::new("diff").arg("--version").output().is_ok() {
            assert_eq!(beta.mirrors[0].status, "drift");
            assert_eq!(inv.summary.drift_mirrors, 1);
        }
    }

    #[test]
    fn folder_sha_matches_nested_and_root_skills() {
        let tree: TreeResponse = serde_json::from_str(
            r#"{"sha":"root-sha","truncated":false,"tree":[
                {"path":"skills","type":"tree","sha":"skills-sha"},
                {"path":"skills/alpha","type":"tree","sha":"alpha-sha"},
                {"path":"skills/alpha/SKILL.md","type":"blob","sha":"blob-sha"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            folder_sha(&tree, "skills/alpha/SKILL.md").as_deref(),
            Some("alpha-sha")
        );
        assert_eq!(folder_sha(&tree, "SKILL.md").as_deref(), Some("root-sha"));
        assert_eq!(folder_sha(&tree, "missing/SKILL.md"), None);
    }

    #[test]
    fn missing_skill_md_is_reported() {
        let t = Tmp::new("bad");
        t.dir(".agents/skills/orphan");
        let inv = inventory_at(&t.0, &UpdateCache::new());
        let orphan = inv.skills.iter().find(|s| s.name == "orphan").unwrap();
        assert!(orphan.notes.iter().any(|n| n.contains("frontmatter")));
    }
}
