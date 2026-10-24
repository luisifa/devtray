//! Bounded, restart-safe log storage shared by the CLI and desktop panel.
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

pub const ARCHIVES: usize = 10;
pub const MAX_BYTES: u64 = 10 * 1024 * 1024;
const SCAN_BYTES: u64 = 2 * 1024 * 1024;

pub fn archive_path(path: &Path, index: usize) -> PathBuf {
    if index == 0 {
        return path.to_path_buf();
    }
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}

pub fn rotate(path: &Path) -> Result<(), String> {
    if !path.exists() || path.metadata().map_err(|e| e.to_string())?.len() == 0 {
        return Ok(());
    }
    let oldest = archive_path(path, ARCHIVES);
    if oldest.exists() {
        fs::remove_file(oldest).map_err(|e| e.to_string())?;
    }
    for i in (0..ARCHIVES).rev() {
        let from = archive_path(path, i);
        if from.exists() {
            fs::rename(from, archive_path(path, i + 1)).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn writer_lock(path: &Path) -> Result<File, String> {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(PathBuf::from(lock_path))
        .map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    Ok(lock)
}

pub fn prepare(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let _lock = writer_lock(path)?;
    rotate(path)?;
    File::create(path).map_err(|e| format!("创建日志失败：{e}"))?;
    Ok(())
}

/// Runs as tmux's pipe child, so collection survives closing/restarting the tray.
pub fn collect(path: &Path, mut input: impl Read, limit: u64) -> Result<(), String> {
    let _lock = writer_lock(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut buf = [0; 8192];
    loop {
        let n = input.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if size >= limit {
            drop(file);
            rotate(path)?;
            file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| e.to_string())?;
            size = 0;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        size += n as u64;
    }
    Ok(())
}

static LEVEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^(?:\[[^\]]*\d[^\]]*\]\s*|\d{4}-\d{2}-\d{2}[T ][\d:.+Z-]+\s*|\d{2}:\d{2}:\d{2}(?:\.\d+)?\s*)?\[?\s*(trace|debug|info|warn(?:ing)?|error|fatal|panic(?:ked)?)\b"
).unwrap()
});

pub fn level(text: &str) -> Option<&'static str> {
    fn normal(s: &str) -> Option<&'static str> {
        match s.to_ascii_lowercase().as_str() {
            "trace" => Some("trace"),
            "debug" => Some("debug"),
            "info" => Some("info"),
            "warn" | "warning" => Some("warn"),
            "error" | "fatal" | "panic" | "panicked" => Some("error"),
            _ => None,
        }
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        for key in ["level", "severity", "log.level", "levelname"] {
            if let Some(value) = value.get(key) {
                if let Some(s) = value.as_str().and_then(normal) {
                    return Some(s);
                }
                if let Some(n) = value.as_u64() {
                    return match n {
                        10 => Some("trace"),
                        20 => Some("debug"),
                        30 => Some("info"),
                        40 => Some("warn"),
                        50 | 60 => Some("error"),
                        _ => None,
                    };
                }
            }
        }
    }
    if text.starts_with("thread '") && text.contains("panicked")
        || text.starts_with("Traceback (most recent call last)")
    {
        return Some("error");
    }
    LEVEL.captures(text.trim()).and_then(|c| normal(&c[1]))
}

#[derive(Default)]
pub struct Query {
    pub levels: Vec<String>,
    pub search: String,
    pub history: bool,
    pub run: usize,
    pub lines: usize,
}

#[derive(Serialize)]
pub struct Entry {
    pub level: String,
    pub text: String,
    pub source: usize,
}
#[derive(Serialize)]
pub struct LogFile {
    pub index: usize,
    pub bytes: u64,
    pub modified: u64,
}
#[derive(Serialize)]
pub struct ResultSet {
    pub entries: Vec<Entry>,
    pub files: Vec<LogFile>,
    pub truncated: bool,
}

pub fn query(path: &Path, q: &Query) -> Result<ResultSet, String> {
    let mut result = ResultSet {
        entries: vec![],
        files: vec![],
        truncated: false,
    };
    let needle = q.search.to_lowercase();
    let limit = q.lines.min(10_000);
    let mut entries = VecDeque::with_capacity(limit);
    for i in (0..=ARCHIVES).rev() {
        let p = archive_path(path, i);
        let meta = match fs::metadata(&p) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("读取日志失败 {}：{e}", p.display())),
        };
        result.files.push(LogFile {
            index: i,
            bytes: meta.len(),
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0),
        });
        if !q.history && i != q.run {
            continue;
        }
        let mut f = File::open(&p).map_err(|e| e.to_string())?;
        let offset = meta.len().saturating_sub(SCAN_BYTES);
        f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
        let mut bytes = vec![];
        f.take(SCAN_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let text = crate::cli::strip_ansi(&String::from_utf8_lossy(&bytes));
        let mut previous = "other";
        for (line_no, line) in text.lines().enumerate() {
            if offset > 0 && line_no == 0 {
                result.truncated = true;
                continue;
            }
            let severity = level(line).unwrap_or_else(|| {
                if line.starts_with([' ', '\t']) {
                    previous
                } else {
                    "other"
                }
            });
            previous = severity;
            if !q.levels.is_empty() && !q.levels.iter().any(|l| l == severity) {
                continue;
            }
            if !needle.is_empty() && !line.to_lowercase().contains(&needle) {
                continue;
            }
            if entries.len() == limit {
                entries.pop_front();
                result.truncated = true;
            }
            if limit == 0 {
                continue;
            }
            entries.push_back(Entry {
                level: severity.into(),
                text: line.trim_end_matches('\r').into(),
                source: i,
            });
        }
    }
    result.entries = entries.into_iter().collect();
    result.files.reverse();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_levels_without_message_false_positives() {
        for (text, want) in [
            ("2026-10-23T09:20:00.123Z WARN db: retry", Some("warn")),
            ("warning: unused variable", Some("warn")),
            ("error[E0123]: failed", Some("error")),
            (r#"{"level":40,"msg":"retry"}"#, Some("warn")),
            ("INFO handle error in request", Some("info")),
            ("there was an error yesterday", None),
        ] {
            assert_eq!(level(text), want, "{text}");
        }
    }
    #[test]
    fn history_survives_reopen_and_rotates_with_limit() {
        let dir = std::env::temp_dir().join(format!("devtray-logs-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("service.log");
        prepare(&p).unwrap();
        collect(&p, &b"INFO hello\nWARN old\n  detail\n"[..], MAX_BYTES).unwrap();
        prepare(&p).unwrap();
        collect(&p, &b"ERROR new\n"[..], MAX_BYTES).unwrap();
        let q = Query {
            history: true,
            levels: vec!["warn".into()],
            lines: 80,
            ..Default::default()
        };
        let found = query(&p, &q).unwrap();
        assert_eq!(found.entries.len(), 2);
        assert_eq!(found.entries[0].text, "WARN old");
        for _ in 0..ARCHIVES + 2 {
            prepare(&p).unwrap();
            fs::write(&p, "INFO x\n").unwrap();
        }
        assert_eq!(query(&p, &q).unwrap().files.len(), ARCHIVES + 1);
        collect(&p, &vec![b'x'; 20_000][..], 8192).unwrap();
        assert!(fs::metadata(&p).unwrap().len() <= 8192);
        fs::remove_dir_all(dir).unwrap();
    }
}
