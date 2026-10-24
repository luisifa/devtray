//! Optional Qt desktop panel. The Python source is bundled into the binary.
use crate::{cli, config, form, tmux};
use std::io::Read;
use std::process::{Command, ExitCode};

pub fn launch(target: Option<&str>, toggle: bool) -> Result<(), String> {
    launch_mode(target, toggle, false, None)
}

pub fn quit() -> Result<(), String> {
    launch_mode(None, false, true, None)
}

fn launch_mode(
    target: Option<&str>,
    toggle: bool,
    quit: bool,
    mode: Option<&str>,
) -> Result<(), String> {
    let python = runtime()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&python);
    let quick_source =
        serde_json::to_string(include_str!("../ui/quick_menu.py")).map_err(|e| e.to_string())?;
    let script = format!(
        "import sys, types\n_quick = types.ModuleType('quick_menu')\nexec({quick_source}, _quick.__dict__)\nsys.modules['quick_menu'] = _quick\n{}",
        include_str!("../ui/panel.py")
    );
    cmd.args(["-c", &script]).arg(exe).arg(crate::config_path());
    if let Some(mode) = mode {
        cmd.args(["--mode", mode]);
    }
    if quit {
        cmd.arg("--quit");
    }
    if toggle {
        cmd.arg("--toggle");
    }
    if let Some(t) = target {
        cmd.args(["--target", t]);
    }
    crate::proc::spawn_detached(&mut cmd).map_err(|e| e.to_string())
}

/// Resolve the same interpreter for startup capability checks and UI launches.
pub fn runtime() -> Result<String, String> {
    let python = std::env::var("DEVTRAY_PYTHON").unwrap_or_else(|_| "python3".into());
    let check = Command::new(&python)
        .args(["-c", "from PySide6 import QtWidgets, QtNetwork"])
        .output()
        .map_err(|e| format!("控制面板需要 Python 3 和 PySide6：{e}"))?;
    if !check.status.success() {
        return Err("控制面板需要 PySide6。Arch：sudo pacman -S pyside6；其他发行版请安装 Python 3 的 PySide6。CLI / 托盘仍可正常使用。".into());
    }
    Ok(python)
}

pub fn command(args: &[String]) -> ExitCode {
    let quit = args.iter().any(|s| s == "--quit");
    if !quit {
        crate::ensure_template(&crate::config_path());
    }
    let target = args
        .iter()
        .position(|s| s == "--target")
        .and_then(|i| args.get(i + 1));
    let mode = if args.iter().any(|s| s == "--menu") {
        Some("small")
    } else if let Some(i) = args.iter().position(|s| s == "--mode") {
        match args.get(i + 1).map(String::as_str) {
            Some(mode @ ("small" | "full")) => Some(mode),
            _ => {
                eprintln!("--mode 需要 small 或 full");
                return ExitCode::from(2);
            }
        }
    } else {
        None
    };
    match launch_mode(
        target.map(String::as_str),
        args.iter().any(|s| s == "--toggle"),
        quit,
        mode,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

pub fn data() -> ExitCode {
    let result = (|| -> Result<serde_json::Value, String> {
        let cfg = config::Config::load(&crate::config_path())?;
        let ctx = cli::Ctx {
            targets: cfg.targets()?,
        };
        let mut value: serde_json::Value =
            serde_json::from_str(&cli::statuses_json(&ctx.statuses()))
                .map_err(|e| e.to_string())?;
        for (value, target) in value["services"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(&ctx.targets)
        {
            value["port"] = serde_json::json!(target.service.port);
            value["ready"] = serde_json::json!(target.service.ready);
        }
        value["config"] = serde_json::json!(crate::config_path());
        value["logs_dir"] = serde_json::json!(config::state_dir().join("logs"));
        value["tmux_available"] = serde_json::json!(crate::terminal::which("tmux"));
        value["retention"] = serde_json::json!({"archives": crate::logs::ARCHIVES, "max_bytes": crate::logs::MAX_BYTES});
        Ok(value)
    })();
    output(result)
}

pub fn service_config(spec: Option<&String>) -> ExitCode {
    output((|| {
        let ctx = cli::Ctx::load()?;
        let t = ctx.resolve(spec.ok_or("缺少目标")?)?;
        serde_json::to_value(form::Fields::from_target(t)).map_err(|e| e.to_string())
    })())
}

pub fn save_service() -> ExitCode {
    output((|| {
        let mut input = String::new();
        std::io::stdin()
            .take(1024 * 1024)
            .read_to_string(&mut input)
            .map_err(|e| e.to_string())?;
        let value: serde_json::Value = serde_json::from_str(&input).map_err(|e| e.to_string())?;
        let fields: form::Fields =
            serde_json::from_value(value["fields"].clone()).map_err(|e| e.to_string())?;
        let path = crate::config_path();
        crate::ensure_template(&path);
        let old = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let new = match value["mode"].as_str() {
            Some("add") => form::apply_add(&old, &fields)?,
            Some("edit") => form::apply_edit(&old, &fields)?,
            _ => return Err("未知保存模式".into()),
        };
        form::save_validated(&path, &new)?;
        Ok(serde_json::json!({"message":"配置已保存；运行中的服务需重启应用命令变更"}))
    })())
}

fn output(result: Result<serde_json::Value, String>) -> ExitCode {
    match result {
        Ok(value) => {
            use std::io::Write;
            let _ = writeln!(std::io::stdout(), "{value}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// Preserve terminal access for interactive dev servers.
pub fn terminal(target: &config::Target, settings: &config::Settings) -> Result<(), String> {
    if !tmux::snapshot().contains_key(&target.session) {
        return Err("此服务没有 tmux 会话；可先启动服务，或查看分类日志中的历史记录。".into());
    }
    let base = crate::terminal::resolve(&settings.terminal).ok_or("找不到终端程序")?;
    crate::terminal::spawn_in(&base, &tmux::attach_cmd(target))
}

pub fn terminal_command(spec: Option<&String>) -> ExitCode {
    let result = (|| {
        let cfg = config::Config::load(&crate::config_path())?;
        let ctx = cli::Ctx {
            targets: cfg.targets()?,
        };
        let target = ctx.resolve(spec.ok_or("缺少目标：devtray terminal <项目/服务>")?)?;
        terminal(target, &cfg.settings)?;
        Ok::<_, String>(format!("已打开 {} 的 tmux 实时终端", target.session))
    })();
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
