//! 后台任务：清理 / 安装 / 升级统一成 job，面板与 MCP 都按 id 查询状态。
//!
//! 状态落盘（`~/.local/state/devtray/jobs/<id>.json`），因此面板关闭、MCP 断连都不影响
//! 任务本身；写入用临时文件 + rename，避免读到半截 JSON。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    PendingApproval,
    Running,
    Done,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub action: String,
    pub state: JobState,
    #[serde(default = "default_source")]
    pub source: String,
    pub project: Option<String>,
    /// 技能操作的目标名等结构化参数
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
    pub paths: Vec<String>,
    #[serde(default)]
    pub total: u64,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    /// 0.0 – 1.0；未知总量时保持 0.0
    pub progress: f32,
    pub message: Option<String>,
    pub error: Option<String>,
    pub result: Option<serde_json::Value>,
}

impl Job {
    pub fn new(action: &str, project: Option<&str>, paths: &[String]) -> Self {
        Self {
            id: new_id(),
            action: action.into(),
            state: JobState::PendingApproval,
            source: "cli".into(),
            project: project.map(str::to_string),
            target: None,
            params: None,
            paths: paths.to_vec(),
            total: 0,
            created_at: now_secs(),
            started_at: None,
            finished_at: None,
            progress: 0.0,
            message: None,
            error: None,
            result: None,
        }
    }

    pub fn mark_running(&mut self, message: Option<&str>) {
        self.state = JobState::Running;
        self.started_at = Some(now_secs());
        if let Some(message) = message {
            self.message = Some(message.to_string());
        }
    }

    pub fn finish_ok(&mut self, result: serde_json::Value) {
        self.state = JobState::Done;
        self.finished_at = Some(now_secs());
        self.progress = 1.0;
        self.result = Some(result);
        self.message = None;
    }

    pub fn finish_error(&mut self, error: String) {
        self.state = JobState::Error;
        self.finished_at = Some(now_secs());
        self.error = Some(error);
    }

    pub fn cancel(&mut self) {
        self.state = JobState::Cancelled;
        self.finished_at = Some(now_secs());
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&dir())
    }

    pub fn save_to(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建任务目录失败：{e}"))?;
        let path = dir.join(format!("{}.json", self.id));
        let tmp = dir.join(format!("{}.json.tmp", self.id));
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).map_err(|e| format!("写任务文件失败：{e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("提交任务文件失败：{e}"))?;
        Ok(())
    }

    pub fn load(id: &str) -> Result<Self, String> {
        Self::load_from(&dir(), id)
    }

    pub fn load_from(dir: &Path, id: &str) -> Result<Self, String> {
        let path = dir.join(format!("{id}.json"));
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("读不到任务 {}：{e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("任务 JSON 损坏：{e}"))
    }
}

pub fn dir() -> PathBuf {
    crate::config::state_dir().join("jobs")
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn default_source() -> String {
    "cli".into()
}

fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{}-{}-{}", now_secs(), std::process::id(), nanos)
}

/// 面板 / MCP 读取任务状态；`--cancel` 仅对尚未开始的任务有效。
pub fn command(args: &[String]) -> std::process::ExitCode {
    let mut id = None;
    let mut cancel = false;
    for arg in args {
        match arg.as_str() {
            "--cancel" => cancel = true,
            other if other.starts_with('-') => {
                eprintln!("未知选项：{other}");
                return std::process::ExitCode::FAILURE;
            }
            other if id.is_none() => id = Some(other.to_string()),
            other => {
                eprintln!("多余的参数：{other}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }
    let result = (|| -> Result<String, String> {
        let id = id.ok_or("用法：devtray _job <job-id> [--cancel]")?;
        let mut job = Job::load(&id)?;
        if cancel {
            if job.state != JobState::PendingApproval {
                return Err(format!("任务 {} 已在执行或已结束，无法取消", job.id));
            }
            job.cancel();
            job.save()?;
        }
        serde_json::to_string_pretty(&job).map_err(|e| e.to_string())
    })();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_roundtrip_and_states() {
        let dir =
            std::env::temp_dir().join(format!("devtray-job-{}-{}", std::process::id(), now_secs()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut job = Job::new("clean", Some("p"), &["a/b".into()]);
        assert_eq!(job.state, JobState::PendingApproval);
        job.save_to(&dir).unwrap();

        let loaded = Job::load_from(&dir, &job.id).unwrap();
        assert_eq!(loaded.action, "clean");
        assert_eq!(loaded.paths, vec!["a/b".to_string()]);

        job.mark_running(Some("删除中"));
        job.progress = 0.5;
        job.save_to(&dir).unwrap();
        assert_eq!(
            Job::load_from(&dir, &job.id).unwrap().state,
            JobState::Running
        );

        job.finish_ok(serde_json::json!({"removed": 1}));
        job.save_to(&dir).unwrap();
        let done = Job::load_from(&dir, &job.id).unwrap();
        assert_eq!(done.state, JobState::Done);
        assert_eq!(done.progress, 1.0);
        assert_eq!(done.result.unwrap()["removed"], 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
