//! MCP server（stdio，换行分隔的 JSON-RPC 2.0）。
//!
//! 只实现 agent 真正需要的那几个方法：`initialize` / `tools/list` / `tools/call` / `ping`。
//! 工具本体全部是 `cli` 底座的一层薄壳，所以「agent 通过 MCP 做的」和「你手敲 devtray
//! start 做的」完全等价。
//!
//! 注意：**stdout 只许写协议消息**，任何日志走 stderr，否则客户端会解析失败。

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::cli::{self, Ctx};
use crate::config::Config;
use crate::disk;
use crate::jobs::{Job, JobState};

const PROTOCOL_FALLBACK: &str = "2025-06-18";

pub fn serve() -> i32 {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[devtray mcp] 收到无法解析的行：{e}");
                continue;
            }
        };
        if let Some(resp) = handle(&msg)
            && (writeln!(out, "{resp}").is_err() || out.flush().is_err())
        {
            break; // 客户端走了
        }
    }
    0
}

fn handle(msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str)?;
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    let result: Result<Value, (i64, String)> = match method {
        // 回显客户端要求的协议版本，避免版本协商把连接谈崩
        "initialize" => Ok(json!({
            "protocolVersion": params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_FALLBACK),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "devtray", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools()),
        "tools/call" => Ok(call_tool(&params)),
        // 通知：不回复
        m if m.starts_with("notifications/") => return None,
        other => Err((-32601, format!("不支持的方法：{other}"))),
    };

    let id = id?; // 没有 id = 通知，即使方法认识也不回复
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    })
}

fn tools() -> Value {
    let target = json!({
        "type": "string",
        "description": "服务标识，形如 \"simple-oa/backend\"；服务名唯一时也可只写 \"backend\""
    });
    json!({
        "tools": [
            {
                "name": "devtray_status",
                "description": "列出 devtray 托管的所有开发服务及其状态。agent 在启停前应先调它确认目标名。state 取值：not_started / running / starting / ready / exited_ok / exited_failed / exited_signal。",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            },
            {
                "name": "devtray_logs",
                "description": "读取某个服务的日志尾部。编译错误、panic、端口占用都会在这里；服务已停止或崩溃也照样能读（日志落盘保留下来的）。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "target": target,
                        "lines": { "type": "integer", "description": "返回最后多少行，默认 80", "minimum": 1, "maximum": 5000 },
                        "level": { "type": "string", "description": "逗号分隔级别：error,warn,info,debug,trace,other" },
                        "search": { "type": "string", "description": "不区分大小写的内容搜索" },
                        "history": { "type": "boolean", "description": "包含跨重启和大小轮转的历史日志" }
                    },
                    "required": ["target"],
                    "additionalProperties": false
                }
            },
            {
                "name": "devtray_open_preview",
                "description": "用用户机器上的默认浏览器打开某个服务的预览地址（前端 dev server 一般是 http://localhost:<port>/）。地址优先取配置的 url，其次从该服务日志里识别，最后用 port 拼；识别不到会返回错误说明原因。会在用户桌面上弹出浏览器窗口，只在用户想看页面时调用。",
                "inputSchema": {
                    "type": "object",
                    "properties": { "target": target },
                    "required": ["target"],
                    "additionalProperties": false
                }
            },
            {
                "name": "devtray_start",
                "description": "启动一个服务（在 tmux 里跑）。若已在运行会先杀掉再重建。启动后返回该服务的新状态；cargo 首次编译可能要几十秒，此时状态是 starting，需要用 status 再看。",
                "inputSchema": {
                    "type": "object",
                    "properties": { "target": target },
                    "required": ["target"],
                    "additionalProperties": false
                }
            },
            {
                "name": "devtray_stop",
                "description": "停止一个服务（先发 SIGINT 等 3 秒，不退再杀会话）。会中断正在跑的服务，确认目标再调。",
                "inputSchema": {
                    "type": "object",
                    "properties": { "target": target },
                    "required": ["target"],
                    "additionalProperties": false
                }
            },
            {
                "name": "devtray_restart",
                "description": "重启一个服务（等价 stop + start）。",
                "inputSchema": {
                    "type": "object",
                    "properties": { "target": target },
                    "required": ["target"],
                    "additionalProperties": false
                }
            },
            {
                "name": "devtray_disk",
                "description": "读取项目构建目录的扫描缓存（target / node_modules / .venv 等）：分配空间、逻辑大小、是否可清理。只读，不触发新扫描；缓存为空时提示用户先在资源页扫描。",
                "inputSchema": {
                    "type": "object",
                    "properties": { "project": { "type": "string", "description": "可选，项目名；省略返回全部项目" } },
                    "additionalProperties": false
                }
            },
            {
                "name": "devtray_job",
                "description": "查询后台任务（清理）的状态、进度、结果；cancel=true 只能取消尚未开始的 pending_approval 任务。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "job_id": { "type": "string", "description": "创建任务时返回的 job_id" },
                        "cancel": { "type": "boolean", "description": "仅在任务尚未开始时取消" }
                    },
                    "required": ["job_id"],
                    "additionalProperties": false
                }
            },
            {
                "name": "devtray_clean",
                "description": "请求清理项目里满足证据链的构建目录。只创建后台任务：默认由用户在桌面 yad 弹窗里批准（mcp_writes=ask），60 秒无响应视为拒绝；返回 job_id 后必须用 devtray_job 轮询结果。不能删除运行中服务所在目录、含挂载点或受 git 跟踪的目录。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": { "type": "string", "description": "项目名" },
                        "kinds": { "type": "array", "items": { "type": "string" }, "description": "目录名，如 node_modules、target、.venv" },
                        "paths": { "type": "array", "items": { "type": "string" }, "description": "绝对路径；与 kinds 二选一" },
                        "all": { "type": "boolean", "description": "清理该项目全部可删候选" }
                    },
                    "required": ["project"],
                    "additionalProperties": false
                }
            }
        ]
    })
}

fn call_tool(params: &Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error_content("tools/call 缺少 name".into());
    };
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    match run_tool(name, &args) {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        // 工具层面的失败用 isError 返回，让 agent 读到原因而不是连接报错
        Err(e) => error_content(e),
    }
}

fn error_content(message: String) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true
    })
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn run_tool(name: &str, args: &Value) -> Result<String, String> {
    match name {
        "devtray_disk" => return disk_tool(args),
        "devtray_job" => return job_tool(args),
        "devtray_clean" => return clean_tool(args),
        _ => {}
    }

    let ctx = Ctx::load()?;

    if name == "devtray_status" {
        let statuses = ctx.statuses();
        let mut body: Value =
            serde_json::from_str(&cli::statuses_json(&statuses)).map_err(|e| e.to_string())?;
        if let Some(advice) = cli::advice(&statuses) {
            body["advice"] = json!(advice);
        }
        return serde_json::to_string_pretty(&body).map_err(|e| e.to_string());
    }

    let target_spec = arg(args, "target").ok_or_else(|| {
        format!(
            "缺少 target 参数。可用服务：{}",
            ctx.list_names().join("、")
        )
    })?;
    let t = ctx.resolve(target_spec)?;

    match name {
        "devtray_logs" => {
            let lines = args
                .get("lines")
                .and_then(Value::as_u64)
                .unwrap_or(80)
                .clamp(1, 5000) as usize;
            if args.get("level").is_none()
                && args.get("search").is_none()
                && args.get("history").is_none()
            {
                return cli::logs(t, lines);
            }
            let levels: Vec<String> = arg(args, "level")
                .unwrap_or("")
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_lowercase())
                .collect();
            if levels.iter().any(|s| {
                !["trace", "debug", "info", "warn", "error", "other"].contains(&s.as_str())
            }) {
                return Err("未知日志级别".into());
            }
            let result = crate::logs::query(
                &t.log_path,
                &crate::logs::Query {
                    lines,
                    levels,
                    search: arg(args, "search").unwrap_or("").into(),
                    history: args
                        .get("history")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    ..Default::default()
                },
            )?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "devtray_open_preview" => cli::open_preview(t),
        "devtray_start" => {
            let msg = cli::start(t)?;
            Ok(format!("{msg}\n{}", after(&ctx, &t.session)))
        }
        "devtray_stop" => {
            let msg = cli::stop(t)?;
            Ok(format!("{msg}\n{}", after(&ctx, &t.session)))
        }
        "devtray_restart" => {
            let msg = cli::restart(t)?;
            Ok(format!("{msg}\n{}", after(&ctx, &t.session)))
        }
        other => Err(format!("未知工具：{other}")),
    }
}

fn config_file() -> std::path::PathBuf {
    std::env::var("DEVTRAY_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| crate::config::default_config_path())
}

fn string_array(args: &Value, key: &str) -> Result<Vec<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("{key} 必须是字符串数组"))
            })
            .collect(),
        Some(_) => Err(format!("{key} 必须是字符串数组")),
    }
}

fn disk_tool(args: &Value) -> Result<String, String> {
    let cfg = Config::load(&config_file())?;
    let roots = cfg.project_roots()?;
    let project = arg(args, "project");
    if let Some(name) = project
        && !roots.iter().any(|r| r.project == name)
    {
        return Err(format!(
            "找不到项目 {name:?}；可用：{}",
            roots
                .iter()
                .map(|r| r.project.as_str())
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    let projects: Vec<disk::ProjectUsage> = disk::cached_all()
        .into_iter()
        .filter(|u| {
            let known = roots
                .iter()
                .any(|r| r.project == u.project && r.root.display().to_string() == u.root);
            known && project.map(|p| u.project == p).unwrap_or(true)
        })
        .collect();
    let hint = if projects.is_empty() {
        "尚无扫描缓存：请用户在资源页运行扫描，或执行 devtray disk --refresh"
    } else {
        ""
    };
    serde_json::to_string_pretty(&json!({
        "projects": projects,
        "scanned": !projects.is_empty(),
        "hint": hint,
    }))
    .map_err(|e| e.to_string())
}

fn job_tool(args: &Value) -> Result<String, String> {
    let id = arg(args, "job_id").ok_or("缺少 job_id")?;
    let mut job = Job::load(id)?;
    if args.get("cancel").and_then(Value::as_bool).unwrap_or(false) {
        if job.state != JobState::PendingApproval {
            return Err(format!("任务 {} 已在执行或已结束，无法取消", job.id));
        }
        job.cancel();
        job.error = Some("agent 取消".into());
        job.save()?;
    }
    serde_json::to_string_pretty(&job).map_err(|e| e.to_string())
}

fn clean_tool(args: &Value) -> Result<String, String> {
    let cfg = Config::load(&config_file())?;
    let mode = cfg.settings.mcp_writes.to_lowercase();
    if mode == "deny" {
        return Err("MCP 写操作已被配置禁用（mcp_writes = deny）".into());
    }
    let project_name = arg(args, "project").ok_or("缺少 project")?;
    let kinds = string_array(args, "kinds")?;
    let paths: Vec<std::path::PathBuf> = string_array(args, "paths")?
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect();
    let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
    if kinds.is_empty() && paths.is_empty() && !all {
        return Err("必须提供 kinds、paths 或 all 之一".into());
    }

    let ctx = crate::clean::Context::load()?;
    let project = ctx
        .roots
        .iter()
        .find(|r| r.project == project_name)
        .cloned()
        .ok_or_else(|| {
            format!(
                "找不到项目 {project_name:?}；可用：{}",
                ctx.roots
                    .iter()
                    .map(|r| r.project.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            )
        })?;
    let candidates = crate::clean::prepare_candidates(&project, &ctx, &kinds, &paths, all)?;
    let job_paths: Vec<String> = candidates.iter().map(|c| c.path.clone()).collect();
    let total: u64 = candidates.iter().map(|c| c.allocated).sum();
    let ids: Vec<String> = candidates.iter().map(|c| c.relative.clone()).collect();

    let mut job = Job::new("clean", Some(&project.project), &job_paths);
    job.source = "mcp".into();
    job.total = total;
    job.save()?;

    if mode == "allow" {
        job.mark_running(Some("mcp_writes = allow"));
        job.save()?;
        crate::clean::spawn_worker("_clean", &job.id)?;
    } else {
        crate::clean::spawn_worker("_approve", &job.id)?;
    }

    serde_json::to_string_pretty(&json!({
        "job_id": job.id,
        "state": if mode == "allow" { "running" } else { "pending_approval" },
        "count": candidates.len(),
        "total": total,
        "paths": ids,
        "next": "用 devtray_job 轮询状态；用户在桌面上批准后才会删除",
    }))
    .map_err(|e| e.to_string())
}

/// 动作做完立刻回报新状态，省掉 agent 再查一次
fn after(ctx: &Ctx, session: &str) -> String {
    ctx.statuses()
        .iter()
        .find(|s| s.session == session)
        .map(|s| s.json())
        .unwrap_or_else(|| r#"{"state":"unknown"}"#.into())
}
