//! 托盘本体：菜单树、`about_to_show` 拉取刷新、图标三态、失败通知、配置热重载。
//!
//! 状态刷新分工（第 7 问的结论）：
//!   - **菜单内容**：靠 ksni 的 `menu_about_to_show`——宿主每次弹菜单前会调用它，
//!     返回后 ksni 自动把 properties + menu 推给宿主，所以「打开即最新」且零轮询。
//!   - **图标 + 通知**：后台 tick，只在菜单关着的时候起作用。
//!
//! 图标名是实测出来的：`dialog-error` / `dialog-information` 在 noctalia 里会被
//! 渲染成纯白圆点，不可辨；`process-stop`（红✕）和 `view-refresh`（蓝↻）正常。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use ksni::Tray;
use ksni::blocking::Handle;
use ksni::menu::{Disposition, MenuItem, StandardItem, SubMenu};

use crate::config::{Config, Settings, Target};
use crate::tmux::{self, State};
use crate::{cli, form, icons, notify, terminal};

pub struct DevTray {
    cfg_path: PathBuf,
    targets: Vec<Target>,
    settings: Settings,
    states: HashMap<String, State>,
    /// 配置改了但服务还在跑 —— 菜单里标注「需重启生效」
    pending_restart: HashSet<String>,
    config_error: Option<String>,
    acknowledged_config: Option<String>,
    acknowledged_failures: HashMap<String, String>,
    failure_tokens: HashMap<String, String>,
    alert_store: PathBuf,
    mtime: Option<SystemTime>,
    last_notified: HashMap<String, Instant>,
    prober: tmux::Prober,
    terminal_base: Option<Vec<String>>,
    /// 配置里的图标名解析后的结果（在 reload 时解析一次，避免每 tick 查文件）
    icon_ok: String,
    icon_starting: String,
    icon_error: String,
}

impl DevTray {
    /// 刻意不返回 Result：配置有问题也要把托盘拉起来（否则 systemd 会 Restart 循环），
    /// 错误挂到菜单第一行让用户看见。
    pub fn new(cfg_path: PathBuf) -> Self {
        let settings = Settings::default();
        let alert_store = crate::alerts::store_path(&cfg_path);
        let mut tray = Self {
            cfg_path,
            targets: Vec::new(),
            settings,
            states: HashMap::new(),
            pending_restart: HashSet::new(),
            config_error: None,
            acknowledged_config: None,
            acknowledged_failures: HashMap::new(),
            failure_tokens: HashMap::new(),
            alert_store,
            mtime: None,
            last_notified: HashMap::new(),
            prober: tmux::Prober::default(),
            terminal_base: terminal::resolve(""),
            icon_ok: icons::FALLBACK_OK.into(),
            icon_starting: icons::FALLBACK_STARTING.into(),
            icon_error: icons::FALLBACK_ERROR.into(),
        };
        tray.reload();
        tray.mtime = std::fs::metadata(&tray.cfg_path)
            .and_then(|m| m.modified())
            .ok();
        tray
    }

    pub fn config_error(&self) -> Option<&str> {
        self.config_error.as_deref()
    }

    pub fn targets(&self) -> &[Target] {
        &self.targets
    }

    // ---------- 状态 ----------

    fn state(&self, t: &Target) -> State {
        self.states
            .get(&t.session)
            .copied()
            .unwrap_or(State::NotStarted)
    }

    /// 完整刷新：先看配置有没有变，再抓一次状态 + 发通知
    pub fn refresh(&mut self) {
        self.reload_if_changed();
        self.refresh_states();
    }

    pub fn refresh_states(&mut self) {
        let snap = tmux::snapshot();
        let mut states = HashMap::with_capacity(self.targets.len());
        for t in &self.targets {
            states.insert(t.session.clone(), self.prober.state_of(t, &snap));
        }
        let previous_tokens = std::mem::take(&mut self.failure_tokens);
        self.failure_tokens = states
            .iter()
            .filter(|(_, state)| state.is_failure())
            .filter_map(|(session, _)| {
                let token = snap.get(session).and_then(crate::alerts::failure_token)?;
                Some((session.clone(), token))
            })
            .collect();
        self.load_acknowledgements();
        self.check_alerts(&states, &previous_tokens);
        self.states = states;
    }

    fn load_acknowledgements(&mut self) {
        let saved = crate::alerts::Acknowledgements::load_at(&self.alert_store);
        self.acknowledged_config = self
            .config_error
            .as_ref()
            .filter(|error| saved.config_matches(error))
            .cloned();
        self.acknowledged_failures = saved.services;
        self.acknowledged_failures
            .retain(|session, token| self.failure_tokens.get(session) == Some(token));
    }

    fn failure_token(&self, session: &str) -> String {
        self.failure_tokens
            .get(session)
            .cloned()
            .unwrap_or_else(|| format!("{:?}", self.states.get(session)))
    }

    fn acknowledge_alerts(&mut self) {
        let failures = self
            .states
            .iter()
            .filter(|(_, state)| state.is_failure())
            .map(|(session, _)| (session.clone(), self.failure_token(session)))
            .collect();
        if let Err(error) =
            crate::alerts::record_at(&self.alert_store, &failures, self.config_error.as_deref())
        {
            notify::send("devtray：保存已读状态失败", &error);
            return;
        }
        self.acknowledged_config = self.config_error.clone();
        self.acknowledged_failures = failures;
    }

    fn service_acknowledged(&self, session: &str) -> bool {
        self.acknowledged_failures.get(session) == Some(&self.failure_token(session))
    }

    fn unacknowledged_config(&self) -> bool {
        self.config_error.is_some() && self.config_error != self.acknowledged_config
    }

    /// 只在「从非失败变成失败」时发一次通知，并按 debounce 去抖
    fn check_alerts(
        &mut self,
        states: &HashMap<String, State>,
        previous_tokens: &HashMap<String, String>,
    ) {
        if !self.settings.notify {
            return;
        }
        let debounce = Duration::from_secs(self.settings.notify_debounce_s.max(1));
        let now = Instant::now();
        for t in &self.targets {
            let Some(state) = states.get(&t.session) else {
                continue;
            };
            if !state.is_failure() {
                continue;
            }
            if self.acknowledged_failures.get(&t.session) == self.failure_tokens.get(&t.session) {
                continue;
            }
            // 上一次已经是失败态就不重复报（例如同一次崩溃被多次轮询看到）
            if previous_tokens.get(&t.session) == self.failure_tokens.get(&t.session) {
                continue;
            }
            if self
                .last_notified
                .get(&t.session)
                .is_some_and(|at| now.duration_since(*at) < debounce)
            {
                continue;
            }
            self.last_notified.insert(t.session.clone(), now);
            let code = match state {
                State::Exited(Some(c)) => format!("exit {c}"),
                _ => "已退出".into(),
            };
            let mut body = format!("{}/{} {}", t.project, t.service.name, code);
            if let Some(snippet) = tmux::error_snippet(&t.log_path) {
                body.push('\n');
                body.push_str(&snippet);
            }
            notify::send(&format!("devtray：{} 挂了", t.service.name), &body);
        }
    }

    fn badge(&self) -> icons::Badge {
        if self.unacknowledged_config() || self.any_failure() {
            icons::Badge::Error
        } else if self.any_starting() {
            icons::Badge::Starting
        } else if self.states.values().any(State::is_live) {
            icons::Badge::Ready
        } else {
            icons::Badge::Idle
        }
    }

    fn any_failure(&self) -> bool {
        self.states.iter().any(|(session, state)| {
            state.is_failure()
                && self.acknowledged_failures.get(session) != Some(&self.failure_token(session))
        })
    }

    fn any_starting(&self) -> bool {
        self.states.values().any(|s| matches!(s, State::Starting))
    }

    fn summary(&self) -> String {
        let total = self.targets.len();
        if total == 0 {
            return "没有配置任何服务".into();
        }
        let ready = self.states.values().filter(|s| **s == State::Ready).count();
        let running = self
            .states
            .values()
            .filter(|s| **s == State::Running)
            .count();
        let starting = self
            .states
            .values()
            .filter(|s| **s == State::Starting)
            .count();
        let failed = self
            .states
            .iter()
            .filter(|(session, state)| state.is_failure() && !self.service_acknowledged(session))
            .count();
        let acknowledged = self
            .states
            .iter()
            .filter(|(session, state)| state.is_failure() && self.service_acknowledged(session))
            .count();
        let up = ready + running;
        let mut parts = vec![format!("{up}/{total} 在跑")];
        if starting > 0 {
            parts.push(format!("{starting} 启动中"));
        }
        if failed > 0 {
            parts.push(format!("{failed} 失败"));
        }
        if acknowledged > 0 {
            parts.push(format!("{acknowledged} 已读异常"));
        }
        parts.join(" · ")
    }

    // ---------- 配置热重载 ----------

    fn reload_if_changed(&mut self) {
        let Ok(meta) = std::fs::metadata(&self.cfg_path) else {
            return;
        };
        let mtime = meta.modified().ok();
        if mtime == self.mtime {
            return;
        }
        self.mtime = mtime;
        self.reload();
    }

    /// 重载；解析失败保留旧配置，只在菜单里挂一条错误提示
    pub fn reload(&mut self) {
        let result = Config::load(&self.cfg_path).and_then(|c| {
            let targets = c.targets()?;
            Ok((c, targets))
        });
        match result {
            Ok((cfg, targets)) => {
                self.pending_restart = pending_restart(&self.targets, &targets, &self.states);
                self.terminal_base = terminal::resolve(&cfg.settings.terminal);
                self.icon_ok = icons::resolve(&cfg.settings.icon_ok, icons::FALLBACK_OK);
                self.icon_starting =
                    icons::resolve(&cfg.settings.icon_starting, icons::FALLBACK_STARTING);
                self.icon_error = icons::resolve(&cfg.settings.icon_error, icons::FALLBACK_ERROR);
                self.settings = cfg.settings;
                self.targets = targets;
                self.config_error = None;
                self.acknowledged_config = None;
            }
            Err(e) => {
                self.config_error = Some(e);
            }
        }
    }

    // ---------- 菜单动作 ----------

    /// 「使用说明」：弹终端跑 `devtray help`，分页器可滚动可搜索
    fn show_help(&self) {
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "devtray".into());
        let pager = if terminal::which("less") {
            " | less -R".to_string()
        } else {
            String::new()
        };
        let line = format!("{} help{}", crate::config::shell_quote(&exe), pager);
        let cmd = vec!["sh".to_string(), "-lc".to_string(), line];
        match &self.terminal_base {
            Some(base) => {
                if let Err(e) = terminal::spawn_in(base, &cmd) {
                    notify::send("devtray：打不开使用说明", &e);
                }
            }
            None => notify::send("devtray", "没有可用终端，请在命令行跑 devtray help"),
        }
    }

    /// 「编辑这个服务…」：yad 表单改完写回（保住注释），校验不过会弹错误框且不落盘
    fn edit_service_form(&self, t: &Target) {
        if !form::available() {
            notify::send(
                "devtray",
                "机器上没有 yad，弹不出表单；请用菜单里的「编辑配置文件」",
            );
            return;
        }
        let path = self.cfg_path.clone();
        let target = t.clone();
        spawn_action_msg(move || form::run_edit(&path, &target));
    }

    /// 「添加服务…」
    fn add_service_form(&self) {
        if !form::available() {
            notify::send(
                "devtray",
                "机器上没有 yad，弹不出表单；请用菜单里的「编辑配置文件」",
            );
            return;
        }
        let path = self.cfg_path.clone();
        let projects = form::project_names(&path);
        spawn_action_msg(move || form::run_add(&path, &projects));
    }

    fn edit_config(&self) {
        let path = self.cfg_path.to_string_lossy().into_owned();
        let mut argv = terminal::editor();
        argv.push(path);
        if let Some(base) = &self.terminal_base
            && terminal::spawn_in(base, &argv).is_ok()
        {
            return;
        }
        if let Some((prog, args)) = argv.split_first() {
            let mut cmd = Command::new(prog);
            cmd.args(args);
            let _ = crate::proc::spawn_detached(&mut cmd);
        }
    }

    fn service_submenu(&self, t: &Target) -> MenuItem<Self> {
        let state = self.state(t);
        let mut label = format!("{} · {}", t.service.name, state_label(state));
        let acknowledged = state.is_failure() && self.service_acknowledged(&t.session);
        if acknowledged {
            label.push_str(" · 已读");
        }
        if self.pending_restart.contains(&t.session) {
            label.push_str(" · 需重启生效");
        }

        let mut sub: Vec<MenuItem<Self>> = Vec::new();
        if state.is_failure() {
            let target = t.clone();
            let config = self.cfg_path.clone();
            let expected = self.failure_token(&t.session);
            sub.push(
                StandardItem {
                    label: if acknowledged {
                        "异常已读（保留退出状态与日志）"
                    } else {
                        "标记异常已读"
                    }
                    .into(),
                    enabled: !acknowledged,
                    activate: Box::new(move |_| {
                        let target = target.clone();
                        let config = config.clone();
                        let expected = expected.clone();
                        spawn_action(move || {
                            crate::alerts::acknowledge_target(&config, &target, Some(&expected))
                        });
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }

        let log_target = t.clone();
        sub.push(
            StandardItem {
                label: "分类日志 / 历史…".into(),
                icon_name: "text-x-generic".into(),
                activate: Box::new(move |_| {
                    let name = format!("{}/{}", log_target.project, log_target.service.name);
                    if let Err(e) = crate::panel::launch(Some(&name), false) {
                        notify::send("devtray：打开面板失败", &e);
                    }
                }),
                ..Default::default()
            }
            .into(),
        );
        let terminal_target = t.clone();
        let settings = self.settings.clone();
        sub.push(
            StandardItem {
                label: "实时终端（tmux 原始输出）".into(),
                icon_name: "utilities-terminal".into(),
                // Exited panes retain the final screen and remain attachable.
                enabled: state != State::NotStarted,
                activate: Box::new(move |_| {
                    if let Err(e) = crate::panel::terminal(&terminal_target, &settings) {
                        notify::send("devtray：打开终端失败", &e);
                    }
                }),
                ..Default::default()
            }
            .into(),
        );

        // 「打开预览」：地址优先取配置的 url，其次从日志里认，最后用 port 拼
        match cli::preview_url(t) {
            Some(url) => {
                let target = t.clone();
                sub.push(
                    StandardItem {
                        label: menu_label(&format!("打开预览（{}）", cli::url_short(&url))),
                        icon_name: "applications-internet".into(),
                        activate: Box::new(move |_| {
                            if let Err(e) = cli::open_preview(&target) {
                                notify::send("devtray：打开预览失败", &e);
                            }
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            }
            None => sub.push(
                StandardItem {
                    label: "打开预览（未识别到地址）".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            ),
        }

        sub.push(MenuItem::Separator);
        let start_target = t.clone();
        sub.push(
            StandardItem {
                label: "启动".into(),
                icon_name: "media-playback-start".into(),
                enabled: !state.is_live(),
                activate: Box::new(move |_| {
                    let t = start_target.clone();
                    spawn_action(move || crate::cli::start(&t).map(|_| ()));
                }),
                ..Default::default()
            }
            .into(),
        );

        let stop_target = t.clone();
        sub.push(
            StandardItem {
                label: "停止".into(),
                icon_name: "process-stop".into(),
                enabled: state != State::NotStarted,
                activate: Box::new(move |_| {
                    let t = stop_target.clone();
                    spawn_action(move || crate::cli::stop(&t).map(|_| ()));
                }),
                ..Default::default()
            }
            .into(),
        );

        let restart_target = t.clone();
        sub.push(
            StandardItem {
                label: "重启".into(),
                icon_name: "view-refresh".into(),
                activate: Box::new(move |_| {
                    let t = restart_target.clone();
                    spawn_action(move || crate::cli::restart(&t).map(|_| ()));
                }),
                ..Default::default()
            }
            .into(),
        );

        sub.push(MenuItem::Separator);
        let form_target = t.clone();
        sub.push(
            StandardItem {
                label: "编辑这个服务…".into(),
                icon_name: "document-edit".into(),
                activate: Box::new(move |tray: &mut DevTray| tray.edit_service_form(&form_target)),
                ..Default::default()
            }
            .into(),
        );

        SubMenu {
            label: menu_label(&label),
            icon_data: if acknowledged {
                icons::Badge::Idle
            } else {
                icons::Badge::from_state(state)
            }
            .png(),
            disposition: if state.is_failure() && !acknowledged {
                Disposition::Alert
            } else {
                Disposition::Normal
            },
            submenu: sub,
            ..Default::default()
        }
        .into()
    }

    fn bulk_item(&self, label: &str, icon: &str, verb: Bulk) -> MenuItem<Self> {
        self.bulk_item_for(&self.targets, label, icon, verb)
    }

    fn bulk_item_for(
        &self,
        targets: &[Target],
        label: &str,
        icon: &str,
        verb: Bulk,
    ) -> MenuItem<Self> {
        let enabled = targets.iter().any(|t| match verb {
            Bulk::Start => !self.state(t).is_live(),
            Bulk::Stop => self.state(t) != State::NotStarted,
            Bulk::Restart => true,
        });
        let targets = targets.to_vec();
        StandardItem {
            label: label.into(),
            icon_name: icon.into(),
            enabled,
            activate: Box::new(move |_| {
                let targets = targets.clone();
                spawn_action(move || {
                    let snap = tmux::snapshot();
                    let mut prober = tmux::Prober::default();
                    let mut first_err = None;
                    for t in &targets {
                        let state = prober.state_of(t, &snap);
                        let r = match verb {
                            Bulk::Start if state.is_live() => Ok(()), // 不打断正在跑的
                            Bulk::Start => crate::cli::start(t).map(|_| ()),
                            Bulk::Restart => crate::cli::restart(t).map(|_| ()),
                            Bulk::Stop if state == State::NotStarted => Ok(()),
                            Bulk::Stop => crate::cli::stop(t).map(|_| ()),
                        };
                        if let Err(e) = r {
                            first_err.get_or_insert(e);
                        }
                    }
                    first_err.map_or(Ok(()), Err)
                });
            }),
            ..Default::default()
        }
        .into()
    }
}

#[derive(Clone, Copy)]
enum Bulk {
    Start,
    Restart,
    Stop,
}

impl Tray for DevTray {
    /// 左键打开上次选择的面板模式；右键由桌面绘制原生菜单。
    const MENU_ON_ACTIVATE: bool = false;

    fn activate(&mut self, _x: i32, _y: i32) {
        std::thread::spawn(|| {
            if let Err(e) = crate::panel::launch(None, true) {
                notify::send("devtray：控制面板", &e);
            }
        });
    }

    fn id(&self) -> String {
        "devtray".into()
    }

    fn title(&self) -> String {
        format!("devtray · {}", self.summary())
    }

    fn icon_name(&self) -> String {
        let (configured, resolved) = match self.badge() {
            icons::Badge::Error => (&self.settings.icon_error, &self.icon_error),
            icons::Badge::Starting => (&self.settings.icon_starting, &self.icon_starting),
            _ => (&self.settings.icon_ok, &self.icon_ok),
        };
        if [
            icons::DEFAULT_OK,
            icons::DEFAULT_STARTING,
            icons::DEFAULT_ERROR,
            "devtray-idle",
        ]
        .contains(&configured.trim())
        {
            // Empty name selects IconPixmap instead of an old installed theme asset.
            String::new()
        } else {
            resolved.clone()
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.badge().pixmaps()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let detail = self
            .config_error
            .as_deref()
            .map(|e| format!("配置未生效：{e}"))
            .unwrap_or_else(|| self.summary());
        ksni::ToolTip {
            icon_name: self.icon_name(),
            icon_pixmap: self.icon_pixmap(),
            title: "devtray · 开发工作台".into(),
            description: format!(
                "{}<br/>左键：面板（记忆模式）<br/>右键：终端、浏览器与服务操作",
                escape_markup(&detail)
            ),
        }
    }

    /// 宿主每次弹菜单前都会调它 —— 菜单内容永久保持「打开即最新」
    fn menu_about_to_show(&mut self) {
        self.refresh();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: format!("devtray · {}", self.summary()),
                enabled: false,
                icon_data: self.badge().png(),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "打开 / 收回控制面板".into(),
                icon_name: "preferences-system".into(),
                activate: Box::new(|_: &mut DevTray| {
                    std::thread::spawn(|| {
                        if let Err(e) = crate::panel::launch(None, true) {
                            notify::send("devtray", &e);
                        }
                    });
                }),
                ..Default::default()
            }
            .into(),
        ];
        if let Some(err) = &self.config_error {
            items.push(
                StandardItem {
                    label: menu_label(&format!("配置未生效 · {}", compact(err, 64))),
                    icon_data: icons::Badge::Error.png(),
                    disposition: Disposition::Alert,
                    activate: Box::new(|t: &mut DevTray| t.edit_config()),
                    ..Default::default()
                }
                .into(),
            );
        }
        if self.unacknowledged_config() || self.any_failure() {
            items.push(
                StandardItem {
                    label: "清除提示并恢复托盘状态".into(),
                    activate: Box::new(|tray: &mut DevTray| tray.acknowledge_alerts()),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.push(MenuItem::Separator);
        let mut projects = Vec::new();
        for target in &self.targets {
            if !projects.contains(&target.project.as_str()) {
                projects.push(target.project.as_str());
            }
        }
        if projects.is_empty() {
            items.push(
                StandardItem {
                    label: "尚未添加服务".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }
        for project in projects {
            let mut targets: Vec<Target> = self
                .targets
                .iter()
                .filter(|t| t.project == project)
                .cloned()
                .collect();
            // Stable sorting keeps the configured order inside each severity group.
            targets.sort_by_key(|t| match self.state(t) {
                state if state.is_failure() => 0,
                State::Starting => 1,
                state if state.is_live() => 2,
                _ => 3,
            });
            let running = targets.iter().filter(|t| self.state(t).is_live()).count();
            let failed = targets
                .iter()
                .filter(|t| self.state(t).is_failure() && !self.service_acknowledged(&t.session))
                .count();
            let starting = targets.iter().any(|t| self.state(t) == State::Starting);
            let badge = if failed > 0 {
                icons::Badge::Error
            } else if starting {
                icons::Badge::Starting
            } else if running > 0 {
                icons::Badge::Ready
            } else {
                icons::Badge::Idle
            };
            let mut label = format!("{project} · {running}/{} 运行", targets.len());
            if failed > 0 {
                label.push_str(&format!(" · {failed} 异常"));
            }
            let mut submenu: Vec<_> = targets.iter().map(|t| self.service_submenu(t)).collect();
            submenu.push(MenuItem::Separator);
            submenu.push(self.bulk_item_for(
                &targets,
                "启动此项目",
                "media-playback-start",
                Bulk::Start,
            ));
            submenu.push(self.bulk_item_for(&targets, "重启此项目", "view-refresh", Bulk::Restart));
            submenu.push(self.bulk_item_for(
                &targets,
                "停止此项目",
                "media-playback-stop",
                Bulk::Stop,
            ));
            items.push(
                SubMenu {
                    label: menu_label(&label),
                    icon_data: badge.png(),
                    submenu,
                    ..Default::default()
                }
                .into(),
            );
        }
        items.push(MenuItem::Separator);
        items.push(
            SubMenu {
                label: "批量操作".into(),
                icon_name: "system-run".into(),
                enabled: !self.targets.is_empty(),
                submenu: vec![
                    self.bulk_item("启动所有未运行服务", "media-playback-start", Bulk::Start),
                    self.bulk_item("重启所有服务", "view-refresh", Bulk::Restart),
                    self.bulk_item("停止所有服务", "media-playback-stop", Bulk::Stop),
                ],
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "添加服务…".into(),
                icon_name: "list-add".into(),
                activate: Box::new(|t: &mut DevTray| t.add_service_form()),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            SubMenu {
                label: "设置与帮助".into(),
                icon_name: "preferences-system".into(),
                submenu: vec![
                    StandardItem {
                        label: "编辑配置文件…".into(),
                        icon_name: "document-edit".into(),
                        activate: Box::new(|t: &mut DevTray| t.edit_config()),
                        ..Default::default()
                    }
                    .into(),
                    StandardItem {
                        label: "重新加载配置".into(),
                        icon_name: "view-refresh".into(),
                        activate: Box::new(|t: &mut DevTray| {
                            t.reload();
                            t.refresh_states();
                        }),
                        ..Default::default()
                    }
                    .into(),
                    MenuItem::Separator,
                    StandardItem {
                        label: "使用说明".into(),
                        icon_name: "help-about".into(),
                        activate: Box::new(|t: &mut DevTray| t.show_help()),
                        ..Default::default()
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "退出 devtray（服务继续运行）".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| {
                    let _ = crate::panel::quit();
                    std::process::exit(0);
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

/// 找出「定义变了、而且还在跑」的服务
fn pending_restart(
    old: &[Target],
    new: &[Target],
    states: &HashMap<String, State>,
) -> HashSet<String> {
    let mut out = HashSet::new();
    for nt in new {
        let Some(ot) = old.iter().find(|o| o.session == nt.session) else {
            continue;
        };
        if ot.service.runtime_eq(&nt.service) {
            continue;
        }
        if states.get(&nt.session).is_some_and(|s| s.is_live()) {
            out.insert(nt.session.clone());
        }
    }
    out
}

/// 动作一律丢到后台线程：停止一个服务可能要等满 3s，卡在托盘服务线程上会让菜单僵住
fn spawn_action<F>(f: F)
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    std::thread::spawn(move || {
        if let Err(e) = f() {
            notify::send("devtray：操作失败", &e);
        }
    });
}

/// 和 spawn_action 一样丢后台线程，但成功时也发一条通知（表单保存要给人反馈）
fn spawn_action_msg<F>(f: F)
where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    std::thread::spawn(move || match f() {
        Ok(msg) if !msg.is_empty() => notify::send("devtray", &msg),
        Ok(_) => {}
        Err(e) => notify::send("devtray：操作失败", &e),
    });
}

/// 后台 tick：只负责图标变色和失败通知
pub fn spawn_ticker(handle: Handle<DevTray>) {
    std::thread::spawn(move || {
        loop {
            let ms = handle
                .update(|t: &mut DevTray| {
                    t.refresh();
                    t.settings.poll_ms.clamp(500, 60_000)
                })
                .unwrap_or(2000);
            std::thread::sleep(Duration::from_millis(ms));
        }
    });
}

/// 托盘起来后按配置自动拉起服务；已经在跑的（tmux 会话还在）一根手指都不碰
pub fn autostart(targets: &[Target]) {
    let snap = tmux::snapshot();
    for t in targets {
        if !t.service.autostart {
            continue;
        }
        // 死 pane（崩溃过）也留着不管，让失败状态在菜单里可见
        if snap.contains_key(&t.session) {
            continue;
        }
        let t = t.clone();
        spawn_action(move || crate::cli::start(&t).map(|_| ()));
    }
}

// dbusmenu interprets a single underscore as an accelerator, including in project names.
fn menu_label(text: &str) -> String {
    text.replace('_', "__").replace(['\n', '\r'], " ")
}
fn state_label(state: State) -> String {
    state
        .text()
        .split_once(' ')
        .map(|(_, text)| text.to_string())
        .unwrap_or_default()
}
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn compact(text: &str, limit: usize) -> String {
    let clean = text.replace(['\n', '\r'], " ");
    if clean.chars().count() > limit {
        format!("{}…", clean.chars().take(limit).collect::<String>())
    } else {
        clean
    }
}

#[cfg(test)]
mod tray_tests {
    use super::*;

    #[test]
    fn restart_restores_acknowledged_failure_and_neutral_service_menu() {
        let mut tray = fixture();
        let session = tray.targets[1].session.clone();
        let token = crate::alerts::failure_token(&tmux::PaneInfo {
            dead: true,
            status: Some(1),
            pid: 4242,
        })
        .unwrap();
        tray.failure_tokens.insert(session.clone(), token);
        tray.acknowledge_alerts();
        let mut restarted = fixture();
        restarted.alert_store = tray.alert_store.clone();
        restarted.failure_tokens = tray.failure_tokens.clone();
        restarted.load_acknowledgements();
        assert_eq!(restarted.badge(), icons::Badge::Ready);
        assert!(restarted.states[&session].is_failure());
        assert!(restarted.summary().contains("已读异常"));
        if let MenuItem::SubMenu(menu) = restarted.service_submenu(&restarted.targets[1]) {
            assert!(menu.label.contains("已读"));
            assert!(matches!(menu.disposition, Disposition::Normal));
            assert!(menu.submenu.iter().any(|item| matches!(item, MenuItem::Standard(action) if action.label.starts_with("异常已读") && !action.enabled)));
        } else {
            panic!("missing service menu");
        }
        restarted
            .failure_tokens
            .insert(session, "exit=1;pid=5252".into());
        restarted.load_acknowledgements();
        assert_eq!(restarted.badge(), icons::Badge::Error);
        let _ = std::fs::remove_file(&tray.alert_store);
        let _ = std::fs::remove_file(tray.alert_store.with_extension("lock"));
    }

    #[test]
    fn acknowledging_restores_badge_without_changing_failures_and_new_episode_alerts() {
        let mut tray = fixture();
        let session = tray.targets[1].session.clone();
        tray.failure_tokens
            .insert(session.clone(), "exit1:10".into());
        assert_eq!(tray.badge(), icons::Badge::Error);
        tray.acknowledge_alerts();
        assert_eq!(tray.badge(), icons::Badge::Ready);
        assert!(tray.states[&session].is_failure());
        tray.failure_tokens.insert(session, "exit1:20".into());
        assert_eq!(tray.badge(), icons::Badge::Error);
    }

    #[test]
    fn acknowledged_config_stays_visible_but_new_error_alerts() {
        let mut tray = fixture();
        tray.config_error = Some("bad configuration A".into());
        tray.acknowledge_alerts();
        assert_eq!(tray.badge(), icons::Badge::Ready);
        assert_eq!(tray.config_error(), Some("bad configuration A"));
        tray.config_error = Some("bad configuration B".into());
        assert_eq!(tray.badge(), icons::Badge::Error);
    }

    fn fixture() -> DevTray {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("devtray-menu-{}-{stamp}.toml", std::process::id()));
        std::fs::write(
            &path,
            r#"
[[project]]
name = "my_project"
[[project.service]]
name = "web_ui"
cwd = "/tmp"
cmd = "true"
port = 5173
[[project.service]]
name = "api"
cwd = "/tmp"
cmd = "true"
[[project]]
name = "other"
[[project.service]]
name = "worker"
cwd = "/tmp"
cmd = "true"
"#,
        )
        .unwrap();
        let mut tray = DevTray::new(path.clone());
        tray.alert_store = path.with_extension("acks.json");
        std::fs::remove_file(path).unwrap();
        tray.states
            .insert(tray.targets[0].session.clone(), State::Ready);
        tray.states
            .insert(tray.targets[1].session.clone(), State::Exited(Some(1)));
        tray
    }

    #[test]
    fn menu_groups_projects_prioritizes_failures_and_embeds_status_icons() {
        let mut tray = fixture();
        let menu = tray.menu();
        let project = menu
            .iter()
            .find_map(|item| match item {
                MenuItem::SubMenu(s) if s.label.starts_with("my__project") => Some(s),
                _ => None,
            })
            .unwrap();
        assert!(project.label.contains("1/2 运行"));
        assert!(project.label.contains("1 异常"));
        assert_eq!(&project.icon_data[..8], b"\x89PNG\r\n\x1a\n");
        let services: Vec<_> = project
            .submenu
            .iter()
            .filter_map(|item| match item {
                MenuItem::SubMenu(s) => Some(s),
                _ => None,
            })
            .collect();
        assert!(services[0].label.starts_with("api · 失败"));
        assert!(services[1].label.starts_with("web__ui · 就绪"));
        for service in &services {
            assert!(service.submenu.iter().any(|item|
                matches!(item, MenuItem::Standard(s) if s.label == "实时终端（tmux 原始输出）" && s.enabled)));
            assert!(service.submenu.iter().any(|item|
                matches!(item, MenuItem::Standard(s) if s.label == "分类日志 / 历史…" && s.enabled)));
        }
        assert!(services[1].submenu.iter().any(|item|
            matches!(item, MenuItem::Standard(s) if s.label.starts_with("打开预览（127.0.0.1:5173") && s.enabled)));
        assert!(
            !services[1].submenu.iter().any(
                |item| matches!(item, MenuItem::Standard(s) if s.label == "启动" && s.enabled)
            )
        );
        assert!(
            menu.iter()
                .any(|item| matches!(item, MenuItem::SubMenu(s) if s.label == "批量操作"))
        );
        assert!(
            menu.iter()
                .any(|item| matches!(item, MenuItem::SubMenu(s) if s.label == "设置与帮助"))
        );
        assert_eq!(tray.badge(), icons::Badge::Error);
        assert_eq!(tray.icon_name(), "");
        tray.states.clear();
        assert_eq!(tray.badge(), icons::Badge::Idle);
        tray.config_error = Some("invalid <toml>".into());
        assert_eq!(tray.badge(), icons::Badge::Error);
        assert!(tray.tool_tip().description.contains("&lt;toml&gt;"));
        tray.settings.icon_error = "process-stop".into();
        tray.icon_error = "process-stop".into();
        assert_eq!(tray.icon_name(), "process-stop");
    }
}
