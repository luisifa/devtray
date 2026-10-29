//! Durable, per-run alert acknowledgements shared by CLI, Qt, and tray.
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use crate::tmux::PaneInfo;
use serde::{Deserialize, Serialize};

fn digest(value: &impl Hash) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hash);
    format!("{:016x}", hash.finish())
}

pub fn store_path(config: &Path) -> PathBuf {
    crate::config::state_dir().join("alerts").join(format!(
        "{}.json",
        digest(&crate::config::absolute(config.to_path_buf()))
    ))
}

pub fn failure_token(pane: &PaneInfo) -> Option<String> {
    let code = pane.status?;
    (pane.dead && code != 0).then(|| format!("exit={code};pid={}", pane.pid))
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Acknowledgements {
    pub services: HashMap<String, String>,
    pub config_error: Option<String>,
}

impl Acknowledgements {
    pub fn load_at(path: &Path) -> Self {
        Self::read_at(path).unwrap_or_default()
    }

    fn read_at(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(data) => serde_json::from_slice(&data).map_err(|e| format!("读取已读记录失败：{e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("读取已读记录失败：{e}")),
        }
    }

    pub fn contains(&self, session: &str, token: &str) -> bool {
        self.services
            .get(session)
            .is_some_and(|saved| saved == token)
    }

    pub fn config_matches(&self, error: &str) -> bool {
        self.config_error.as_deref() == Some(digest(&error).as_str())
    }
}

pub fn record_at(
    path: &Path,
    failures: &HashMap<String, String>,
    config_error: Option<&str>,
) -> Result<(), String> {
    let _guard = crate::ops::acquire_at(&path.with_extension("lock"), Duration::from_secs(2))?;
    let mut acknowledgements = Acknowledgements::read_at(path)?;
    acknowledgements.services.extend(failures.clone());
    if let Some(error) = config_error {
        acknowledgements.config_error = Some(digest(&error));
    }
    let data = serde_json::to_vec(&acknowledgements).map_err(|e| e.to_string())?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, data).map_err(|e| format!("保存已读记录失败：{e}"))?;
    std::fs::rename(&temporary, path).map_err(|e| format!("提交已读记录失败：{e}"))
}

pub fn acknowledge_target(
    config: &Path,
    target: &crate::config::Target,
    expected: Option<&str>,
) -> Result<(), String> {
    let snapshot = crate::tmux::snapshot_result()?;
    let token = snapshot
        .get(&target.session)
        .and_then(failure_token)
        .ok_or("服务当前没有待确认的异常退出，请刷新状态")?;
    if expected.is_some_and(|value| value != token) {
        return Err("服务已进入新的一轮运行状态，请刷新后再确认，未标记新的异常".into());
    }
    record_at(
        &store_path(config),
        &HashMap::from([(target.session.clone(), token)]),
        None,
    )
}

pub fn dismiss(config: &Path) -> Result<(), String> {
    match crate::config::Config::load(config).and_then(|cfg| cfg.targets()) {
        Ok(targets) => {
            let snapshot = crate::tmux::snapshot_result()?;
            let failures = targets
                .iter()
                .filter_map(|target| {
                    let token = snapshot.get(&target.session).and_then(failure_token)?;
                    Some((target.session.clone(), token))
                })
                .collect();
            record_at(&store_path(config), &failures, None)
        }
        Err(error) => record_at(&store_path(config), &HashMap::new(), Some(&error)),
    }
}

pub fn command(args: &[String]) -> ExitCode {
    let result = (|| {
        let spec = args.first().ok_or("用法：devtray ack <项目/服务>")?;
        let expected = match args.len() {
            1 => None,
            3 if args[1] == "--failure-token" => Some(args[2].as_str()),
            _ => return Err("用法：devtray ack <项目/服务> [--failure-token TOKEN]".into()),
        };
        let ctx = crate::cli::Ctx::load()?;
        acknowledge_target(&crate::config_path(), ctx.resolve(spec)?, expected)?;
        Ok::<_, String>(serde_json::json!({"acknowledged": true,"target": spec}))
    })();
    match result {
        Ok(data) => {
            println!("{data}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_ack_matches_only_this_failed_run_and_preserves_other_services() {
        let path = std::env::temp_dir().join(format!("devtray-acks-{}.json", std::process::id()));
        let pane = PaneInfo {
            dead: true,
            status: Some(1),
            pid: 4242,
        };
        let token = failure_token(&pane).unwrap();
        record_at(&path, &HashMap::from([("web".into(), token.clone())]), None).unwrap();
        record_at(
            &path,
            &HashMap::from([("api".into(), "other-run".into())]),
            Some("bad configuration"),
        )
        .unwrap();
        let restored = Acknowledgements::load_at(&path);
        assert!(restored.contains("web", &token));
        assert!(restored.contains("api", "other-run"));
        assert!(restored.config_matches("bad configuration"));
        assert!(!restored.config_matches("new bad configuration"));
        let new_token = failure_token(&PaneInfo { pid: 5252, ..pane }).unwrap();
        assert!(!restored.contains("web", &new_token));
        assert_eq!(
            failure_token(&PaneInfo {
                dead: false,
                ..pane
            }),
            None
        );
        std::fs::remove_file(&path).unwrap();
        let _ = std::fs::remove_file(path.with_extension("lock"));
    }
}
