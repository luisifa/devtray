//! 图形化改配置：调 `yad --form` 弹表单，拿回字段值后用 `toml_edit` 写回。
//!
//! 为什么是 yad 而不是给 devtray 引 GTK：GTK4/Adwaita 会带进上百个 crate，
//! 还会把「纯 Rust、不依赖 GUI 工具链」这个优点弄没了。yad 支持 `:DIR` 目录选择器、
//! `:CHK` 复选框、`:TXT` 多行，够用；没装 yad 的机器自动退回 `$EDITOR`。
//!
//! 两条硬性保证：
//!   1. **保住注释**：`toml_edit` 是保格式的编辑器，你写的中文注释不会被抹掉。
//!   2. **先校验再落盘**：改完先解析 + 语义校验（cwd 存在、正则合法、会话名不冲突），
//!      全过了才原子写入（临时文件 + rename），填错不会把配置写坏。

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use toml_edit::{ArrayOfTables, DocumentMut, Item, Key, Table, value};

use crate::config::{Config, Target};
use crate::terminal;

/// 表单里的字段（都是字符串，落盘时才转类型——这样"填错"能给出人话错误）
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Fields {
    pub project: String,
    pub service: String,
    pub cwd: String,
    pub cmd: String,
    pub ready: String,
    pub port: String,
    pub url: String,
    pub autostart: bool,
    pub env: Vec<(String, String)>,
}

impl Fields {
    pub fn from_target(t: &Target) -> Self {
        Self {
            project: t.project.clone(),
            service: t.service.name.clone(),
            cwd: t.service.cwd.clone(),
            cmd: t.service.cmd.clone(),
            ready: t.service.ready.clone().unwrap_or_default(),
            port: t.service.port.map(|p| p.to_string()).unwrap_or_default(),
            url: t.service.url.clone().unwrap_or_default(),
            autostart: t.service.autostart,
            env: t
                .service
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }

    fn env_text(&self) -> String {
        self.env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn set_env_text(&mut self, text: &str) -> Result<(), String> {
        let mut out = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                return Err(format!("环境变量这一行缺少 `=`：{line}"));
            };
            let k = k.trim();
            if k.is_empty() {
                return Err(format!("环境变量名是空的：{line}"));
            }
            out.push((k.to_string(), v.trim().to_string()));
        }
        self.env = out;
        Ok(())
    }
}

pub fn available() -> bool {
    terminal::which("yad")
}

// ── 纯函数：改 TOML（可以单测，不需要 GUI）──────────────────────────────────

/// 改一个已存在的服务
pub fn apply_edit(text: &str, f: &Fields) -> Result<String, String> {
    let mut doc = parse_doc(text)?;
    let projects = projects_mut(&mut doc)?;
    let proj = projects
        .iter_mut()
        .find(|t| t.get("name").and_then(Item::as_str) == Some(f.project.as_str()))
        .ok_or_else(|| format!("配置里找不到项目 {:?}", f.project))?;
    let services = proj
        .get_mut("service")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| format!("项目 {} 下没有 service 段", f.project))?;
    let tbl = services
        .iter_mut()
        .find(|t| t.get("name").and_then(Item::as_str) == Some(f.service.as_str()))
        .ok_or_else(|| format!("配置里找不到服务 {}/{}", f.project, f.service))?;
    fill(tbl, f)?;
    Ok(doc.to_string())
}

/// 新增一个服务（项目不存在就建）
pub fn apply_add(text: &str, f: &Fields) -> Result<String, String> {
    if f.project.trim().is_empty() || f.service.trim().is_empty() {
        return Err("项目名和服务名都不能为空".into());
    }
    let mut doc = parse_doc(text)?;
    if doc.get("project").is_none() {
        doc.insert("project", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let projects = projects_mut(&mut doc)?;

    // 先按名字找下标，避免同时持有两个可变借用
    let idx = projects
        .iter()
        .position(|t| t.get("name").and_then(Item::as_str) == Some(f.project.trim()));
    let proj = match idx {
        Some(i) => projects.get_mut(i).ok_or("项目下标越界")?,
        None => {
            let mut t = Table::new();
            t.insert("name", value(f.project.trim()));
            projects.push(t);
            let n = projects.len();
            projects.get_mut(n - 1).ok_or("新增项目失败")?
        }
    };

    if let Some(services) = proj.get("service").and_then(Item::as_array_of_tables)
        && services
            .iter()
            .any(|t| t.get("name").and_then(Item::as_str) == Some(f.service.trim()))
    {
        return Err(format!(
            "{}/{} 已经存在了（要改就编辑它，不要新增）",
            f.project.trim(),
            f.service.trim()
        ));
    }

    let mut t = Table::new();
    // 和仓库模板/现有配置的缩进保持一致：`[[project.service]]` 头与字段都缩进两格。
    // 不设这个的话，新追加的服务会顶格，混在已缩进的服务里很难看（实测过）。
    t.decor_mut().set_prefix("  ");
    let mut name_key = Key::new("name");
    name_key.leaf_decor_mut().set_prefix("  ");
    t.insert_formatted(&name_key, value(f.service.trim()));
    fill(&mut t, f)?;
    if proj.get("service").is_none() {
        proj.insert("service", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    proj.get_mut("service")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or("service 段结构不对")?
        .push(t);
    Ok(doc.to_string())
}

fn parse_doc(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>()
        .map_err(|e| format!("TOML 解析失败：{e}"))
}

fn projects_mut(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables, String> {
    doc.get_mut("project")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| "配置里没有 [[project]] 段".to_string())
}

/// 把表单字段写进一个服务表。
/// 两条原则：
///   ① 只在**值真的变了**时才动它 —— 于是「打开表单直接保存」是字节级幂等的，
///      不会因为一次无意义保存把注释/格式搅乱。
///   ② 改值时用 `get_key_value_mut` **就地换 value**，把 key 留在原地 ——
///      这样 key 上面的注释（你写的中文说明）不会被 `insert` 一起换掉。
///      实测踩过：用 `Table::insert` 会连注释一起丢。
fn fill(tbl: &mut Table, f: &Fields) -> Result<(), String> {
    let cwd = f.cwd.trim();
    let cmd = f.cmd.trim();
    if cwd.is_empty() {
        return Err("工作目录不能为空".into());
    }
    if cmd.is_empty() {
        return Err("命令不能为空".into());
    }
    put_str(tbl, "cwd", cwd);
    put_str(tbl, "cmd", cmd);
    put_opt(tbl, "ready", f.ready.trim());
    put_opt(tbl, "url", f.url.trim());

    match f.port.trim() {
        "" => {
            tbl.remove("port");
        }
        p => {
            let n: u16 = p
                .parse()
                .map_err(|_| format!("端口 {p:?} 不是 0-65535 之间的数字"))?;
            if n == 0 {
                return Err("端口不能是 0".into());
            }
            if tbl.get("port").and_then(Item::as_integer) != Some(n as i64) {
                put(tbl, "port", value(n as i64));
            }
        }
    }

    // 按「有效值」比较：配置里没写 autostart 就等于 false，
    // 所以不写 key 也算一致 —— 否则一次无改动保存会平白多出一行。
    let cur_autostart = tbl
        .get("autostart")
        .and_then(Item::as_bool)
        .unwrap_or(false);
    if cur_autostart != f.autostart {
        put(tbl, "autostart", value(f.autostart));
    }

    // env：只在内容变化时动，且逐个 key 就地改（保住 env 段里的注释）
    let want: BTreeMap<&str, &str> = f
        .env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let cur: BTreeMap<String, String> = tbl
        .get("env")
        .and_then(Item::as_table)
        .map(|t| {
            t.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.to_string(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let same = cur.len() == want.len()
        && cur
            .iter()
            .all(|(k, v)| want.get(k.as_str()).is_some_and(|w| w == v));
    if !same {
        if want.is_empty() {
            tbl.remove("env");
        } else {
            if tbl.get("env").and_then(Item::as_table).is_none() {
                tbl.insert("env", Item::Table(Table::new()));
            }
            let env = tbl
                .get_mut("env")
                .and_then(Item::as_table_mut)
                .ok_or("env 段结构不对")?;
            let stale: Vec<String> = env
                .iter()
                .map(|(k, _)| k.to_string())
                .filter(|k| !want.contains_key(k.as_str()))
                .collect();
            for k in stale {
                env.remove(&k);
            }
            for (k, v) in &want {
                if env.get(k).and_then(Item::as_str) != Some(v) {
                    put(env, k, value(*v));
                }
            }
        }
    }
    Ok(())
}

/// 就地换 value：key（连同它上面的注释）保持不动。
/// 新建 key 时补上与既有风格一致的缩进 —— 否则新插的 `url`/`port` 会顶格，
/// 跟同段里缩进两格的老 key 混在一起很难看。
fn put(tbl: &mut Table, key: &str, item: Item) {
    match tbl.get_key_value_mut(key) {
        Some((_key, slot)) => *slot = item,
        None => {
            // 新 key 的缩进在 **key 的装饰**上（不是 value 的 —— 放 value 上会插到
            // `key` 和 `=` 之间，直接把 TOML 写坏，实测踩过）
            let mut k = Key::new(key);
            k.leaf_decor_mut().set_prefix("  ");
            tbl.insert_formatted(&k, item);
        }
    }
}

fn put_str(tbl: &mut Table, key: &str, v: &str) {
    if tbl.get(key).and_then(Item::as_str) != Some(v) {
        put(tbl, key, value(v));
    }
}

/// 可选字段：清空 = 删掉 key（不写空串）
fn put_opt(tbl: &mut Table, key: &str, v: &str) {
    if v.is_empty() {
        tbl.remove(key);
    } else {
        put_str(tbl, key, v);
    }
}

// ── 校验 + 原子落盘 ────────────────────────────────────────────────────────

/// 写盘前必须过 Config::parse + targets()（语义校验）；不过就不写
pub fn save_validated(path: &Path, text: &str) -> Result<(), String> {
    let cfg = Config::parse(text)?;
    cfg.targets()?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("写临时文件失败：{e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("替换配置文件失败：{e}"))?;
    Ok(())
}

// ── yad 表单 ──────────────────────────────────────────────────────────────

const SEP: char = '\u{1f}';

fn form_argv(title: &str, f: &Fields, edit_mode: bool) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "--form".into(),
        format!("--title=devtray — {title}"),
        "--width=780".into(),
        "--separator=\u{1f}".into(),
        "--button=取消:1".into(),
        "--button=保存:0".into(),
    ];
    if edit_mode {
        a.push("--field=项目（要改名请直接编辑配置文件）:RO".into());
        a.push("--field=服务（要改名请直接编辑配置文件）:RO".into());
    } else {
        a.push("--field=项目".into());
        a.push("--field=服务".into());
    }
    a.push("--field=工作目录:DIR".into());
    a.push("--field=命令（经 sh -lc 执行）".into());
    a.push("--field=就绪正则（匹配日志输出，可选）".into());
    a.push("--field=端口（可选）".into());
    a.push("--field=预览地址（可选）".into());
    a.push("--field=随托盘自动启动:CHK".into());
    a.push("--field=环境变量 K=V（每行一个，可选）:TXT".into());

    a.push(f.project.clone());
    a.push(f.service.clone());
    // DIR 字段不认字面 `~`（会显示成 (None)、保存时还可能把路径弄丢），
    // 所以只在喂给 yad 这一层展开；配置里仍保持用户写的 `~/...` 风格。
    a.push(
        crate::config::expand_home(&f.cwd)
            .to_string_lossy()
            .into_owned(),
    );
    a.push(f.cmd.clone());
    a.push(f.ready.clone());
    a.push(f.port.clone());
    a.push(f.url.clone());
    a.push(if f.autostart { "TRUE" } else { "FALSE" }.into());
    a.push(f.env_text());
    a
}

/// 解析 yad 的输出（9 个字段，用 \x1f 分隔，末尾可能多一个分隔符）
pub fn fields_from_yad(out: &str, edit_mode: bool, orig: &Fields) -> Result<Fields, String> {
    let parts: Vec<&str> = out.trim_end_matches(['\n', SEP]).split(SEP).collect();
    let get = |i: usize| {
        let v = parts.get(i).copied().unwrap_or("").trim();
        if v == "(None)" {
            String::new()
        } else {
            v.to_string()
        }
    };
    let mut f = Fields {
        project: if edit_mode {
            orig.project.clone()
        } else {
            get(0)
        },
        service: if edit_mode {
            orig.service.clone()
        } else {
            get(1)
        },
        cwd: {
            let shown = get(2);
            let same = crate::config::expand_home(&shown) == crate::config::expand_home(&orig.cwd);
            if edit_mode && same {
                orig.cwd.clone() // 没改就保留 `~/...` 原样，别把配置改成绝对路径
            } else {
                shown
            }
        },
        cmd: get(3),
        ready: get(4),
        port: get(5),
        url: get(6),
        autostart: get(7).eq_ignore_ascii_case("true"),
        env: Vec::new(),
    };
    f.set_env_text(parts.get(8).copied().unwrap_or(""))?;
    Ok(f)
}

fn spawn_form(title: &str, f: &Fields, edit_mode: bool) -> Result<Option<Fields>, String> {
    let out = Command::new("yad")
        .args(form_argv(title, f, edit_mode))
        .output()
        .map_err(|e| format!("启动 yad 失败：{e}"))?;
    match out.status.code() {
        Some(0) => {
            let text = String::from_utf8_lossy(&out.stdout);
            fields_from_yad(&text, edit_mode, f).map(Some)
        }
        Some(1) => Ok(None), // 取消
        other => Err(format!("yad 异常退出（{other:?}）")),
    }
}

fn error_dialog(msg: &str) {
    let _ = Command::new("yad")
        .args([
            "--error",
            "--title=devtray 配置没有保存",
            "--width=640",
            &format!("--text={msg}"),
        ])
        .status();
}

/// 编辑一个已有服务（阻塞，调用方放到后台线程里跑）
pub fn run_edit(cfg_path: &Path, t: &Target) -> Result<String, String> {
    let orig = Fields::from_target(t);
    let title = format!("编辑 {}/{}", t.project, t.service.name);
    let Some(f) = spawn_form(&title, &orig, true)? else {
        return Ok("已取消".into());
    };
    let text = std::fs::read_to_string(cfg_path).map_err(|e| format!("读配置失败：{e}"))?;
    let new_text = match apply_edit(&text, &f) {
        Ok(v) => v,
        Err(e) => {
            error_dialog(&e);
            return Err(e);
        }
    };
    if let Err(e) = save_validated(cfg_path, &new_text) {
        error_dialog(&e);
        return Err(e);
    }
    Ok(format!(
        "已保存 {}/{}（配置自动重载）",
        t.project, t.service.name
    ))
}

/// 新增服务：项目名可以选已有的，也可以新建
pub fn run_add(cfg_path: &Path, projects: &[String]) -> Result<String, String> {
    let hint = if projects.is_empty() {
        String::new()
    } else {
        format!("（已有项目：{}）", projects.join("、"))
    };
    let f0 = Fields {
        project: projects.first().cloned().unwrap_or_default(),
        ..Default::default()
    };
    let title = format!("新增服务 {hint}");
    let Some(f) = spawn_form(&title, &f0, false)? else {
        return Ok("已取消".into());
    };
    let text = std::fs::read_to_string(cfg_path).map_err(|e| format!("读配置失败：{e}"))?;
    let new_text = match apply_add(&text, &f) {
        Ok(v) => v,
        Err(e) => {
            error_dialog(&e);
            return Err(e);
        }
    };
    if let Err(e) = save_validated(cfg_path, &new_text) {
        error_dialog(&e);
        return Err(e);
    }
    Ok(format!(
        "已新增 {}/{}（配置自动重载）",
        f.project, f.service
    ))
}

/// 配置里的项目名列表（给「新增服务」表单做提示）
pub fn project_names(cfg_path: &Path) -> Vec<String> {
    Config::load(cfg_path)
        .map(|c| c.projects.into_iter().map(|p| p.name).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"# 顶层注释，必须保住
[settings]
poll_ms = 2000

[[project]]
name = "demo"

  # 这是一条服务级注释
  [[project.service]]
  name = "backend"
  cwd = "/tmp"
  cmd = "sleep 1"
  ready = "listen on"
"#;

    fn f(project: &str, service: &str) -> Fields {
        Fields {
            project: project.into(),
            service: service.into(),
            cwd: "/tmp".into(),
            cmd: "echo hi".into(),
            ..Default::default()
        }
    }

    #[test]
    fn edit_updates_fields_and_keeps_comments() {
        let mut fields = f("demo", "backend");
        fields.cmd = "cargo run".into();
        fields.port = "3060".into();
        fields.url = "http://localhost:3060/".into();
        fields.autostart = true;
        fields.env = vec![("RUST_LOG".into(), "debug".into())];

        let out = apply_edit(SAMPLE, &fields).unwrap();
        assert!(
            out.contains("# 顶层注释，必须保住"),
            "顶层注释丢了：\n{out}"
        );
        assert!(
            out.contains("# 这是一条服务级注释"),
            "服务级注释丢了：\n{out}"
        );
        assert!(out.contains(r#"cmd = "cargo run""#));
        assert!(out.contains("port = 3060"));
        assert!(out.contains(r#"url = "http://localhost:3060/""#));
        assert!(out.contains("autostart = true"));
        assert!(out.contains(r#"RUST_LOG = "debug""#));
        // 落盘后必须仍能通过解析与语义校验
        Config::parse(&out).unwrap().targets().unwrap();
    }

    #[test]
    fn edit_clears_optional_fields_instead_of_writing_empty() {
        let mut fields = f("demo", "backend");
        fields.ready = "  ".into(); // 用户清空了 → 应删掉 key，而不是写空正则
        let out = apply_edit(SAMPLE, &fields).unwrap();
        assert!(!out.contains("ready"), "空 ready 没被删掉：\n{out}");
        // 空 ready 若被写成空串会匹配一切，导致假就绪
        let cfg = Config::parse(&out).unwrap();
        let t = cfg.targets().unwrap();
        assert!(t[0].ready_re.is_none());
    }

    #[test]
    fn edit_keeps_comment_above_the_edited_key() {
        // 这条注释就压在 ready 上面；用 insert 换值会把它一起丢掉（实测踩过）
        let mut fields = f("demo", "backend");
        fields.ready = "ready-changed".into();
        let out = apply_edit(SAMPLE, &fields).unwrap();
        assert!(
            out.contains("# 这是一条服务级注释"),
            "服务级注释被丢了：\n{out}"
        );
        assert!(out.contains(r#"ready = "ready-changed""#));
    }

    #[test]
    fn saving_without_changes_is_byte_identical() {
        // 「打开表单直接保存」必须不动文件，否则每保存一次就蚕食一点注释
        let target = Config::parse(SAMPLE).unwrap().targets().unwrap()[0].clone();
        let same = Fields::from_target(&target);
        assert_eq!(apply_edit(SAMPLE, &same).unwrap(), SAMPLE);
    }

    #[test]
    fn dir_field_expands_tilde_but_save_keeps_tilde_style() {
        use crate::config::Service;
        let t = Target {
            project: "p".into(),
            service: Service {
                name: "s".into(),
                cwd: "~/some/dir".into(),
                cmd: "true".into(),
                autostart: false,
                port: None,
                ready: None,
                url: None,
                env: Default::default(),
                log_file: None,
            },
            session: "devtray-p-s".into(),
            log_path: "/tmp/x.log".into(),
            ready_re: None,
        };
        let f = Fields::from_target(&t);
        // 原样保留用户写的 `~/...`：展开只发生在喂给 yad 的那一刻（见 form_argv 的测试）
        assert_eq!(f.cwd, "~/some/dir");
        let expanded = crate::config::expand_home("~/some/dir")
            .to_string_lossy()
            .into_owned();
        // 用户没改这个字段（yad 回的是展开后的绝对路径）→ 保存时仍写回 `~/...`
        let out = fields_from_yad(
            &format!("p{SEP}s{SEP}{expanded}{SEP}true{SEP}{SEP}{SEP}{SEP}FALSE{SEP}"),
            true,
            &f,
        )
        .unwrap();
        assert_eq!(out.cwd, "~/some/dir");
        // 用户真的改了 → 用新值
        let out2 = fields_from_yad(
            &format!("p{SEP}s{SEP}/tmp{SEP}true{SEP}{SEP}{SEP}{SEP}FALSE{SEP}"),
            true,
            &f,
        )
        .unwrap();
        assert_eq!(out2.cwd, "/tmp");
    }

    #[test]
    fn form_argv_gives_yad_an_expanded_dir() {
        let mut fields = f("demo", "backend");
        fields.cwd = "~/some/dir".into();
        let argv = form_argv("t", &fields, true);
        let expanded = crate::config::expand_home("~/some/dir")
            .to_string_lossy()
            .into_owned();
        assert!(
            argv.contains(&expanded),
            "argv 里没给展开后的路径：{argv:?}"
        );
        assert!(!argv.contains(&"~/some/dir".to_string()));
    }

    #[test]
    fn yad_none_dir_becomes_empty_for_clear_error() {
        let orig = f("demo", "backend");
        let out = format!("demo{SEP}backend{SEP}(None){SEP}cmd{SEP}{SEP}{SEP}{SEP}FALSE{SEP}");
        let got = fields_from_yad(&out, true, &orig).unwrap();
        assert_eq!(got.cwd, "");
        assert!(
            apply_edit(SAMPLE, &got)
                .unwrap_err()
                .contains("工作目录不能为空")
        );
    }

    #[test]
    fn add_appends_to_existing_project() {
        let out = apply_add(SAMPLE, &f("demo", "frontend")).unwrap();
        assert!(out.contains(r#"name = "frontend""#));
        let targets = Config::parse(&out).unwrap().targets().unwrap();
        assert_eq!(targets.len(), 2);
        // 新服务的表头和字段要和已有服务一样缩进两格
        assert!(
            out.contains("  [[project.service]]\n  name = \"frontend\"\n  cwd = \"/tmp\"\n  cmd = \"echo hi\""),
            "新增服务的缩进不一致：\n{out}"
        );
    }

    #[test]
    fn add_creates_new_project_and_keeps_comments() {
        let out = apply_add(SAMPLE, &f("newproj", "web")).unwrap();
        assert!(out.contains("# 顶层注释，必须保住"));
        assert!(out.contains(r#"name = "newproj""#));
        assert_eq!(Config::parse(&out).unwrap().targets().unwrap().len(), 2);
        // 新项目里的服务段同样要缩进两格
        assert!(
            out.contains(
                "[[project]]\nname = \"newproj\"\n  [[project.service]]\n  name = \"web\""
            ),
            "新增项目的格式不对：\n{out}"
        );
    }

    #[test]
    fn add_rejects_duplicate() {
        let err = apply_add(SAMPLE, &f("demo", "backend")).unwrap_err();
        assert!(err.contains("已经存在"), "{err}");
    }

    #[test]
    fn rejects_bad_port_and_empty_cmd() {
        let mut bad = f("demo", "backend");
        bad.port = "70000".into();
        assert!(apply_edit(SAMPLE, &bad).unwrap_err().contains("端口"));
        let mut bad2 = f("demo", "backend");
        bad2.cmd = "  ".into();
        assert!(
            apply_edit(SAMPLE, &bad2)
                .unwrap_err()
                .contains("命令不能为空")
        );
    }

    #[test]
    fn parse_yad_fields() {
        let orig = f("demo", "backend");
        // yad 输出：9 个字段 + 末尾分隔符 + 换行；TXT 字段里带换行
        let out = format!(
            "demo{SEP}backend{SEP}/tmp{SEP}bun run dev{SEP}ready in{SEP}5173{SEP}http://localhost:5173/{SEP}TRUE{SEP}A=1\nB=2{SEP}\n"
        );
        let got = fields_from_yad(&out, true, &orig).unwrap();
        assert_eq!(got.project, "demo");
        assert_eq!(got.service, "backend");
        assert_eq!(got.cmd, "bun run dev");
        assert_eq!(got.port, "5173");
        assert!(got.autostart);
        assert_eq!(
            got.env,
            vec![("A".into(), "1".into()), ("B".into(), "2".into())]
        );
    }

    #[test]
    fn reject_bad_env_line() {
        let orig = f("demo", "backend");
        let out = format!(
            "demo{SEP}backend{SEP}/tmp{SEP}cmd{SEP}{SEP}{SEP}{SEP}FALSE{SEP}NOT_A_PAIR{SEP}\n"
        );
        let err = fields_from_yad(&out, true, &orig).unwrap_err();
        assert!(err.contains("缺少 `=`"), "{err}");
    }
}
