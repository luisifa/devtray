//! Read-only local inventory. No configuration, tmux, WSL or network is required.
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Tool {
    pub id: String,
    pub status: String,
    pub path: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct Inventory {
    pub schema_version: u32,
    pub environment_id: &'static str,
    pub platform: &'static str,
    pub architecture: &'static str,
    pub collected_at: u64,
    pub capabilities: serde_json::Value,
    pub tools: Vec<Tool>,
}

pub fn collect() -> Inventory {
    let mut specs = vec![
        ("git", "git"),
        ("rust", "rustc"),
        ("cargo", "cargo"),
        ("node", "node"),
        ("bun", "bun"),
    ];
    specs.extend(crate::agents::inventory_specs());
    // Four bounded workers; one broken tool does not block unrelated probes.
    let tools = std::thread::scope(|scope| {
        let handles: Vec<_> = specs
            .chunks(specs.len().div_ceil(4))
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|(id, bin)| inspect(id, bin))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("inventory worker"))
            .collect()
    });
    Inventory {
        schema_version: 1,
        environment_id: "local",
        platform: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        collected_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        capabilities: crate::platform::capabilities()["capabilities"].clone(),
        tools,
    }
}

fn inspect(id: &str, bin: &str) -> Tool {
    let primary = crate::platform::find_executable(bin);
    // Agent-specific fallback locations are evidence, not a replacement for PATH.
    let path = primary
        .clone()
        .or_else(|| crate::platform::find_executable_in(bin, crate::agents::search_dirs()));
    let mut tool = Tool {
        id: id.into(),
        status: "not_found".into(),
        path: None,
        version: None,
        source: None,
        reason: None,
    };
    let Some(path) = path else { return tool };
    tool.path = Some(path.display().to_string());
    tool.source = Some(
        if primary.is_some() {
            "path"
        } else {
            "user_directory"
        }
        .into(),
    );
    match probe(&path, Duration::from_secs(2)) {
        Ok(text) => {
            tool.version = crate::agents::parse_version(&text);
            tool.status = if tool.version.is_some() {
                "found"
            } else {
                "probe_failed"
            }
            .into();
            if tool.version.is_none() {
                tool.reason = Some("无法解析版本输出".into());
            }
        }
        Err(reason) => {
            tool.status = "probe_failed".into();
            tool.reason = Some(reason);
        }
    }
    tool
}

// Temp output avoids inherited pipe handles blocking a completed probe. Poll the
// output size as well as the deadline; never return or retain arbitrary CLI output.
struct ProbeFile {
    file: Option<std::fs::File>,
    path: PathBuf,
}
impl Drop for ProbeFile {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn probe(path: &Path, timeout: Duration) -> Result<String, String> {
    #[cfg(windows)]
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
    {
        return Err("已找到 Windows 包装脚本；尚未支持该脚本的版本探测".into());
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = std::env::temp_dir().join(format!(
        "devtray-inventory-{}-{stamp}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&name)
        .map_err(|e| format!("无法创建探测输出：{e}"))?;
    let mut output = ProbeFile {
        file: Some(file),
        path: name,
    };
    let file = output.file.as_ref().unwrap();
    let mut child = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(file.try_clone().map_err(|e| e.to_string())?)
        .stderr(file.try_clone().map_err(|e| e.to_string())?)
        .spawn()
        .map_err(|e| format!("无法运行版本探测：{e}"))?;
    let deadline = Instant::now() + timeout;
    let result = loop {
        match file.metadata() {
            Ok(meta) if meta.len() > 65536 => break Err("版本输出超过 64 KiB".into()),
            Err(e) => break Err(e.to_string()),
            _ => {}
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(format!("版本探测退出：{status}"))
                };
            }
            Err(e) => break Err(e.to_string()),
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            break Err("版本探测超时".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result?;
    let file = output.file.as_mut().unwrap();
    file.rewind().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("版本输出超过 64 KiB".into());
    }
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

pub fn command(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a != "--json") {
        eprintln!("用法：devtray inventory [--json]");
        return ExitCode::from(2);
    }
    let value = collect();
    if args.iter().any(|a| a == "--json") {
        println!("{}", serde_json::to_string(&value).unwrap());
    } else {
        println!("本机环境 · {} / {}", value.platform, value.architecture);
        for tool in value.tools {
            println!(
                "{}  {}  {}\n  {}{}",
                tool.id,
                tool.status,
                tool.version.as_deref().unwrap_or("—"),
                tool.path.as_deref().unwrap_or("未找到"),
                tool.reason
                    .as_ref()
                    .map(|r| format!(" · {r}"))
                    .unwrap_or_default()
            );
        }
    }
    ExitCode::SUCCESS
}
