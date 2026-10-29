# 资产与技能管理设计（v0.2 / v0.3 / 可选 v0.4）

记录日期：2026-10-28。**复核修订：2026-10-28（第二轮，安全与一致性）**。
状态：**v0.2 资源页 / 清理门禁 / MCP 工具与 v0.3 Agent、Skills 清单、更新检查、安装/升级/删除、快照恢复已实现；Agent 升级执行与项目级 Skills 待实现**。
范围：项目磁盘占用、清理、打开目录、本机 Agent 检测与升级、Skills 清单 / 更新 / 安装、
MCP 扩展，以及可选的 AI 发现能力。

> 本轮复核推翻了三条原设计假设：① `skills` CLI 的 `remove` / `update` 会改写或删除 agent 副本，
> 所以"只写主存储"不成立；② `tmux::snapshot()` 失败时返回空表，不能用来证明"没有服务在跑"；
> ③ 目录名白名单不构成删除依据。对应章节已重写，未解决前不得进入写操作实施。

需求原文六条（编号沿用）：

1. 扫描每个工作项目的磁盘占用，知道编译库大小（`target/`、`node_modules/` 等）。
2. 增加"打开目录"按钮，用本机默认文件管理器打开。
3. 增加"清空"按钮，清理 `node_modules/` 等缓存；Python 的 `venv` 如何清理待定。
4. 扫描本机各个 Agent，参考 cc-switch 的扫描功能，用来升级 Agent。
5. 扫描对应的 Skills，查看版本、删除、升级；避免重复处理。
6. 把 Skills 安装交给 Agent，或由 devtray 执行 `npx skills add ...`。

## 0. 结论

| 需求 | 可行性 | 方案要点 | 主要风险 |
| --- | --- | --- | --- |
| 1 磁盘扫描 | **高**（纯本地） | 自研 Rust 遍历；白名单目录 + 命中即停；`st_blocks` 计费；inode 去重 | 13G 级目录耗时；pnpm 硬链接口径 |
| 2 打开目录 | **高**（很小） | `xdg-open`，失败回退 `gio open`；项目根 / 服务 cwd 两个入口 | 无 |
| 3 清理 | **中** | 白名单 + **证据链**（构建工具归属 / VCS 未跟踪 / `pyvenv.cfg`）+ 运行状态未知即拒绝 + 挂载边界 + 二次确认 + 审计 | 误删不可撤销；推断根过宽 |
| 4 Agent 升级 | **中** | 来源感知（native / npm / pacman）；优先 agent 自带 `update`；sudo 交终端 | 来源判断错误会装出第二份 |
| 5 Skills 版本 / 升级 | **中** | tree sha 对比；**CLI 会改写/删除 agent 副本**，必须全路径预览 + 备份 + 恢复 | 无 semver；CLI 写入范围不可完全约束；lock 竞态 |
| 6 Skills 安装 | **中高** | 确定性管线；显式 `-a`；固定已测 CLI 版本；三身份命名与冲突处理 | 供应链来源不可信；同名覆盖 |
| AI 发现 | **低优先，可延后** | 只做候选排序与解释；OpenAI 兼容端点；永不执行 | 无明确任务时收益低；prompt injection |

一句话：**1/2 可以立刻做且风险低；3 的成本在证据链与状态判定，不在技术；4/5/6 可行，但 Skills
写操作必须按"枚举全部受影响路径 → 全量快照 → 执行 → 校验"来做，CLI 不是只写主存储。**

复核后新增的门槛（未满足则不进入写操作实施）：

- 清理候选必须同时通过：白名单、构建工具归属、VCS 未跟踪、项目根围栏、挂载边界、运行状态可确认。
- Skills 写操作必须能列出**每一个**将被改动的路径（主存储、各 agent 副本、lock），并全部备份；
  存在独立副本（非符号链接）时默认拒绝，除非用户逐项确认。
- 任务模型（清理 / 安装 / 升级）统一为后台 job，MCP 与面板都通过 job id 查询，不阻塞状态接口。

## 1. 已核实的现状（决定设计的事实）

架构：

- Rust 二进制负责 CLI、MCP、tmux 托管、托盘；PySide6 面板源码由 `include_str!` 内嵌，通过
  `_panel-data`、`_save-service` 等一次性子进程调用拿 JSON（`src/panel.rs`、`ui/panel.py`）。
- 面板 `request()` 的单次调用**硬超时 30 秒**（`ui/panel.py`），所有请求共用一个超时。
- 配置只有 `[[project]] name` + `[[project.service]]`，路径只存在于 `service.cwd`。
- 面板 4 页：总览 / 服务管理 / 日志中心 / 设置；最小窗口 1040×720；小面板固定 430px 宽。

本机实测（决定了性能与兼容设计）：

| 事实 | 数值 |
| --- | --- |
| 最大构建目录 | `local_agent_studio/target` ≈ **13G**；`devtray/target` 1.1G |
| 前端依赖目录 | `simple-oa/frontend/node_modules` ≈ 1.0G；另有 826M、923M 两个 |
| `~/Workspace` 下 `target` / `node_modules` 数量 | 10 / 4 |
| 配置中的项目 | 5 个（simple-oa、killtower、local-agent-studio、hb_scf、hb_ecom） |
| Skills | `~/.agents/skills` 42 个目录，`.skill-lock.json` 只记录 31 个 → **11 个来源未知** |
| 镜像漂移 | `~/.claude/skills/ui-ux-pro-max` 与 `~/.agents/skills/ui-ux-pro-max` **内容不同**；design / brand 一致 |
| 项目级 skills | 4 个项目根下存在 `.claude/skills` 或 `.agents/skills`（共 6 处目录） |
| Agent 安装形态 | claude 2.1.312 = 原生安装（`~/.local/share/claude/versions/`，自带 `update`）；codex 0.165.0 = CachyOS pacman 包（`openai-codex`，需 sudo） |
| 工具可用性 | `yad` / `zenity` / `dust` / `gh`（已登录 luisifa）/ `curl` / `node` / `npx` / `bun` / `uv` 均在；`gemini`、`opencode` 未安装 |
| 本地 AI 运行时 | 无（ollama / llama-server / lms 均不存在） |
| cc-switch | 已安装，自带 skill 存储与同步（`skillStorageLocation: cc_switch`）——与 devtray 存在双写风险 |

外部 CLI 事实（`skills@1.7.4` 源码与帮助实测）：

- `skills ls -g --json` 存在，清单可机器读取；`skills add ... --json -y` 存在。
- `skills check` **没有** `--json`，只能解析文本 → 不作为数据源。
- lock 的 `skillFolderHash` 是 **40 位 git tree sha**（上游目录树哈希），不是语义版本。
- `skills add` 支持 `-g`、`-a <agent>`、`-s <skill>`、`--copy`、`-y`、`--all`；默认符号链接。
- `skills update <names> -g -y`、`skills remove <names> -g -y` 均支持非交互。
- `skills find` 是交互式 fzf；**裸名字搜索改用 `https://skills.sh/api/search?q=`**（已实测返回
  JSON：`id` / `source` / `skillId` / `installs`）。

复核新增的 CLI 行为事实（`skills@1.7.4` 源码，直接决定写操作设计）：

- `remove -g <name> -y` 未指定 `-a` 时 `targetAgents = Object.keys(agents)`，对每个 agent 的
  `globalSkillsDir/<sanitizeName(name)>` 执行递归删除——**包括 `--copy` 产生的独立副本**。
- `update -g <name> -y` 内部重新执行 `add <source> --skill <name> -y`，**不带 `-a`**；`-y` 分支会
  `ensureUniversalAgents(detectInstalledAgents())`，即向所有已检测到的 agent 重装/重建链接。
- `sanitizeName` 规则：转小写 → 非 `[a-z0-9._]` 替换为 `-` → 去掉首尾 `.`/`-` → 截断 255 字符 →
  空值回落 `unnamed-skill`。技能身份 = frontmatter `name`，安装目录 = `sanitizeName(name)`，
  二者与仓库路径的父目录名都可能不同。

复核新增的代码事实：

- `tmux::snapshot()`（`src/tmux.rs:105`）在 `tmux list-panes` 失败时直接返回空 `HashMap`，
  空表同时表示"没有 tmux server"和"查询失败"，不能作为清理前置判断。
- 稀疏文件实测（本机）：`truncate -s 1G` 后 `size=1073741824`、`blocks=0`、`du` 显示 0。
- Rust 1.97.1：`std::fs::File::try_lock` 可用，文件锁可以持有到操作结束，无需新增 crate。

## 2. 决策记录

| # | 决策 | 理由 | 后果 / 否决项 |
| --- | --- | --- | --- |
| D1 | **分级执行**：缓存清理、`npx skills` 直接执行；sudo 类升级只拉起终端；AI 建议永不执行 | 破坏性/提权操作需要人在场 | 否决"全部静默执行" |
| D2 | **AI 本期不做**，只留只读 Advisor 扩展点；安装执行走确定性管线 | 6 条需求都是确定性逻辑；本机无本地推理运行时 | 否决"AI 自主安装"；AI 只做候选排序与解释 |
| D3 | Skills **全局单清单**：`~/.agents/skills` + `.skill-lock.json` 为唯一事实来源；`~/.claude/skills` 只做漂移告警 | 避免同一 skill 多版本各自升级（正是"重复处理"的来源） | 项目级 skills 本期只展示数量与路径 |
| D4 | "有没有新版" = **上游 git tree sha 对比**，缓存 6h | lock 只有 tree sha，没有版本号；`skills check` 无 JSON | 否决解析 CLI 文本；否决"不检测" |
| D5 | `[[project]]` 新增可选 **`root`**，缺省用 service cwd 的公共祖先推断 | 现有模型推不准 killtower / hb_scf 的根 | v0.2 只支持手改配置；项目级表单后置 |
| D6 | 扫描**自研 Rust 遍历**，不依赖 dust | dust 无进度、JSON 尺寸需再解析、缺归类信息、多一个运行时依赖 | 否决 dust / 混合方案；dust 仅作为手工排查工具保留 |
| D7 | 清理**白名单 + 证据链 + 状态未知即拒绝 + 路径/挂载围栏 + 二次确认** | 目录名不证明目录可删；快照查询失败不能当成"没在跑" | 否决仅按名字删除；否决"先停服务再清理" |
| D8 | Python **只处理项目内** `.venv`（需 `pyvenv.cfg` + 可重建证明），家目录共享缓存不碰 | `~/.cache/uv`、`~/.cargo/registry` 影响所有项目 | 否决本期清共享缓存 |
| D9 | Agent 升级**来源感知**，优先 agent 自带 `update`，sudo 交终端 | claude 原生安装、codex pacman 包；统一 npm 会双安装 | 否决"统一 npm i -g" |
| D10 | Skills 写操作**先枚举全部受影响路径 → 全量快照 → 执行 → 校验**；恢复叫"快照恢复"，不承诺无损回滚 | CLI 会删/改 agent 副本；lock 全局唯一 | 否决"全量一键升级"、否决自研 clone 覆盖、否决"只备份主存储" |
| D11 | 安装走**确定性管线**；默认 **symlink**；"扫尾"只校验 | 漂移正是 copy 造成的；agent 执行不可复现 | 否决"透传交互式 npx"、否决"交给 agent 执行" |
| D12 | 侧栏新增 **资源** 与 **Agent 与 Skills** 两页；总览只加一行摘要；小面板不变 | 1040×720 现有布局已满；小面板不适合表格 | 否决全部塞进总览 / 改成 Tab |
| D13 | MCP **写操作也暴露**，但默认 `mcp_writes = "ask"`，用 yad 弹窗审批，超时默认拒绝，全程审计 | 用户要求 agent 可执行；同时不能无人在场删 13G | 否决"只读"；否决"不审批只审计" |
| D14 | 资源页**只扫纳管项目**，另给"扫描任意目录"只读入口 | 未纳管目录无归属、无删除依据 | 否决扫全 `~/Workspace`、否决未纳管也可清理 |
| D15 | **两期交付**：v0.2 资源（1/2/3），v0.3 Agent & Skills（4/5/6），v0.4 可选 AI | 两块的依赖与回归面完全不同 | 否决一期全量 |
| D16 | `skills add` 必须显式 `-a`；`update` / `remove` 的隐含 agent 目标必须**预览 + 备份**；任一受影响路径是独立副本（非符号链接）时**默认拒绝写** | 源码实测：`remove` 默认遍历所有 agent；`update` 重新 `add -y` 到所有已检测 agent | 否决"CLI 只写 `~/.agents`"；否决静默覆盖漂移副本 |
| D17 | 新增可区分错误的运行状态接口；清理与 start/stop/restart 共用 `ops` 文件锁；审批后、删除前各复检一次；project root 重叠时检查所有相关项目的服务 | `snapshot()` 失败返回空表；审批与删除之间存在 TOCTOU | 否决"读一次快照就删"；否决只按项目名判断运行中 |
| D18 | 删除前独立解析 `/proc/self/mountinfo`，候选子树含任何挂载点（包括 bind mount）一律拒绝 | 扫描的 `st_dev` 跳过只是展示口径；`remove_dir_all` 没有文件系统边界 | 否决"扫描跳过 = 删除安全" |
| D19 | 尺寸分"分配空间（`st_blocks`）"与"逻辑大小（`st_size`）"两列；`st_blocks == 0` 保留 0；不承诺"预计释放字节"，删除后只报目录消失 + 文件系统可用空间近似变化 | 稀疏文件 1G 但 `blocks=0`；外部硬链接使删除不释放数据块 | 否决"删除后精确报告释放字节" |
| D20 | 技能身份三列：仓库路径、frontmatter `name`、安装目录名（`sanitizeName`）；同名不同源阻断，覆盖需逐项确认 | CLI 用 frontmatter 名做身份、用 sanitize 后的名字做目录，二者可与父目录名不同 | 否决用一个 `name` 字段兼任三种身份 |
| D21 | "快照恢复"覆盖：内容 + 各 agent 副本 + 链接形态 + 该技能的 lock 条目；恢复本身是写操作（flock + 备份 + 校验 + 审计）；不可恢复时明确标"仅内容导出" | 只恢复目录会与 lock 不一致，删除后也不会恢复链接 | 否决"复制目录就叫回滚" |
| D22 | MCP 写工具立即返回 `job_id`；审批与执行在后台线程；新增 `devtray_job`（查询 / 取消）；幂等键去重；断连后任务继续 | `mcp.rs` 同步逐条处理，阻塞 60 秒会卡住同一连接的状态查询 | 否决"一次同步调用跑完审批+删除" |
| D23 | 默认固定 CLI 版本 `npx --yes skills@1.7.4`；升级 pin 需跑回归；`skills_cli` 可覆盖但 UI 标注"未验证版本" | 浮动版本无法支撑"确定性安装"承诺 | 否决裸 `npx skills` 作为默认 |

## 3. 配置与数据模型

### 3.1 配置新增（向后兼容，均为可选）

```toml
[settings]
# 新字段；默认值即当前行为
mcp_writes = "ask"          # ask | deny | allow
disk_cache_ttl_s = 600      # 资源页缓存
skill_check_ttl_s = 21600   # 上游更新检查缓存（6h）
# 固定已验证版本；改 pin 必须跑回归，裸 `npx skills` 不接受
skills_cli = "npx --yes skills@1.7.4"
skill_agents = []           # 空 = 已检测到的 agent；显式列出则只向这些 agent 安装
clean_extra_dirs = []       # 额外允许清理的目录名；默认不含 dist/build/vendor

[[project]]
name = "hb_scf"
root = "~/Workspace/hb_software/hb_scf"   # 新增；不写则按 service.cwd 推断
```

推断规则：取该 project 全部 `service.cwd` 的公共祖先；只有一个 cwd 时结果就是它，并在 UI 上标注
"推断自 cwd"。跨顶层目录无法归并时回退到第一个 cwd 并给出警告。

推断结果的**清理禁用条件**（扫描仍可用，但只读）：根等于 `/`、`$HOME`、`~/Workspace` 这类过宽目录；
或根下直接子目录数量超过 50。此时资源页显示"根目录过宽，已禁用清理，请显式配置 `root`"。

### 3.2 运行期状态

| 路径 | 内容 | 生命周期 |
| --- | --- | --- |
| `~/.local/state/devtray/cache/disk/<project>.json` | 扫描结果 + `scanned_at` + 耗时 + 完整性标记 | TTL 10 分钟，手动刷新强制重扫 |
| `~/.local/state/devtray/cache/skills.json` | 清单 + 上游 sha + 检查时间 | TTL 6 小时 |
| `~/.local/state/devtray/jobs/<id>.json` | 后台任务：`pending_approval` / `running` / `done` / `error` / `cancelled` | 完成 24h 后清理 |
| `~/.local/state/devtray/ops.lock` | **跨进程 `flock`**：清理与 start/stop/restart 共用；进程退出由内核释放，不按年龄判陈旧 | 进程存活期间持有 |
| `~/.local/state/devtray/skills-backup/<name>/<ts>/` | 内容快照 + `manifest.json`（lock 条目、各 agent 副本、链接形态） | 保留 3 份 |
| `~/.local/state/devtray/audit.jsonl` | 写操作审计（10 MiB 轮转） | 长期 |

审计记录字段：`ts`、`source`（`panel` / `cli` / `mcp:<client>`）、`action`、`job_id`、`project`、
`paths`、`allocated_bytes`、`command`、`exit`、`error`、`external_change`。
**不记录 token / 密钥。**

### 3.3 核心类型（Rust 侧）

```rust
struct ProjectRoot {
    project: String, root: PathBuf, inferred: bool,
    cleaning_allowed: bool,         // 根过宽 / 推断异常时为 false
    reason: Option<String>,
}
struct Artifact {
    kind: ArtifactKind, path: PathBuf,
    allocated: u64,                 // st_blocks * 512；稀疏文件为 0
    logical: u64,                   // st_size 之和，与 allocated 分列
    files: u64,
    evidence: Evidence,             // 构建文件、pyvenv.cfg、VCS 跟踪、挂载点
    deletable: bool,                // 证据链全部通过才为 true
}
struct Evidence {
    build_file: Option<PathBuf>,    // Cargo.toml / package.json+lock / pyvenv.cfg
    vcs_tracked: bool,              // git ls-files 命中即禁止删除
    pyvenv_cfg: bool,
    mount_point: Option<PathBuf>,   // 候选内最近挂载点（含 bind mount）
}
struct ProjectUsage {
    project: String, root: PathBuf, artifacts: Vec<Artifact>,
    other_top: Vec<(PathBuf, u64)>, scanned_at: u64, duration_ms: u64,
    skipped: u64, mount_skipped: Vec<PathBuf>,
    complete: bool,                 // 有跳过/错误时为 false，UI 需标注
    warnings: Vec<String>,
}
struct SkillIdentity {
    repo_path: Option<String>,      // 仓库内 SKILL.md 路径
    frontmatter_name: Option<String>, // SKILL.md frontmatter 的 name
    install_dir: String,            // sanitizeName(frontmatter_name)，实际目录名
}
struct AgentInfo {
    id: String, bin: String, version: Option<String>,
    provenance: Provenance, package: Option<String>,
    latest: Option<String>, latest_source: String, self_update: bool, needs_sudo: bool,
}
struct SkillInfo {
    identity: SkillIdentity, dir: PathBuf, source: Option<String>, source_url: Option<String>,
    git_ref: Option<String>, tree_sha: Option<String>,
    installed_at: Option<String>, updated_at: Option<String>,
    update_available: Option<bool>, mirrors: Vec<Mirror>, managed: bool,
}
struct Job {
    id: String, action: String, state: JobState, plan_hash: String,
    created_at: u64, started_at: Option<u64>, finished_at: Option<u64>,
    progress: f32, result: Option<serde_json::Value>, error: Option<String>,
    source: String,                 // panel / cli / mcp:<client>
}
```

## 4. 模块设计

### 4.1 磁盘扫描（`src/disk.rs`）

扫描对象：项目根下的已知构建目录，**任意深度、命中即停**；不进入已命中的目录内部。

| 类型 | 目录名 | 默认清理 | 重建成本 |
| --- | --- | --- | --- |
| Node 依赖 | `node_modules` | ✅ | 中（需联网/缓存） |
| Rust 构建 | `target` | ✅ | 高（首次编译数分钟） |
| Python 虚拟环境 | `.venv`、`venv` | ✅ 需可重建证明 | 中 |
| Python 缓存 | `__pycache__`、`.pytest_cache`、`.mypy_cache`、`.ruff_cache` | ✅ | 无 |
| 前端构建缓存 | `.next`、`.nuxt`、`.turbo`、`.parcel-cache`、`.svelte-kit` | ✅ | 无（下次构建重建） |
| 构建产物 | `dist`、`build`、`vendor` | ❌（需 `clean_extra_dirs` 显式加入） | 视项目而定 |

尺寸口径（复核后修正）：

- 分两列：**分配空间** = `st_blocks * 512`（与 `du` 一致），**逻辑大小** = `st_size` 之和。
  `st_blocks == 0` 的稀疏文件分配空间就是 0，**不回落** `st_size`。
- 一次扫描内用 `(st_dev, st_ino)` 去重，避免 pnpm 同一 inode 被重复计数；但去重只解决"重复计数"，
  不能证明"删除后能回收"——硬链接指向的外部 store 仍持有数据块。
- 不跟随符号链接；跨文件系统（`st_dev` 变化）不进入，并记入 `mount_skipped`。
- 无权限目录计入 `skipped`，置 `complete = false`；完整性不足时**禁止据此清理**。
- 额外输出项目根下一层的 top-5 大目录（未归类部分），帮助发现"钱花在哪"。

性能与交互：

- 每个项目一个独立进程 `devtray _disk-scan --project <name> --json`，面板并发 2 个，可取消。
- 扫描进程可选 `--progress`：stdout 每 500ms 输出一行 JSON 进度，结束输出最终结果；面板用
  `readyRead` 增量读取，不依赖 30 秒超时。
- 缓存命中直接返回（带 `scanned_at`）；用户点"重新扫描"时传 `--refresh`。

CLI：

```bash
devtray disk                         # 人类可读表格（全部项目，默认读缓存）
devtray disk <project> --refresh     # 单项目，忽略缓存
devtray disk --json                  # 结构化输出
devtray _disk-scan <project>         # 面板内部用：单项目 JSON
devtray _disk-cache                  # 面板首屏：只读缓存
devtray disk open <project|项目/服务>  # 打开根目录 / 服务工作目录
devtray disk clean <项目> --kind <目录名> [--kind …] [--yes] [--dry-run] [--json]
devtray _clean <job-id>              # 面板 / MCP 的任务入口
devtray _job <job-id> [--cancel]     # 任务状态查询
```

清理默认只做检查：不传 `--yes` 时需要输入项目名确认；`--dry-run` 只打印计划不删除；
`--all` 才会选中项目内全部可删候选。没有显式 `--kind` / `--all` 时直接拒绝，避免"手滑清空"。

当前实现为**同步一次扫描**：85.7 GiB 的 `simple-oa` 实测 4.1 秒，14.6 GiB 的
`local-agent-studio` 3.2 秒。缓存（10 分钟 TTL）与流式进度随资源页一起加（v0.2 步骤 5），
不阻塞现有 CLI。

### 4.2 清理（`src/clean.rs`）

"发现候选"与"允许删除"是两件事：扫描阶段列出所有白名单目录，**只有六项检查全部通过才置
`deletable = true`**；否则只读展示并写明原因。

| # | 检查 | 规则 |
| --- | --- | --- |
| 1 | 根围栏 | 路径严格在 `root` 之下（`path != root`，canonicalize 后以 root 为前缀）；路径本身不是符号链接；root 未触发 3.1 的清理禁用条件 |
| 2 | 名字白名单 | 目录名在允许清单（或 `clean_extra_dirs`）；`dist` / `build` / `vendor` 默认拒绝 |
| 3 | 构建工具归属 | 见下方证据表；仅名字相同不算 |
| 4 | VCS 未跟踪 | root 是 git worktree 时，`git ls-files -- <path>` 有输出即拒绝（防 `src/target` 这类被纳入版本控制的目录） |
| 5 | 挂载边界 | 解析 `/proc/self/mountinfo`；候选真实路径之下存在任何挂载点（含 bind mount，`st_dev` 相同也算）即拒绝 |
| 6 | 运行状态可确认 | 新接口返回 `Err` 即拒绝；检查**所有 root 与候选重叠的项目**；任一服务处于 running / starting / ready 即拒绝 |

证据表（构建工具归属）：

| 目录 | 必须同时满足 |
| --- | --- |
| `node_modules` | 父目录有 `package.json`，且存在 `bun.lock` / `bun.lockb` / `pnpm-lock.yaml` / `yarn.lock` / `package-lock.json` 之一 |
| `target` | 祖先有 `Cargo.toml`，且内部有 `CACHEDIR.TAG` 或 `debug/` / `release/` / `<triple>/` 之一 |
| `.venv` / `venv` | 内部有 `pyvenv.cfg`，且父目录有 `uv.lock` 或 `requirements.txt`；**只有 `pyproject.toml` 时拒绝**（无法确定 extras 与安装方式） |
| `__pycache__` | 内部至少一个 `*.pyc` |
| `.next` / `.nuxt` / `.turbo` / `.parcel-cache` / `.svelte-kit` | 父目录有 `package.json` |
| `clean_extra_dirs` 中的名字 | 父目录有 `package.json` 或 `Cargo.toml`；仍需通过 1、4、5、6 |

运行状态与互斥：

- 现有 `tmux::snapshot()` 在 `tmux list-panes` 失败时返回空表，无法区分"没有服务"与"查询失败"。
  新增 `snapshot_result() -> Result<Snapshot, String>`；清理只接受 `Ok`，`Err` 一律拒绝并提示
  "无法确认服务状态，请检查 tmux 后重试"。
- 新增 `ops.lock`（`std::fs::File::try_lock` 独占），**清理与 `start` / `stop` / `restart` 共用**；
  拿不到锁返回"有启动/停止/清理操作进行中"。不按文件年龄判断陈旧——进程退出由内核释放锁。
- 审批通过后、真正删除前，重新执行六项检查（路径 inode 变化、新增挂载、服务被启动、VCS 状态变化
  任一命中即中止）。

执行方式：

- Rust `remove_dir_all`，不经过 shell；不跟随符号链接。
- 以独立任务进程运行：`devtray _clean --job <id> --project <p> --path <abs>`，进度写
  `jobs/<id>.json`；面板轮询。**面板中途关闭不会中断删除**，重开后可看到结果。
- 删除是幂等的：大目录删到一半失败时，重跑会继续；单条失败不阻塞其他路径，最终统一报告。
- 结束后**只报事实**："目录已不存在 / 仍存在（原因）"；不承诺"释放 N 字节"。可选附
  `statvfs` 前后可用空间差，并标注"文件系统级近似值，受其他进程影响"。
- 审计记录路径、分配空间、结果、是否检测到外部变更。

重建提示（展示，不自动执行）：

| 目录 | 检测依据 | 提示命令 |
| --- | --- | --- |
| `node_modules` | `bun.lock`/`bun.lockb` | `bun install` |
| | `pnpm-lock.yaml` | `pnpm install` |
| | `yarn.lock` | `yarn install` |
| | `package-lock.json` | `npm ci` |
| `target` | `Cargo.toml` | `cargo build` |
| `.venv` | `pyvenv.cfg` + `uv.lock` | `uv sync` |
| | `pyvenv.cfg` + `requirements.txt` | `python -m venv .venv && pip install -r requirements.txt` |
| | 只有 `pyproject.toml` | 拒绝删除（无法确定安装方式） |

### 4.3 打开目录（`src/disk.rs` / `cli.rs`）

- 先 `xdg-open <dir>`；失败回退 `gio open <dir>`；用 `proc::spawn_detached`，避免僵尸进程。
- 两个入口：项目根（资源页、总览摘要）、服务 cwd（服务卡右键/详情）。
- 目录不存在时返回明确错误，不静默失败。
- 与现有 `cli::open_preview` 共用"外部打开"的封装，避免两套逻辑。

### 4.4 Agent 检测与升级（`src/agents.rs`）

初始内置清单（可扩展；只展示 PATH 中检测到的）：

| id | 二进制 | 版本命令 | 已知安装形态 | 最新版本来源 |
| --- | --- | --- | --- | --- |
| `claude-code` | `claude` | `claude --version` | native（`~/.local/share/claude`）/ npm | `claude update`（自带）或 npm registry |
| `codex` | `codex` | `codex --version` | pacman `openai-codex` / npm `@openai/codex` | `codex update` / `pacman -Si` |
| `pi` | `pi` | `pi --version` | `~/.pi/agent` 自管理 | `pi update self` |
| `dsh` | `dsh` | `dsh --version` | npm `@deepseek-ai/dsh` | npm |
| `gemini-cli` | `gemini` | `gemini --version` | npm `@google/gemini-cli` | npm |
| `opencode` | `opencode` | `opencode --version` | npm `opencode-ai` | npm |
| `qwen-code` | `qwen` | `qwen --version` | npm `@qwen-code/qwen-code` | npm |
| `aider` | `aider` | `aider --version` | pipx / uv | PyPI（可选） |

来源判定顺序：

1. 解析二进制真实路径（`readlink -f` 等价）；
2. 包管理器归属：`pacman -Qo <path>`（Arch 系）→ `dpkg -S <path>`（Debian 系）→ distro 包；
3. npm 全局前缀：`npm prefix -g`，路径在其下 → npm；
4. 版本化目录（`~/.local/share/<name>/versions/<ver>` 或带版本后缀的软链）→ native；
5. 否则 `unknown`，不提供升级按钮，只显示"来源未知，请手动升级"。

版本比较：

- distro 包：`pacman -Si <pkg>` 的 `Version`；与已装版本比较（去掉 `-1.1` 之类的 pkgrel 后缀）。
- npm 包：`npm view <pkg> version`（需要 node；失败则显示"未知"）。
- native：优先执行 `<agent> update`，不预先查"最新"；UI 显示"由自身 updater 管理"。
- 结果缓存 1 小时；网络失败保留上次结果并标注时间。

升级执行：

- **有自带 `update`**：直接执行（claude、codex）。若命令以"需要包管理器"退出，转为终端执行。
- **distro 包**：不静默 sudo。用已配置终端打开并运行 `sudo pacman -S <pkg>`（Debian：`sudo apt install --only-upgrade <pkg>`），用户在终端里输密码。
- **npm 全局**：`npm i -g <pkg>@latest`（用户级，无需 sudo），输出流式写入日志。
- **来源未知**：禁用按钮，提供复制命令。
- MCP 不暴露升级（结果不可观测且可能提权），只暴露只读的 `devtray_agents`。

### 4.5 Skills 清单与版本（`src/skills.rs`）

数据来源优先级：

1. `~/.agents/skills/` 目录（实际文件，判断存在性、大小、漂移）；
2. `~/.agents/.skill-lock.json`（`version: 3`；字段 `source`、`sourceType`、`sourceUrl`、`ref`、
   `skillPath`、`skillFolderHash`、`installedAt`、`updatedAt`）；
3. 二者对账：磁盘有、lock 无 → `managed = false`（来源未知，不可检查更新）；
   lock 有、磁盘无 → 标记"残留记录"，提供清理入口（只删 lock 条目还是重装，由用户选）。
4. 身份三列（D20）：`skillPath`（仓库内路径）、frontmatter `name`、`sanitizeName(name)`（安装目录）。
   清单以安装目录为行键，但更新、覆盖判断、安装后校验都用三元组；同名不同源必须阻断。

CLI 语义（`skills@1.7.4` 实测，直接影响第 4.6 节）：

- `remove -g <name> -y` 不带 `-a` → `targetAgents = Object.keys(agents)`，递归删除每个 agent 的
  `<globalSkillsDir>/<install_dir>`，包括独立副本。
- `update -g <name> -y` 内部执行 `add <source> --skill <name> -y`，**不带 `-a`**；`-y` 会向所有已检测
  agent 重装/重建链接。
- 因此"只写 `~/.agents/skills`"不成立。任何写操作都必须先枚举主存储 + 各 agent 副本 + lock。

纳管范围（写操作允许触碰的全部路径，超出即拒绝）：

- 主存储：`~/.agents/skills/<install_dir>`；
- 已检测 agent 的全局目录：`<agent.globalSkillsDir>/<install_dir>`；
- lock：`~/.agents/.skill-lock.json`（仅"快照恢复"会写，且只改单条）；
- 项目级 `.claude/skills` / `.agents/skills` **不在纳管范围**，写操作不触碰、也不提供删除入口。

版本展示（不伪造 semver）：

```
来源    vercel-labs/agent-skills
版本    9289507（上游目录树 sha 前 7 位）
安装    2026-04-26
状态    有更新 · 1 个 agent 副本已漂移
```

更新检测：

- 对每个 `sourceType = github` 的条目：解析 `owner/repo`；`ref` 缺省时取仓库默认分支（每仓库缓存一次）。
- 调 GitHub `GET /repos/{o}/{r}/git/trees/{ref}?recursive=1`，取 `skillPath` 所在目录的 tree sha，
  与 lock 的 `skillFolderHash` 比较。
- 优先用 `gh api`（已登录，5000 次/小时）；否则 `curl` 匿名（60 次/小时，需处理 403 与
  `X-RateLimit-Reset`，返回"检查失败，稍后再试"而不是误报"有更新"）。
- 同一仓库复用一次 tree 请求；结果缓存 6 小时；`truncated = true` 时退化为"无法检查"。
- 网络不可用：显示上次结果与时间戳，不清空。

漂移检测：

- 对每个已检测 agent 的 `<globalSkillsDir>/<install_dir>` 与主存储比较：符号链接比较目标；
  独立副本用 `diff -rq --exclude=node_modules --exclude=.git` 比较内容。
- 清单里只**告警**，不自动修复；但**写操作前必须强制展示**：`update` / `remove` 会改写或删除这些
  副本，漂移副本默认拒绝操作，除非用户逐项确认"已备份、允许覆盖"（见 4.6）。
- cc-switch 可能在 devtray 之外同步这些目录；写操作前后检测外部变更，发现即标注
  `external_change`，不假装本次结果权威。

### 4.6 升级 / 删除 / 快照恢复（`src/skills.rs`）

写操作前置（D16 / D21）：

1. **枚举受影响路径**：主存储 `~/.agents/skills/<install_dir>`；每个已检测 agent 的
   `<globalSkillsDir>/<install_dir>`（记录 `kind = symlink | copy | absent`，symlink 记录目标）；
   lock 文件本身。
2. **漂移门禁**：任一副本与主存储内容不同 → 标 `copy_drift`；默认拒绝，用户必须逐项勾选
   "已备份、允许覆盖该副本"才继续。
3. **互斥**：获取 `ops.lock`（`File::try_lock`，持有到恢复结束），与清理、启停共用。
4. **外部基线**：记录 lock 与各路径的 `(ino, mtime, size)`；操作后若变化且非本进程造成 →
   标 `external_change` 并在结果中警告（cc-switch / 用户直跑 CLI 不受本锁约束）。

快照（每次写操作一份，保留 3 份）：

```
skills-backup/<install_dir>/<ts>/
  manifest.json    # 三身份、lock 条目原文、各路径 kind/target、时间、工具版本
  canonical/       # 主存储整目录（跳过 .git、node_modules）
  agents/<agent>/  # 仅 copy 副本；symlink 只记 target，不复制内容
  lock-entry.json  # 该技能的 lock 条目原文（不含其他技能）
```

执行：

- 升级：`{skills_cli} update <names> -g -y`（默认即 `npx --yes skills@1.7.4`），在 tmux 会话
  `devtray-skills` 中执行，输出落日志、面板可 attach。`update` 不支持 `-a`，会向所有已检测 agent
  重装——这正是必须先枚举与备份的原因，不能把它当成"只动主存储"。
- 删除：`{skills_cli} remove <names> -g -y`；不带 `-a` 即删除所有已知 agent 的副本，
  与"彻底卸载"一致，但删除前必须展示全部路径与大小。
- 校验：升级后 lock 条目的 tree sha 等于刚查到的上游 sha、主存储 `SKILL.md` 存在、各 agent 路径
  符合预期（symlink 指向主存储，或 copy 与主存储 `diff` 一致）；删除后 lock 条目消失、路径不存在。
- 失败：保留快照与完整输出，不自动重试，不自动回滚。

快照恢复（不叫"回滚"，D21）：

- **默认：内容恢复**。取同一把锁 → 先把当前状态再快照一份 → 恢复主存储与各 agent 副本/链接 →
  **不改 lock** → 校验 `diff` 与快照一致 → 审计。恢复后该技能标为 `diverged_from_lock`，
  下次更新检查会显示"本地内容与 lock 记录不一致"，这是预期行为，不是错误。
- **可选：完整恢复**（写回该技能的单条 lock 条目）。仅在 lock 条目存在、恢复期间未被外部修改、
  且用户显式勾选"同时恢复版本记录"时提供：再快照 → 恢复文件 → 原子写回单条（临时文件 + rename）
  → 校验 lock 可解析且其余条目字节不变 → 审计。失败保留第二份快照，状态 `restore_failed`。
- 来源未知（无 lock 条目）的技能只能做内容恢复，恢复后仍是 `managed = false`。

### 4.7 Skills 安装（第 6 条）

管线：

```
输入解析 → (裸名字) skills.sh 搜索 → 选定 repo/skill
        → GitHub 预检（SKILL.md、stars、最近提交、license、大小、frontmatter name）
        → 选目标 agent（`settings.skill_agents`，为空则用检测结果；显式 `-a`）
        → tmux 执行 {skills_cli} add <src> --skill <id> -a … -g -y --json
        → 校验 lock 条目 / 主存储目录 / agent 链接
        → 报告（安装位置、链接的 agent、hash、重建命令）
```

输入形式：

| 输入 | 处理 |
| --- | --- |
| `https://github.com/o/r`、`git@github.com:o/r.git` | 归一化为 `o/r` |
| `https://github.com/o/r/tree/<ref>/<path>` | 拆出 ref 与 skill 路径，安装时用 `-s <skill>` |
| `o/r@skill` | 直接定位 skill |
| 裸名字（如 `huashu-design`） | `GET https://skills.sh/api/search?q=` → 候选（按 installs 排序，显示来源与安装量） |
| 本地路径 | 不在本期范围，提示改用 `npx skills add ./path` |

预检内容（全部展示，不自动决策）：仓库 `stargazers_count`、`pushed_at`、`license`、目标
`SKILL.md` 路径、仓库内候选 skill 列表（GitHub tree 中所有 `SKILL.md`）、是否已安装。
候选必须给出**三个名字**：仓库路径、frontmatter `name`、`sanitizeName(name)` 安装目录。
frontmatter 通过 GitHub raw / blob API 读取，只解析首个 `---` 块里的 `name:`（带引号需去引号）；
解析失败时回退为目录名并标注"身份未确认"，不允许在未确认时覆盖已有技能。
同名不同源 → 阻断；同安装目录已存在但来源不同 → 要求显式确认覆盖，并先走 4.6 的快照。

执行与校验：

- 在 tmux 会话里跑（面板关闭不中断；可 attach 看输出；日志落盘）。
- 必须显式传 `-a <agents>`（避免 `-y` 默认装到所有已检测 agent）；CLI 版本固定为
  `skills@1.7.4`，升级 pin 需回归。
- `--json` 结果与 `.skill-lock.json` 对账；校验主存储 `~/.agents/skills/<install_dir>/SKILL.md` 存在、
  `install_dir == sanitizeName(frontmatter name)`、每个目标 agent 的路径符合预期。
- 默认符号链接（`--copy` 仅在用户显式选择时使用），避免再次制造漂移。
- `npx` / `node` 缺失：不假装成功，返回"需要 Node.js，可复制命令自行执行"。
- 安装完成后立即做一次漂移检查；若 CLI 仍生成了独立副本（例如目标 agent 不支持链接），在结果里
  标注 `copy_drift`，不静默视为成功。

### 4.8 MCP 扩展（`src/mcp.rs` + `src/jobs.rs`）

| 工具 | 类型 | 说明 |
| --- | --- | --- |
| `devtray_disk` | 只读 | 全部项目或指定项目的占用、可清理项、扫描时间、完整性 |
| `devtray_agents` | 只读 | 已装 agent、版本、来源、是否有更新 |
| `devtray_skills` | 只读 | 技能清单、来源、hash、更新状态、漂移、受影响路径 |
| `devtray_job` | 只读 | 按 `job_id` 查询状态 / 进度 / 结果；`cancel: true` 取消 |
| `devtray_clean` | 写 | 参数：`project`、`kinds[]` 或 `paths[]`；返回 `job_id` |
| `devtray_skill_install` | 写 | 参数：`source`、`skill?`、`agents[]`；返回 `job_id` |
| `devtray_skill_update` | 写 | 参数：`names[]`；返回 `job_id` |
| `devtray_skill_remove` | 写 | 参数：`names[]`；返回 `job_id` |

生命周期（D22）：MCP 是同步读循环，**写工具不允许阻塞等待审批或执行**。

1. **同步阶段**（毫秒级）：参数校验 → 只读预检（围栏 / 证据链 / 运行状态 / 是否已安装）→ 生成
   计划（含全部路径、命令、`plan_hash`）→ 创建 `job`（`pending_approval`）→ 起后台线程 →
   立即返回 `{job_id, state, plan}`。同一连接随后仍可正常响应 `devtray_status`。
2. **后台审批**：`yad` 弹窗绑定 `job_id` + `plan_hash`，展示动作、确切路径、大小、将执行的命令、
   来源客户端；60 秒无响应 → 拒绝；`yad` 缺失 → 拒绝（fail closed），除非 `mcp_writes = "allow"`。
3. **执行**：批准后重新执行一次前置检查（防 TOCTOU）→ 拿 `ops.lock` → 执行 → 校验 → 写
   `jobs/<id>.json` 终态与审计。拒绝 / 超时 / 失败同样写终态与原因。
4. **查询与取消**：`devtray_job` 返回状态、进度、`result`（实际删除/安装路径清单）或 `error`；
   `pending_approval` 可直接取消，`running` 尽力终止（清理任务按路径逐条检查取消标记）。
5. **幂等与断连**：可选 `client_request_id`；同 client + action + `plan_hash` 在 10 分钟内返回已有
   job，不重复执行。客户端断连后任务继续，结果保留 24 小时。
6. `mcp_writes = "deny"`：写工具直接拒绝（不创建 job）；`"allow"`：跳过弹窗但仍创建 job 并审计。

`devtray_clean` 的 `paths` 仅接受项目根内、证据链通过的目录；拒绝越界路径、符号链接、挂载点、
`dist/build/vendor`（除非配置放开）。MCP 不提供 agent 升级工具（结果不可观测且可能提权）。

### 4.9 面板（`ui/panel.py`）

导航：`总览 / 服务管理 / 资源 / Agent 与 Skills / 日志中心 / 工作台设置`。

**总览**新增一行"资产摘要"：`磁盘占用 18.4G（分配）· 可清理候选 14.2G · 可升级技能 3`，
点击跳转对应页。这是扫描口径的候选值，不是"删除后一定释放"的承诺。指标卡与现有服务卡保持不变。

**资源页**：

- 左侧项目列表（按分配空间降序），右侧选中项目的明细：
  - 顶部：项目根路径、根来源（显式/推断）、总占用（分配 / 逻辑两列）、上次扫描时间、
    "重新扫描 / 打开目录"。
  - 若 `complete = false` 或存在 `mount_skipped`，顶部显示黄色横幅："扫描不完整/含挂载点，
    清理已禁用"。
  - 明细表：类型、路径（相对项目根）、分配空间、逻辑大小、文件数、证据状态（通过/缺 Cargo.toml/
    被 git 跟踪/含挂载点…）、操作（清理 / 打开）。
  - 底部："其他占用 top-5"（只读）。
- 清理交互：勾选（仅 `deletable` 可勾）→ 显示合计（标注"分配空间"）→ 二次确认弹窗（列出**每个**
  路径、大小、重建命令、"运行中服务检测不到用户自己启动的进程"这一限制）→ 后台 job → 结果。
- 加载/取消/失败状态明确；服务运行中、状态不可确认、根过宽时清理按钮禁用并显示原因。
- "扫描任意目录"：只读，选择目录 → 复用扫描逻辑 → 不显示清理按钮。

**Agent 与 Skills 页**（一个页面两个区块，或子 Tab）：

- Agents：名称、二进制路径、当前版本、来源、最新版本、更新按钮（需 sudo 时标注"将在终端执行"）、
  "复制命令"。
- Skills：搜索框、筛选（有更新 / 副本漂移 / 来源未知 / 可否安全写）、表格（名称、来源、
  版本 hash、安装时间、受影响副本数、状态）。
- 写操作确认框必须先列出**主存储 + 每个 agent 副本 + lock** 的路径与动作；存在独立副本漂移时，
  默认禁止提交，需逐项勾选；提交后走 job，可离开页面。
- 安装对话框：输入框 + 搜索候选 + 预检信息（三个名字、stars、license、大小）+ skill 多选 +
  agent 多选 + 确认。
- "快照恢复"入口独立于"升级/删除"，明确标注"会修改该技能的 lock 条目（其他条目不动）"或
  "仅恢复内容"。

**小面板**：不新增页面，只保留服务启停与状态；可选在底部加"可清理候选 14.2G"一行，
点击打开完整面板。

请求层：**长任务（清理 / 安装 / 升级 / 恢复）统一走 job**：子进程立即返回 job id，面板轮询
`_panel-job <id>`（300ms）；扫描单独走流式 JSON 进度（5–10 分钟上限，可取消）。`request()`
的固定 30 秒超时只保留给普通读写，不用于长任务。

**设置页**新增：`mcp_writes`、`clean_extra_dirs`、缓存 TTL、`skills_cli`（固定版本，修改需确认）、
`skill_agents`、审计日志路径。

## 5. 安全与隐私

| 威胁 | 缓解 |
| --- | --- |
| 符号链接逃逸（`node_modules` 指向 `~`） | 不跟随链接；`canonicalize` 后必须落在项目根内 |
| 候选内挂载点（含 bind mount）被删除 | 删除前独立解析 `/proc/self/mountinfo`，候选含挂载点一律拒绝；扫描的 `st_dev` 跳过不算保障 |
| 被 VCS 跟踪的 `target` / `node_modules` 被删 | `git ls-files -- <path>` 命中即拒绝；根过宽时整项目禁用清理 |
| tmux 查询失败被当成"没在跑" | 新接口返回 `Result`；`Err` 一律拒绝清理；审批后与删除前各复检一次 |
| 审批与删除之间的 TOCTOU | 清理与启动/停止共用 `ops.lock`；删除前重放六项检查 |
| Agent 幻觉触发删除 | 默认 `mcp_writes = ask` + yad 审批 + 60s 拒绝；写工具返回 job id 与实际清单 |
| **CLI 隐式改写/删除 agent 副本** | 写前枚举主存储 + 各 agent 副本 + lock；全量快照；独立副本漂移默认拒绝；显式 `-a` 用于 add |
| lock 并发写 / 外部修改 | `File::try_lock` 持有整个操作；操作前后比对 lock 与目录的 inode/mtime，变化即标 `external_change` |
| 恶意 skill 仓库 | 安装前展示来源/星数/提交时间/license；不执行仓库脚本；CLI 自带大小限制；快照可恢复 |
| sudo 提权 | devtray 从不静默提权；只在终端里运行命令，由用户输密码 |
| 配置/环境中的密钥 | 扫描与展示只读尺寸/路径；审计不记录 env 与 token；README 示例不含真实值 |
| 与 cc-switch 双写 | devtray 不碰 cc-switch DB；写操作走固定版本的 CLI；恢复写 lock 前备份、只改一条、校验其余条目未变 |
| GitHub 限流 | gh 优先、缓存 6h、失败显示"未知"而非"有更新" |
| 清理失败留下半删目录 | 删除幂等；报告剩余路径；重跑可继续 |

## 6. 测试与验收

单元（Rust，`cargo test`）：

- 根目录推断：单 cwd、多 cwd 公共祖先、跨顶层回退；根过宽（`/`、`$HOME`、`~/Workspace`）时
  `cleaning_allowed = false`。
- 白名单匹配与剪枝：嵌套 `node_modules` 不重复计数；`dist` 默认拒绝。
- 尺寸统计：稀疏文件（`size=1G, blocks=0`）分配空间为 0、逻辑大小为 1G；硬链接在树内只计一次；
  树外硬链接不改变"分配空间"，且不得据此宣称可回收。
- 证据链：`src/target` 无 `Cargo.toml` 拒绝；`node_modules` 无 lockfile 拒绝；`__pycache__` 无 `.pyc`
  拒绝；`.venv` 无 `pyvenv.cfg` 或只有 `pyproject.toml` 拒绝。
- VCS：`git ls-files` 命中即拒绝（fixture 仓库）。
- 挂载：用 fixture mountinfo 解析器验证"候选含挂载点 / bind mount / 候选本身是挂载点"。
- 运行状态：`snapshot_result()` 的 `Err` 分支必须拒绝清理；跨项目 root 重叠时任一服务运行即拒绝。
- 路径围栏：`../`、绝对路径越界、符号链接指向根外，全部拒绝。
- 技能 lock 解析：v3 正常、缺字段、损坏 JSON、lock 有而目录无。
- 技能身份：frontmatter 名与目录名不一致、`sanitizeName` 边界（大写/空格/首尾点）、同名不同源阻断。
- tree sha 对比：fixture JSON（有更新/无更新/truncated/403）。
- Agent 来源判定：伪造 PATH 与包管理器输出（注入到函数参数，不真的调 pacman）。
- 快照恢复：manifest 覆盖主存储 + 副本 + lock 条目；恢复后其他 lock 条目字节不变；缺条目时仅内容恢复。
- Job：状态机、幂等键、取消语义、断连后仍可查询。
- MCP 参数校验与 `mcp_writes` 分支（`deny` 直接拒；`allow` 跳过审批）。

集成（Python，沿用 `tests/` 风格）：

- `tests/disk_scan.py`：临时项目 + 真实二进制，验证 JSON 字段、缓存、`--refresh`、取消、
  稀疏文件口径、`complete=false` 与 `mount_skipped`。
- `tests/disk_clean.py`：临时目录验证删除、证据链拒绝、git 跟踪拒绝、`snapshot_result()` 失败时拒绝、
  `ops.lock` 争用、审计落盘。
- `tests/skills_write.py`：用假 `npx` 脚本模拟"删除/覆盖 agent 副本"，验证：
  受影响路径完整枚举、副本备份、漂移默认拒绝、`add` 必带 `-a` 与固定版本、恢复后 lock 其他条目不变。
- `tests/skills_inventory.py`：临时 `HOME` + fixture lock，验证清单、未知来源、漂移检测（不联网）。
- `tests/panel_assets.py`：job 轮询、资源页渲染、写操作确认框（假数据）。
- `tests/mcp.py` 扩展：`tools/list` 包含新工具；写工具立即返回 job id；`deny` 拒绝；`allow` 跳过审批；
  `devtray_job` 查询与取消。

验收标准（每条需求）：

1. 资源页能看到每个项目的 `target` / `node_modules` 分配空间与逻辑大小，13G 项目冷扫不阻塞 UI，
   可取消、可缓存；扫描不完整时明确标注。
2. 任意项目/服务的"打开目录"能在文件管理器中打开正确路径。
3. 清理前必须看到路径、分配空间与证据状态；任一项证据失败或运行状态无法确认时无法清理；
   删除后只报目录是否消失，不承诺释放字节；审计可查。
4. Agents 页正确显示 claude（native）与 codex（pacman）的版本与来源；升级路径符合来源。
5. Skills 页正确显示 31 个有元数据 + 11 个来源未知；"有更新"判断可通过固定 fixture 复现；
   升级/删除前列出主存储 + 各 agent 副本 + lock，全部快照，可恢复到操作前；副本漂移默认拒绝。
6. 输入 `alchaincyf/huashu-design` 或裸名字 `huashu-design` 能搜到、预检（三身份）、安装、校验。
7. MCP 写工具立即返回 job id，审批与执行不阻塞状态查询；断连后任务继续且可按 job id 查询结果。

## 7. 实施计划

### v0.2 资源（约 10–13 人日）

先做"规则与接口"，再做 UI；规则没被单测钉住之前不接清理按钮。

| 步骤 | 内容 | 主要文件 | 出口 |
| --- | --- | --- | --- |
| 0 | 状态接口 `snapshot_result()`、`ops.lock`、job 骨架、六项检查的纯函数判定器 | `src/tmux.rs`、`src/jobs.rs`（新）、`src/guard.rs`（新） | 单测覆盖六项检查；不接 UI |
| 1 | 配置 `root` + 推断 + 根过宽禁用清理 | `src/config.rs` | 单测通过，旧配置无变化 |
| 2 | 扫描核心（分配/逻辑、inode、挂载跳过、完整性、进度） | `src/disk.rs`（新） | 单测 + 13G 实测 |
| 3 | CLI `disk scan/open` + JSON + 流式进度 | `src/main.rs`、`src/cli.rs` | 人类输出与 JSON 一致 |
| 4 | 清理 + job + 审计 + 挂载/VCS/证据检查 | `src/clean.rs`（新） | `disk_clean.py` 通过 |
| 5 | 面板资源页 + 总览摘要 + job 轮询 | `ui/panel.py` | 1040×720 不挤爆 |
| 6 | MCP 只读 + 清理 job + 审批 | `src/mcp.rs` | `mcp.py` 扩展通过 |
| 7 | 文档（双语 README、CHANGELOG）、打包说明 | `README*.md`、`CHANGELOG*.md` | `cargo test/clippy/fmt` 通过 |

### v0.3 Agent & Skills（约 12–15 人日）

写操作在"假 CLI 桩 + 隔离 HOME"下全部通过后才接真实目录；开发期不动用户的真实 skills。

| 步骤 | 内容 | 主要文件 | 出口 |
| --- | --- | --- | --- |
| 0 | 固定 CLI 版本 + 假 `npx` 桩 + lock 解析 + 三身份 | `src/skills.rs`（新）、`tests/` | 身份/同名/解析单测 |
| 1 | Agent 注册表 + 检测 + 来源 + 版本 | `src/agents.rs`（新） | 本机识别 claude/codex |
| 2 | Skills 清单 + 副本枚举 + 漂移 + 更新检测（gh/curl、缓存、限流） | `src/skills.rs` | 42/31 对账正确 |
| 3 | 全路径预览 + 快照 manifest + job 化 | `src/skills.rs`、`src/jobs.rs` | 副本全覆盖，可恢复 |
| 4 | 升级/删除（固定版本、隐式 agent 目标、备份、校验） | `src/skills.rs` | `skills_write.py` 通过 |
| 5 | 快照恢复（内容 + 副本 + 单条 lock） | `src/skills.rs` | 恢复后其余条目不变 |
| 6 | 安装管线（搜索、预检三身份、显式 `-a`、tmux、校验） | `src/skills.rs`、`src/tmux.rs` | 在隔离 HOME 装一个真实 skill |
| 7 | 面板两页 + 写确认框 + 快照恢复入口 | `ui/panel.py` | 交互完整 |
| 8 | MCP 读写 + job 生命周期 + 审批 + 审计 | `src/mcp.rs` | 审批/去重/取消测试 |
| 9 | 文档与发布 | `README*.md`、`CHANGELOG*.md` | 打包检查通过 |

### v0.4（可选，约 2–3 人日）

- AI Advisor：OpenAI 兼容端点（base_url / model / key 来自 env 或配置），**只接收结构化事实**
  （技能名、来源、安装量、更新时间），不读取 SKILL.md 正文；输出排名理由文本；默认关闭。
- 家目录共享缓存清理（`uv cache clean` / `pip cache purge` / `npm cache clean`），独立确认。
- 项目级 skills 的只读展示 → 是否管理，另行评审。

## 8. 依赖与发布影响

- **不新增 Rust crate**：遍历用 `std::fs`，互斥用 `std::fs::File::try_lock`（Rust 1.97 已稳定），
  HTTP 走 `gh`/`curl`，JSON 用现有 `serde_json`。
- 新增外部命令（全部可选、缺失时降级）：`gh`（推荐 GitHub 登录）、`curl`、`node`/`npx`、
  `xdg-open`、`diff`；已有 `tmux`、`yad`。
- Skills 写操作固定 `npx --yes skills@1.7.4`；升级 pin 必须跑 `skills_write.py` 与隔离 HOME
  安装回归，并在 CHANGELOG 说明。
- 打包：deb/rpm 建议把 `nodejs`、`gh`、`curl` 放进 `Recommends`（不是硬依赖），MCP 审批依赖
  `yad`（缺失时 fail closed）。
- 文档：中英文 README 增加两个模块的章节；`CHANGELOG` 记录 v0.2 / v0.3；`docs/control-panel.md`
  的信息结构章节同步更新。

## 9. 风险登记

| 风险 | 概率 | 影响 | 缓解 |
| --- | --- | --- | --- |
| 13G 目录扫描拖慢面板 | 高 | 中 | 独立进程 + 并发 2 + 缓存 + 取消 + 进度 |
| 扫描不完整/含挂载点仍被清理 | 中 | 高 | `complete=false` 禁用清理；删除前独立查 mountinfo |
| pnpm 硬链接 / 稀疏文件导致口径错误 | 中 | 中 | 分配与逻辑分列；inode 去重；不承诺可回收量 |
| 用户确认后误删 `dist` 或被 git 跟踪的目录 | 中 | 高 | 默认排除；证据链 + `git ls-files`；根过宽禁用 |
| **skills CLI 改写/删除 agent 副本** | 高 | 高 | 全路径预览 + 全量快照 + 漂移默认拒绝 + 显式 `-a` |
| tmux 查询失败被当成未运行 | 中 | 高 | `Result` 接口；`Err` 拒绝；审批后复检 |
| 恢复写 lock 破坏其他条目 | 低 | 高 | 只改一条 + 原子写 + 其余条目字节校验 + 二次快照 |
| `skills` CLI 版本漂移 | 中 | 高 | 固定 1.7.4；升级需回归；`skills_cli` 覆盖时标注未验证 |
| lock 格式升级（v4） | 低 | 中 | 解析失败时退化为"来源未知"，不阻断页面 |
| GitHub 限流 | 中 | 低 | gh 优先、缓存、失败显示未知 |
| cc-switch / 用户直跑 CLI 并发写 | 中 | 中 | 操作前后比对 inode/mtime，标 `external_change`；不假装权威 |
| sudo 升级体验割裂 | 高 | 低 | 终端执行 + 复制命令；不假装自动化 |
| MCP 审批弹窗被忽略 | 低 | 高 | 60s 默认拒绝 + job 终态 + 审计 |

## 10. 未决 / 后续

- **完整恢复默认关闭**：当前设计默认只恢复内容、不改 lock；写回单条 lock 作为显式高级选项。
  v0.3 上线后按实际使用反馈决定是否改变默认值。
- 固定 CLI 版本的升级节奏：谁触发、多久评估一次、回归清单是否包含真实安装。
- 与 cc-switch 的并发协调：目前只做检测与告警，不共享锁；是否需要在 skills 写操作前提示用户
  暂停 cc-switch 的自动同步，待实测冲突频率后再定。
- 项目级表单（编辑 `root`）——v0.2 只手改配置，面板展示即可。
- 家目录共享缓存清理——独立功能，需逐个工具确认命令与回滚。
- Windows / macOS——当前架构（ksni 托盘、tmux、xdg-open、deb/rpm）不支持；扫描模块可移植，
  但跨平台需单独立项。
- AI provider 选型（本地 Ollama vs 云端小模型）——v0.4 前再定；本机目前无本地运行时。
- 技能版本是否引入"项目级 lock"（`skills-lock.json`）——本期只做全局。
- 是否给 cc-switch 提供只读集成（读取其 DB 的 agent 列表）——目前独立检测，避免耦合。

## 11. 复核响应（2026-10-28）

| 复核项 | 结论 | 落地位置 |
| --- | --- | --- |
| P1 CLI 会修改承诺只读的 agent 副本 | 成立。`remove` 默认遍历所有 agent；`update` 重新 `add -y` 到所有已检测 agent | 1 节事实、4.5 纳管范围、4.6 全路径枚举/备份/漂移拒绝、D16 |
| P1 目录名白名单不能证明可删 | 成立。新增证据链、VCS 跟踪检查、根过宽禁用；`src/target` 不再可删 | 4.2 六项检查与证据表、3.1 禁用条件、D7 |
| P1 状态快照不足以硬阻断 | 成立。新增 `Result` 接口、`ops` 跨进程锁、重叠项目检查、审批后复检 | 4.2、D17、测试（`snapshot_result` Err 分支） |
| P1 扫描跳过挂载点、删除却进入 | 成立。删除端独立解析 mountinfo（含 bind mount），扫描跳过不作保障 | 4.2 检查 5、D18、测试（mountinfo fixture） |
| P1 只恢复目录不构成完整回滚 | 成立。改为"快照恢复"：默认内容 + 副本（不改 lock），完整恢复为显式选项 | 4.6、D21、10 节 |
| P1 不能凭锁年龄解除互斥 | 成立。改用 `File::try_lock` 持有全程；不按年龄判陈旧；另做外部修改检测 | 3.2、4.6、D16 |
| P2 尺寸算法误报 | 成立。分配/逻辑分列；`blocks=0` 不回落；不承诺释放字节 | 4.1、D19、测试（稀疏/外部硬链接） |
| P2 目录名不等于 CLI 技能名 | 成立。三身份分列；frontmatter 读取；同名阻断 | 3.3、4.5、4.7、D20 |
| P2 MCP 缺少长任务契约 | 成立。写工具立即返回 job id；审批/执行后台化；`devtray_job` 查询/取消；幂等键 | 4.8、D22、验收 7 |
| 建议固定 CLI 版本 | 采纳。默认 `npx --yes skills@1.7.4`，升级 pin 需回归 | 3.1、4.6、4.7、8 节、D23 |

## 12. 实施进度（2026-10-28）

已完成（v0.2 扫描 + 清理门禁）：

- 配置与扫描：`[[project]] root` + 公共祖先推断 + 根过宽禁用；白名单任意深度命中即停；
  分配/逻辑分列；inode 去重；不跟随符号链接；跨设备与 mountinfo 挂载点跳过并标记不完整；
  证据链（Cargo.toml + 构建痕迹、package.json + lockfile、pyvenv.cfg + uv.lock/requirements、
  `.pyc`、package.json）；`git ls-files` 跟踪检查；其他占用 top-5；JSON 输出。
- 清理：六项检查（根围栏 / 白名单 / 证据 / VCS / 挂载 / 运行状态）在确认前与取锁后各执行一次；
  `ops.lock` 用 `File::try_lock` 持有全程，CLI、托盘、MCP 的启停路径全部改走带锁的 `cli` 包装；
  job 模型落盘；审计 JSONL（10 MiB 轮转）；`_clean` / `_job` 入口。
- 缓存与面板：`disk_cache_ttl_s`（默认 600s）、`_disk-cache`、`_disk-scan --refresh`；
  面板侧栏新增「资源」页（项目列表、占用表、勾选清理、扫描全部/并发 2、job 轮询、打开目录），
  总览新增资产摘要；日志页索引为 4（Agent 与 Skills 页加入后）。
- 面板接口：`_clean-plan`（只出计划 + 六项检查）、`_clean-start`（建 job 并后台 `_clean`，
  子进程输出重定向，避免污染 JSON）。
- MCP：新增 `devtray_disk`（只读缓存）、`devtray_job`（查询/取消）、`devtray_clean`
  （创建 job，默认拉 `_approve` 弹 yad，60 秒超时拒绝，`mcp_writes` 为 deny/allow 时进入对应分支）；
  `_approve` 与 `_clean` 都由独立进程执行，MCP 连接不被 60 秒审批阻塞。
- v0.3 只读：`src/agents.rs` 检测 claude/codex/pi/dsh/gemini/opencode/qwen/aider 的版本、来源与建议升级命令；
  `src/skills.rs` 对账 lock v3 + 主存储 + agent 副本（symlink 目标 / copy diff），面板新增
  「Agent 与 Skills」页（Agent 表、Skills 表、筛选、漂移状态），CLI 新增 `agents` / `skills` /
  `skills open`。本机实测 43 个技能（纳管 32 / 来源未知 11 / 副本漂移 1）。
- Skills 更新检查：`devtray skills check [--refresh]` 与面板「检查更新」用 GitHub tree sha
  对比上游，结果缓存 6 小时（`skill_check_ttl_s`）；`gh api` 优先、匿名 `curl` 兜底，
  限流/失败标 `unknown` 而不是误报；缓存落 `state/cache/skills-updates.json`。
- Skills 写门禁（升级/删除）：`skills upgrade|remove`、`_skills-plan` / `_skills-start` /
  `_skills-run`；固定 `npx --yes skills@1.7.4`，枚举主存储 + 各 agent 副本 + lock，快照到
  `state/skills-backup/<技能>/<时间>/`（保留 3 份）；副本漂移默认拒绝，需 `--allow-drift`；
  执行前重新 plan + `ops` 锁，执行后校验目录/lock，失败保留快照；job 与审计复用清理那套。
  面板 Skills 表支持「升级选中 / 删除选中」，漂移会二次确认。
- 快照恢复：`skills restore <name> [--snapshot ID] [--restore-lock]`、`_skills-backups` /
  `_skills-restore`；默认还原主存储 + agent 副本、不改 lock；`--restore-lock` 原子写回单条
  lock 条目并校验其余条目语义不变（文件会重新序列化）。面板「恢复快照」列出最近 3 份供选择。
- Skills 安装：`skills install <来源>`、`_skills-install-plan` / `_skills-install-start`；
  支持 GitHub URL / `owner/repo@skill` / 裸名字（skills.sh 搜索唯一命中）；GitHub tree 枚举
  `SKILL.md` 候选，已有同名默认拒绝（`--allow-existing` 放开）；默认使用检测到的 agent，
  也可显式 `-a`；安装后校验 lock、主存储与每个 agent 副本。面板「安装技能…」复用同一套计划。
- CLI：`disk [项目] [--json] [--refresh]`、`_disk-scan`、`_disk-cache`、`disk open`、
  `disk clean`、`_clean-plan`、`_clean-start`、`_clean`、`_job`。
- 测试：Rust 62 条全绿；`tests/disk_clean.py` 端到端覆盖 dry-run、证据删除、审计/job、
  符号链接拒绝、git 跟踪拒绝、ops 锁互斥、`_clean` 任务执行；`tests/panel_assets.py`
  覆盖资源页渲染、勾选启用清理按钮、以及"面板 → `_disk-cache` → `_disk-scan`"真实链路。
  `tests/mcp.py` 覆盖九个工具、磁盘缓存读取、job 缺失与 `mcp_writes=deny`；
  `_approve` 的 allow / deny / yad 缺失（fail closed）在 `tests/disk_clean.py` 中验证。
  `tests/agents_skills.py` 覆盖 Agent 版本/来源与 Skills lock 对账、未纳管、副本漂移；
  `tests/skills_check.py` 用假 `gh` 覆盖 tree sha 对比、available/current、6h 缓存与
  `--refresh` 绕过；`tests/skills_write.py` 用假 `npx` 覆盖计划、漂移拒绝、快照、升级、删除、
  `_skills-start` worker、lock 校验，以及恢复内容/恢复 lock；`tests/skills_install.py` 用假
  `gh`/`npx` 覆盖安装计划、候选选择、显式 agent、安装后校验与同名拒绝；`tests/panel_assets.py`
  覆盖 Agent/Skills 表格、漂移/更新状态、筛选、检查更新与升级/删除按钮接线。
  既有 Qt 回归 `tests/test_panel.py` 15 条通过（日志页索引已同步）。

实测：

- 扫描：`simple-oa` 85.7 GiB / 4.1s；`hb_scf` 8.2 GiB / 0.28s；`local-agent-studio` 14.6 GiB / 3.2s。
- `disk clean simple-oa --kind node_modules --dry-run` 正确列出 991.7 MiB 计划，未删除。
- 临时项目端到端：`--yes` 删除 32 KiB 目录，job 状态 `done`，审计记录 `result: ok`。
- 持锁并发验证：外部 `flock` 占住 `ops.lock` 时，清理在 5 秒后拒绝，目录保持原样。
- 真实 tmux 会话 `devtray-e2e2-frontend` 运行时，dry-run 被拒绝（"服务正在运行"），目录未动。
- 取消任务后 `_job --cancel` 状态为 `cancelled`，`_clean` 拒绝重复执行。
- `.cargo-home/.../src/target` 这类同名源码目录会被识别为候选，但因证据不通过标为只读。
- `killtower` 的根仍推断到 `tools/content-editor`（单 cwd），需要显式配置 `root` 才是仓库根。
- Skills 更新检查实测（临时 state，38s）：43 个技能 → 有更新 24、已最新 5、无法检查 3、未检查 11。
  3 个 `unknown` 用 `gh api` 核对上游 tree，确认是目录已删除/改名（`baoyu-image-cards`、
  `baoyu-imagine`、`diagnose`），不是解析失败。

未做：流式扫描进度（当前是单次同步 QProcess，600s 上限 + 可取消）、Agent 升级执行、
项目级 Skills 管理。
README 双语已补充 `root`、`disk_cache_ttl_s`、`mcp_writes`、资源页 / Agent 与 Skills 命令及只读边界。


## 13. 2026-10-29 交互修复

资源清理的面板调用已修正为项目根下绝对路径，修复“解析失败”。Agent 检测补查用户安装目录并增加 Kimi/Hermes；版本读取兼容 stderr，单个版本命令有两秒上限。Agent/Skills 右键菜单已接入，副本详情降为次级信息，写保护保留。Agent 重检本地版本与 Skills 上游 tree hash 检查分开，不承诺通用 Agent 更新检查。

新增清除提示和按配置隔离的托盘告警确认信号；确认后不改真实服务状态。总览批量启停按钮已移除。三列资源页已按用户选定的松绿色方向迁入 Qt，详见第14节。

新增验证与边界见 control-panel.md 的2026-10-29小节；设计文件见 design-demos/。


## 14. 松绿色方案已落地（2026-10-29）

修订A方案已迁入真实Qt面板，保留左侧项目/服务树；资源页为项目、目录、跨项目待清理清单三列。所有项目先预检并统一确认，再串行执行；失败停止后续项目，未完成选择保留。选择随扫描结果复核，根变化或证据失效时丢弃过期选择。浅色方向C保留为后续设计备选。实现、截图和验证入口见control-panel.md最后一节。
