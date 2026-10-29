//! devtray —— 托盘常驻的开发服务管理器。
//!
//! 三个入口：无参数 = 起托盘；`--status` = 打印一行一服务；`--check` = 自检。

mod agents;
mod alerts;
mod clean;
mod cli;
mod config;
mod disk;
mod form;
mod icons;
mod jobs;
mod logs;
mod mcp;
mod notify;
mod ops;
mod panel;
mod proc;
mod skills;
mod terminal;
mod tmux;
mod tray;

use std::path::PathBuf;
use std::process::ExitCode;

use ksni::blocking::TrayMethods;

use config::{Config, default_config_path, state_dir};
use tray::DevTray;

const TEMPLATE: &str = r#"# devtray 配置 —— 改完自动生效（托盘菜单里也有「重新加载配置」）
[settings]
poll_ms = 2000            # 只影响托盘图标变色与失败通知；菜单内容靠 about_to_show 即时刷新
notify = true
notify_debounce_s = 60
# terminal = "kitty"      # 留空 = 自动探测（$TERMINAL → xdg-terminal-exec → kitty → …）

# 下面是示例，整段取消注释后改成你自己的路径即可。
# 一个项目可以有多条 [[project.service]]；多项目就再写一段 [[project]]。
#
# [[project]]
# name = "my-project"
#
#   [[project.service]]
#   name = "backend"
#   cwd = "~/Workspace/my-project/backend"
#   cmd = "cargo run"
#   autostart = false
#   port = 3060              # 可选：TCP 探活通过才算「就绪」
#   ready = "Listening on"   # 可选：日志正则命中才算「就绪」
#   [project.service.env]
#   RUST_LOG = "debug"
#
#   [[project.service]]
#   name = "frontend"
#   cwd = "~/Workspace/my-project/frontend"
#   cmd = "bun run dev"
#   ready = "ready in"
#   url = "http://localhost:5173/"   # 可选：菜单「打开预览」用它；留空则从日志/port 认
"#;
/// `println!` 在管道被提前关闭（`devtray --status | head -1`）时会 panic，
/// CLI 输出统一走这个，写失败就算了。
macro_rules! say {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

/// 完整操作说明。托盘菜单「使用说明」= 在终端里跑 `devtray help | less -R`。
fn print_manual() {
    say!(
        r#"devtray {} —— 托盘常驻的开发服务管理器（tmux 托管）

────────────────────────────────────────
一、托盘菜单怎么用
────────────────────────────────────────
  · 左键打开 / 收回面板，记忆上次选择的小面板或完整面板；首次默认小面板
  · 右键使用原生菜单，保留项目、实时终端、浏览器、分类日志与批量操作
  · 小面板底部 / 完整面板顶栏切换模式；切换选择在重启后保留
  · 拖动标题移动；默认固定显示，取消固定后点击其他窗口自动收回
  · Esc / 关闭按钮收回浮窗，服务继续运行
  · 两种面板都有实时终端、浏览器和分类日志入口；原生菜单不依赖 PySide6

────────────────────────────────────────
二、状态怎么读
────────────────────────────────────────
  ● 就绪      进程活着，且就绪探测（ready 正则 / port）已通过
  ◌ 启动中    进程活着但探测还没过 —— cargo run 首次编译可能几十秒到几分钟，
              这时不要判定失败，等一会再看
  ○ 未启动    没有对应的 tmux 会话
  ✕ 失败      非零退出（cargo 编译失败是 exit 101），会弹桌面通知并变红图标
  ✓ 已结束    正常退出（exit 0）
  ■ 已停止    被信号杀掉（通常是你自己点了「停止」）

  没配 ready / port 的服务，活着就直接显示「● 运行中」。

────────────────────────────────────────
三、看日志
────────────────────────────────────────
  · 菜单「查看日志」→ 控制面板日志中心（筛选 / 搜索 / 历史 / 复制 / 导出）
    「实时终端」直接 attach 到原来的 tmux，显示原始屏幕、颜色、进度与交互输入
  · 日志同时落盘：{}
    重启归档，按 10 MiB 轮转，保留最近 10 份；停止后仍可查询
  · 命令行：devtray logs <目标> [-n 行数]（会剥掉 ANSI 颜色码）

────────────────────────────────────────
四、加 / 改服务
────────────────────────────────────────
  配置文件：{}
  菜单里有三条路：
    · 「添加服务…」/「编辑这个服务…」= yad 表单（工作目录带目录选择器），
      写回时用保格式的编辑器，**你的中文注释不会被抹掉**；写盘前会先校验，
      填错会弹错误框且不改动配置。
    · 「编辑配置文件」= 用 $EDITOR 直接开 TOML（要改项目名/结构调整时用这个）。
    · 也可以自己手改；三种方式改完都自动生效。
  解析失败会保留上一份可用配置，并把错误挂在菜单第一行。

  一条服务的字段：
    name      服务名（菜单和命令行都用它）
    cwd       工作目录（必须存在）
    cmd       要跑的命令，经 `sh -lc` 执行，所以可写 env 前缀、管道
    autostart 托盘启动时若没在跑是否自动拉起（默认 false）
    ready     就绪正则，匹配「这个服务自己日志里」的输出 —— 优先用它
    port      端口探活（可选；分不清端口上是不是别的进程占着，所以不如 ready 可靠）
    env       环境变量表，会拼成 export K='V'; <cmd>
    log_file  日志路径（留空用默认）

────────────────────────────────────────
五、命令行（agent 也走这一套）
────────────────────────────────────────
  devtray status [--json]         列出服务与状态
  devtray start|stop|restart <目标>
  devtray panel [--toggle]        打开 / 收回桌面控制面板
  devtray panel --mode small|full 切换模式并记住选择
  devtray menu [--toggle]         打开 / 收回小面板
  devtray terminal <目标>         打开原来的 tmux 实时终端
  devtray logs <目标> [-n N] [--level warn,error] [--history]
  devtray open <目标>             用默认浏览器打开预览地址
  devtray edit <目标>             弹表单编辑这个服务（写回时保住注释，先校验再落盘）
  devtray check                   自检（配置/tmux/终端/通知/图标/单元）
  devtray mcp                     以 MCP server（stdio）运行，供 Claude Code / Codex 调用
  devtray help                    本说明

  <目标> 形如 simple-oa/backend；服务名唯一时可以只写 backend。

────────────────────────────────────────
六、打开预览（前端 dev server）
────────────────────────────────────────
  每个服务的子菜单里有「打开预览」，用系统默认浏览器打开它的地址。
  地址按这个优先级确定：
    ① 配置里的 url = "http://localhost:5173/"（最可靠，推荐显式写）
    ② 从该服务的日志里认（优先 localhost/127.0.0.1，会先剥掉 ANSI 颜色码，
       因为 vite 打印的是 http://localhost:\e[1m5173\e[22m/ 这种，颜色码夹在端口中间）
    ③ 用配置里的 port 拼 http://127.0.0.1:<port>/
  都不满足时菜单项是灰的（写「未识别到地址」）。
  命令行等价：devtray open <目标>

────────────────────────────────────────
七、换图标
────────────────────────────────────────
  默认图标内嵌为四态多尺寸位图，修改 assets/devtray*.svg 后生成资源并重新编译。
  标准主题图标同时安装到：
    {}/{{scalable,22x22,32x32,48x48,256x256}}/apps/devtray.svg|png
    …-starting（启动中）、…-error（失败）
  只想换某个状态？配置里改 icon_ok / icon_starting / icon_error。
  填的名字在主题里找不到时会自动退回兜底图标（避免宿主画成白圆点）。

────────────────────────────────────────
八、排障
────────────────────────────────────────
  devtray check                    先跑这个
  systemctl --user status devtray  托盘本体
  tmux ls | grep devtray           服务会话（一个服务一个会话）
  tmux capture-pane -p -S - -t devtray-<项目>-<服务> | tail   不进终端看日志

  症状速查：
    命令 not found / exit 127 → systemd 的 PATH 里没有那个命令的 bin 目录
                                （改 devtray.service 的 Environment=PATH，见 README）
    点了退出图标还在          → unit 用了 Restart=always，改 on-failure
    状态在就绪/启动中之间跳   → ready 正则命中的是会被刷掉的短暂输出，换个更靠后的标志
"#,
        env!("CARGO_PKG_VERSION"),
        state_dir().join("logs").join("<项目>-<服务>.log").display(),
        default_config_path().display(),
        config::expand_home("~/.local/share/icons/hicolor").display(),
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => run_tray(),
        // agent 入口：MCP over stdio（stdout 只走协议消息）
        Some("mcp") => ExitCode::from(mcp::serve() as u8),
        Some("status") | Some("--status") => status(&args[1..]),
        Some("start") => action(&args, cli::start),
        Some("stop") => action(&args, cli::stop),
        Some("restart") => action(&args, cli::restart),
        Some("logs") => logs_cmd(&args[1..]),
        Some("disk") => disk::command(&args[1..]),
        Some("agents") => agents::command(&args[1..]),
        Some("skills") => skills::command(&args[1..]),
        Some("_agents") => agents::command(&["--json".to_string()]),
        Some("_skills") => skills::command(&["--json".to_string()]),
        Some("_skills-check") => skills::command(&["check".to_string(), "--json".to_string()]),
        Some("_skills-plan") => skills::plan_command(&args[1..]),
        Some("_skills-start") => skills::start_command(&args[1..]),
        Some("_skills-run") => skills::run_job(args.get(1)),
        Some("_skills-backups") => skills::backups_command(&args[1..]),
        Some("_skills-restore") => skills::restore_command(&args[1..]),
        Some("_skills-install-plan") => skills::install_plan_command(&args[1..]),
        Some("_skills-install-start") => skills::install_start_command(&args[1..]),
        Some("_disk-scan") => disk::scan_only(&args[1..]),
        Some("_disk-cache") => disk::cache_command(),
        Some("_clean") => clean::job_command(args.get(1)),
        Some("_approve") => clean::approve_command(args.get(1)),
        Some("_clean-plan") => clean::plan_command(&args[1..]),
        Some("_clean-start") => clean::start_command(&args[1..]),
        Some("_job") => jobs::command(&args[1..]),
        Some("ack") => alerts::command(&args[1..]),
        Some("_dismiss-alerts") => match alerts::dismiss(&config_path()) {
            Ok(()) => {
                say!("{{\"dismissed\":true}}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Some("terminal") => panel::terminal_command(args.get(1)),
        Some("panel") => panel::command(&args[1..]),
        Some("menu") => {
            let mut options = vec!["--menu".to_string()];
            options.extend_from_slice(&args[1..]);
            panel::command(&options)
        }
        Some("_panel-data") => panel::data(),
        Some("_service-config") => panel::service_config(args.get(1)),
        Some("_save-service") => panel::save_service(),
        Some("_log-writer") => {
            match args.get(1).ok_or("缺少日志路径".to_string()).and_then(|p| {
                logs::collect(
                    std::path::Path::new(p),
                    std::io::stdin().lock(),
                    logs::MAX_BYTES,
                )
            }) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("open") => action(&args, cli::open_preview),
        Some("edit") => edit_cmd(&args),
        Some("check") | Some("--check") => check(),
        Some("help") => {
            print_manual();
            ExitCode::SUCCESS
        }
        Some("-h") | Some("--help") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some("-V") | Some("--version") => {
            say!("devtray {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("未知命令：{other}\n");
            print_help();
            ExitCode::from(2)
        }
    }
}

fn print_help() {
    say!(
        "devtray {} —— 托盘常驻的开发服务管理器（tmux 托管）

用法:
  devtray                    起托盘（默认）
  devtray panel [--toggle]    打开 / 收回控制面板（Python 3 + PySide6）
  devtray panel --mode small|full  切换并记忆面板模式
  devtray menu [--toggle]     打开 / 收回小面板
  devtray terminal <目标>    打开 tmux 实时终端
  devtray status [--json]    列出服务与状态
  devtray start <目标>       启动
  devtray stop <目标>        停止
  devtray restart <目标>     重启
  devtray ack <目标>         将当前异常标记已读，保留退出状态与日志
  devtray logs <目标> [-n N] [--level warn,error] [--search 文本]
               [--history | --run N] [--json]  日志筛选（默认 80 行）
  devtray disk [项目] [--json] [--refresh]  扫描构建目录占用（默认读 10 分钟缓存）
  devtray disk open <目标>    用默认文件管理器打开项目根 / 服务工作目录
  devtray disk clean <项目> --kind <目录名> [--yes] [--dry-run]
                              清理构建目录（默认只做检查，--dry-run 不删）
  devtray agents [--json]     检测本机 Agent 的版本与安装来源（只读）
  devtray skills [--json|open]  Skills 清单 / lock 对账 / 副本漂移（只读）
  devtray skills check [--refresh]  对比上游 tree sha（6h 缓存，只读）
  devtray skills upgrade|remove <技能名> [--yes] [--allow-drift] [--dry-run]
                             升级/删除技能（写入前快照，漂移默认拒绝）
  devtray skills restore <技能名> [--snapshot ID] [--restore-lock] [--yes]
                             从快照恢复内容与副本（默认不改 lock）
  devtray skills install <来源> [--skill 名称] [--agent ID]... [--yes] [--dry-run]
                             安装技能（GitHub 预检，显式 -a，固定 CLI）
  devtray open <目标>        用默认浏览器打开该服务的预览地址
  devtray edit <目标>        弹出表单改这个服务（yad；无 yad 时用 $EDITOR）
  devtray check              自检依赖/配置/单元
  devtray mcp                以 MCP server（stdio）方式运行，给 agent 用
  devtray help               完整操作说明（托盘菜单「使用说明」就是它）

  <目标> 形如 simple-oa/backend；服务名唯一时可只写 backend。

配置文件: {}
日志目录: {}",
        env!("CARGO_PKG_VERSION"),
        default_config_path().display(),
        state_dir().join("logs").display()
    );
}

fn config_path() -> PathBuf {
    std::env::var("DEVTRAY_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_config_path())
}

/// 首次运行落一份模板，免得托盘起来后菜单空空如也不知道该写什么
fn ensure_template(path: &PathBuf) {
    if path.exists() {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(path, TEMPLATE).is_ok() {
        eprintln!(
            "已生成配置模板：{}（请把示例服务改成你自己的）",
            path.display()
        );
    }
}

fn run_tray() -> ExitCode {
    let path = config_path();
    ensure_template(&path);

    if let Err(e) = acquire_lock() {
        eprintln!("{e}");
        notify::send("devtray 未启动", &e);
        return ExitCode::FAILURE;
    }

    let t = DevTray::new(path);
    if let Some(err) = t.config_error() {
        eprintln!("配置有问题，托盘仍会起来但没有任何服务：{err}");
        notify::send("devtray 配置有问题", err);
    }
    let autostart_targets = t.targets().to_vec();

    let handle = match t.spawn() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("托盘启动失败（找不到 StatusNotifierWatcher？）：{e}");
            return ExitCode::FAILURE;
        }
    };

    // spawn 会消费 DevTray，先把它即将启动的服务名单独播出去
    tray::autostart(&autostart_targets);
    tray::spawn_ticker(handle.clone());

    loop {
        std::thread::park();
    }
}

/// 同一时间只允许一个托盘（systemd 之外手动再跑一个会出现两个图标）
fn acquire_lock() -> Result<(), String> {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let path = PathBuf::from(dir).join("devtray.lock");
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(pid) = text.trim().parse::<u32>()
        && PathBuf::from(format!("/proc/{pid}")).exists()
    {
        return Err(format!("devtray 已在运行（pid {pid}）"));
    }
    std::fs::write(&path, std::process::id().to_string())
        .map_err(|e| format!("写锁文件失败 {}：{e}", path.display()))
}

fn load() -> Result<(Config, Vec<config::Target>), String> {
    let path = config_path();
    let cfg = Config::load(&path)?;
    let targets = cfg.targets()?;
    Ok((cfg, targets))
}

fn status(args: &[String]) -> ExitCode {
    let ctx = match cli::Ctx::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let statuses = ctx.statuses();
    if args.iter().any(|a| a == "--json") {
        say!("{}", cli::statuses_json(&statuses));
    } else {
        say!("{}", cli::statuses_human(&statuses));
        if let Some(advice) = cli::advice(&statuses) {
            say!("\n{advice}");
        }
    }
    ExitCode::SUCCESS
}

/// start/stop/restart 共用的入口：解析目标 → 执行 → 打印结果
fn action(args: &[String], op: fn(&config::Target) -> Result<String, String>) -> ExitCode {
    let Some(spec) = args.get(1) else {
        eprintln!(
            "用法：devtray {} <目标>（例如 devtray {} simple-oa/backend）",
            args[0], args[0]
        );
        return ExitCode::from(2);
    };
    let ctx = match cli::Ctx::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let t = match ctx.resolve(spec) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match op(t) {
        Ok(msg) => {
            say!("{msg}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// 弹 yad 表单改配置。没装 yad 就退回编辑器，不让这条入口变成死路。
fn edit_cmd(args: &[String]) -> ExitCode {
    let Some(spec) = args.get(1) else {
        eprintln!("用法：devtray edit <目标>（例如 devtray edit simple-oa/frontend）");
        return ExitCode::from(2);
    };
    let ctx = match cli::Ctx::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let t = match ctx.resolve(spec) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    if !form::available() {
        eprintln!("机器上没有 yad，改用 $EDITOR 打开配置文件");
        return run_editor();
    }
    let path = config_path();
    match form::run_edit(&path, t) {
        Ok(msg) => {
            say!("{msg}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// 没有 yad 时的兜底：仍然用 $EDITOR 打开配置文件
fn run_editor() -> ExitCode {
    let path = config_path();
    let mut argv = terminal::editor();
    argv.push(path.display().to_string());
    match argv.split_first() {
        Some((prog, rest)) => match std::process::Command::new(prog).args(rest).status() {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("启动编辑器失败：{e}");
                ExitCode::FAILURE
            }
        },
        None => ExitCode::FAILURE,
    }
}

fn logs_cmd(args: &[String]) -> ExitCode {
    let mut spec = None;
    let mut q = logs::Query {
        lines: 80,
        ..Default::default()
    };
    let mut json = false;
    let mut it = args.iter();
    let parsed: Result<(), String> = (|| {
        while let Some(a) = it.next() {
            match a.as_str() {
                "-n" | "--lines" => {
                    q.lines = it
                        .next()
                        .ok_or("缺少行数")?
                        .parse()
                        .map_err(|_| "行数必须是非负整数")?
                }
                "--level" => {
                    q.levels = it
                        .next()
                        .ok_or("缺少日志级别")?
                        .split(',')
                        .map(|s| s.to_lowercase())
                        .collect();
                    if q.levels.iter().any(|s| {
                        !["trace", "debug", "info", "warn", "error", "other"].contains(&s.as_str())
                    }) {
                        return Err("日志级别：trace,debug,info,warn,error,other".into());
                    }
                }
                "--search" => q.search = it.next().ok_or("缺少搜索文本")?.clone(),
                "--history" => q.history = true,
                "--run" => {
                    q.run = it
                        .next()
                        .ok_or("缺少归档编号")?
                        .parse()
                        .map_err(|_| "归档编号必须是整数")?;
                    if q.run > logs::ARCHIVES {
                        return Err("归档编号须在 0..10 范围".into());
                    }
                }
                "--json" => json = true,
                s if s.starts_with('-') => return Err(format!("未知选项：{s}")),
                s => {
                    if spec.replace(s.to_string()).is_some() {
                        return Err("只接受一个目标".into());
                    }
                }
            }
        }
        Ok(())
    })();
    let result = parsed.and_then(|_| {
        let spec = spec.ok_or("用法：devtray logs <目标> [-n N] [--level warn,error] [--search 文本] [--history | --run N] [--json]")?;
        let ctx = cli::Ctx::load()?;
        let t = ctx.resolve(&spec)?;
        logs::query(&t.log_path, &q)
    });
    match result {
        Ok(r) => {
            if json {
                say!("{}", serde_json::to_string(&r).unwrap());
            } else {
                for entry in r.entries {
                    say!("{}", entry.text);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}

fn check() -> ExitCode {
    let mut problems: Vec<String> = Vec::new();
    let path = config_path();
    say!("配置文件 : {}", path.display());

    if !path.exists() {
        problems.push(format!(
            "配置不存在：{}（跑一次 `devtray` 会生成模板）",
            path.display()
        ));
    }
    match load() {
        Ok((_, targets)) => {
            say!("配置解析 : OK（{} 个服务）", targets.len());
            let snap = tmux::snapshot();
            let mut prober = tmux::Prober::default();
            say!("服务状态 :");
            for t in &targets {
                say!(
                    "  {:<26} {:<16} cwd={} 日志={}",
                    format!("{}/{}", t.project, t.service.name),
                    prober.state_of(t, &snap).text(),
                    t.service.cwd,
                    t.log_path.display()
                );
            }
        }
        Err(e) => {
            say!("配置解析 : 失败");
            problems.push(e);
        }
    }

    let python = std::env::var("DEVTRAY_PYTHON").unwrap_or_else(|_| "python3".into());
    let ui_ready = std::process::Command::new(&python)
        .args(["-c", "from PySide6 import QtWidgets, QtNetwork"])
        .output()
        .is_ok_and(|o| o.status.success());
    say!(
        "控制面板 : {}",
        if ui_ready {
            "Python / PySide6 可用"
        } else {
            "未安装 PySide6（可选；仅影响桌面面板）"
        }
    );

    if terminal::which("tmux") {
        let version = std::process::Command::new("tmux")
            .arg("-V")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        say!("tmux     : {}", version);
    } else {
        problems.push("找不到 tmux".into());
    }

    match terminal::resolve("") {
        Some(base) => say!("终端     : {}", base.join(" ")),
        None => {
            say!("终端     : 未找到（交互终端不可用；控制面板日志不依赖终端）");
        }
    }
    say!(
        "通知     : {}",
        if notify::available() {
            "notify-send 可用"
        } else {
            "notify-send 缺失（失败时不会有桌面通知）"
        }
    );

    let icons_line = |label: &str, configured: &str, fallback: &str| {
        say!(
            "图标 {:<9}: {}",
            label,
            icons::explain(configured, fallback)
        );
    };
    say!("图标     : （配置在 ~/.config/devtray/config.toml 的 icon_* 三项）");
    let cfg = Config::load(&path).map(|c| c.settings).unwrap_or_default();
    icons_line("ok", &cfg.icon_ok, icons::FALLBACK_OK);
    icons_line("starting", &cfg.icon_starting, icons::FALLBACK_STARTING);
    icons_line("error", &cfg.icon_error, icons::FALLBACK_ERROR);

    let unit = config::expand_home("~/.config/systemd/user/devtray.service");
    say!(
        "systemd  : {}",
        if unit.exists() {
            format!("{} 存在", unit.display())
        } else {
            format!("{} 缺失（登录不会自启）", unit.display())
        }
    );

    say!("日志目录 : {}", state_dir().join("logs").display());
    if problems.is_empty() {
        say!("\n自检通过");
        ExitCode::SUCCESS
    } else {
        say!("\n发现 {} 个问题：", problems.len());
        for p in &problems {
            say!("  - {p}");
        }
        ExitCode::FAILURE
    }
}
