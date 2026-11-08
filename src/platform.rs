//! Host primitives only. Optional WSL environments must never be inferred here.
use std::path::{Path, PathBuf};

pub fn home_dir_option() -> Option<PathBuf> {
    #[cfg(windows)]
    let key = "USERPROFILE";
    #[cfg(not(windows))]
    let key = "HOME";
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub fn home_dir() -> PathBuf {
    home_dir_option().unwrap_or_default()
}

#[cfg(windows)]
pub fn user_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join("AppData/Local"))
        .join("DevTray")
}

pub fn find_executable(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    find_executable_in(program, std::env::split_paths(&path).collect())
}

pub fn find_executable_in(program: &str, dirs: Vec<PathBuf>) -> Option<PathBuf> {
    #[cfg(windows)]
    let extensions: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    #[cfg(not(windows))]
    let extensions: Vec<String> = vec![];
    #[cfg(windows)]
    let names: Vec<String> = if Path::new(program).extension().is_none() {
        // npm installs an extensionless Unix script beside its .cmd wrapper.
        // Windows must choose PATHEXT candidates, not the Unix script.
        extensions
            .iter()
            .map(|ext| format!("{program}{ext}"))
            .collect()
    } else {
        vec![program.to_owned()]
    };
    #[cfg(not(windows))]
    let names = {
        let _ = extensions;
        vec![program.to_owned()]
    };
    dirs.iter()
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| is_executable(path))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(windows)]
    {
        true
    }
}

pub fn symlink(source: impl AsRef<Path>, target: impl AsRef<Path>) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target)
    }
    #[cfg(windows)]
    {
        // Copy/restore must not delete an existing mirror and then discover that
        // restoring its link needs privileges. Enable only with a planned Windows writer.
        let _ = (source, target);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Windows Skills 符号链接写入尚未实现",
        ))
    }
}

pub fn capabilities() -> serde_json::Value {
    let unix = cfg!(unix);
    serde_json::json!({
        "schema_version": 1,
        "environment_id": "local",
        "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "capabilities": {
            "agent_inventory": true,
            "tool_inventory": true,
            "service_control": unix && find_executable("tmux").is_some(),
            "terminal_attach": unix && find_executable("tmux").is_some(),
            "disk_scan": unix,
            "skills_write": unix,
            "disk_clean": cfg!(target_os = "linux"),
            "wsl_environments": false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_directory_is_not_executable() {
        assert!(find_executable_in("devtray-no-such-tool", vec![std::env::temp_dir()]).is_none());
        assert!(!is_executable(&std::env::temp_dir()));
    }
    #[test]
    fn capabilities_are_explicit_and_local() {
        let value = capabilities();
        assert_eq!(value["environment_id"], "local");
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["capabilities"]["wsl_environments"], false);
    }
}
