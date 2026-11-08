# DevTray 跨平台开发设计：原生环境优先，WSL2 可选

状态：待实施的设计。目标是让开发者知道“这个项目实际用的是哪套工具和配置”，并且能在**同一环境**中管理服务。首发架构为 Windows x64、macOS arm64，保留现有 Linux x86_64。本文取代旧构建指引中“Windows 首版依赖 WSL2”的产品决策。已有的 desktop-probe 是移植探针，不是支持声明。

## 1. 从用户操作定义边界

- **只用 Windows 的人**：装一个 Setup.exe，打开即看到“Windows 本机”，可以采集 Git、运行时、Agent、Skills、项目和服务信息，在本机启停 PowerShell 或可执行程序。安装、首次启动、刷新和升级均不要求 WSL、tmux 或系统 Python；未启用 WSL 时不调用 wsl.exe。
- **两种环境都用的人**：在“环境 → 添加 → WSL2”选择已有发行版，确认后才探测该环境、部署后端。界面可以切回本机。不安装发行版、不修改系统默认发行版，不把原生项目的命令改交给 WSL。删掉环境连接只移除本地登记，不删除发行版、项目或配置。
- **Linux / macOS 用户**：默认只有本机。缺 tmux 时仍能采集环境和查看项目；仅服务托管显示缺少依赖。Linux 保留原有 CLI、桌面托盘和安装布局。Mac 用户从 Finder 和终端打开应用，应能看到实际执行服务所用的工具路径。

**切换的是视图，新建项目归属当前环境。** 已有项目、后台服务、排队任务和日志仍归原环境。项目迁移是单独的显式操作，首版不做自动迁移。例：Windows 的 api 正在运行，切到 Ubuntu 后即使也有 api，点“停止”只能影响 Ubuntu 的 api；切回本机，原服务仍在。

## 2. 分开“操作哪个环境”和“用什么托管服务”

| 目标环境 | 命令在哪里执行 | 如何采集 | 如何托管服务 |
| --- | --- | --- | --- |
| Windows 本机 | 本机 Rust CLI，MSVC x64 | Windows 用户目录、工具发现和文件系统实现 | 用户级 DevTray supervisor + Windows Job Object |
| Windows 上的 WSL2 发行版 | 经 wsl.exe 调用该发行版内的 Linux CLI | 在发行版内复用 Linux 采集器，注明可能来自宿主 PATH 的工具 | 发行版自己的 tmux |
| Linux 本机 | 本机 Rust CLI | 延用现有采集器 | 延用 tmux |
| macOS 本机 | 应用内置 Rust CLI | Mac 用户目录、工具查找和文件系统实现 | tmux；菜单栏由 Qt 接入 |

建议调用顺序：UI / CLI / MCP → 确定环境 ID → 本机直接执行或 WSL 转发 → 环境内采集器 / 服务后端。**路由不读取工具版本，采集不启动服务，面板不拥有服务进程。** 即使 supervisor 停止或 tmux 未安装，版本和路径仍可采集。先在 Rust CLI 实现路由，Qt 只传操作和环境 ID，避免 MCP 与桌面向不同目标发命令。

宿主登记稳定的环境 ID、类型、显示名、发行版名和目标配置位置。不把用于展示的名称当主键。每个环境继续读取自己的 config.toml；服务、作业、告警、缓存和日志以“环境 ID + 配置标识 + 本地对象 ID”定位。配置路径、工作目录、工具路径都按所属环境解析；仅在“打开目录／选择文件”边界转换为桌面能用的路径。不要在 Windows 上用 Path.resolve() 规范化 Linux 路径。异步请求携带环境 ID 与视图代次，切换后丢弃旧视图的迟到结果；已发出的写操作保持原目标，可在任务记录查看。

## 3. 环境采集先行

先交付只读的环境页：**工具、解析到的路径、版本、数据来源、最近刷新时间**。同一个工具在 Windows 和 Ubuntu 版本不同是正常结果。区分“没有找到”“无法探测”“不支持”“过期快照”；版本探测失败不等于工具没安装。下面是拟新增的命令和返回示例，并非当前可执行接口：

```text
devtray env list --json
devtray --environment local inventory --json
devtray --environment <已登记的环境ID> status --json
```

```json
{"schema_version":1,"environment_id":"local","platform":"windows","tools":[{"id":"git","status":"found","path":"C:\\Program Files\\Git\\cmd\\git.exe","version":"2.x","source":"path"}],"capabilities":{"inventory":true,"service_control":true,"terminal_attach":false,"disk_clean":false}}
```

| 采集对象 | 实现要求 | 验收案例 |
| --- | --- | --- |
| 工具链与 Agent | 先覆盖 Git、Rust/Cargo、Node/Bun 及现有 agents.rs 清单。发现与执行共享解析器；记录最终路径，区分 .exe、.cmd 包装脚本、符号链接及未知来源。探测限定超时和输出长度。 | 同机装两版 Git；显示的路径必须是该环境执行项目 Git 操作时使用的路径。单个工具卡住不拖死整页。 |
| 项目、Git、Skills | 按目标环境读取配置、项目根和用户目录。区分“只读清单”和安装、更新、写入能力。 | Windows 和发行版各有同名项目与 Skill，清单分别显示；修改一个不改变另一个。 |
| 资源与就绪状态 | 首版支持有证据的指标。磁盘逻辑大小、实际占用和不可测分开；探活在服务所在环境执行，宿主浏览器能否访问单独判断。 | 不能判断挂载边界、junction 或重解析点时，后端拒绝清理；不可测不显示为零。 |

默认只收集为上述字段所需的证据；不导出全量环境变量、令牌或 Git 凭据。缓存按环境隔离，显示采集时间，支持手动刷新。发行版离线时可以显示上一次快照，必须标为过期，不自动启动已关闭的发行版来填充总览。

## 4. 服务编排与平台动作

**Windows 原生。** 独立用户进程持有服务句柄，UI、CLI 和 MCP 通过限当前用户访问的命名管道调用它。每次运行生成 run ID，保存项目、退出码、开始结束时间与日志路径；PID 仅作展示。先挂起创建进程、纳入对应 Job、再恢复执行，任何一步失败都清理未接管的进程。禁止允许子进程自行脱离 Job 的设置。普通退出记录退出码；停止优先执行服务支持的正常退出方式，超时才结束该 Job。不要按端口或进程名杀进程。[Microsoft：Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)

首版明确生命周期：关窗口、退托盘都不停止服务；主动“停止服务”才停止。supervisor 异常退出则利用最后一个 Job 句柄关闭时终止托管树，重启后将对应运行记录标记为中断，保留日志，不假装已重新接管。并发 start 必须串行判定；重复 stop 可安全重试。先支持 stdout/stderr、轮转、ready 正则和端口，不承诺交互式重连；有真实需求再加 [ConPTY](https://learn.microsoft.com/en-us/windows/console/pseudoconsoles)。

**命令契约。** 旧 Unix 配置的 cmd 保持原 shell 语义；不把含 export、管道或引号的命令自动翻译成 PowerShell。新增“程序 + 参数数组”形式与显式 shell 形式，配置互斥校验；env、cwd 独立传给子进程。Windows 新建项目默认推荐直接执行 .exe；脚本或 .cmd/.bat 由对应 shell 处理，界面显示实际解释器及缺失原因。路径需覆盖空格、中文、盘符和 UNC。服务配置迁移由用户选择目标路径与 shell 后预览。

**WSL2 可选。** 用户主动添加时列出已装发行版并核验 WSL 版本、默认用户、CPU 架构和后端协议版本。只在所选发行版用户目录放置匹配的 Linux CLI；通过固定命令和分离参数调用，显式传入发行版内配置路径，不拼接用户输入到 shell。后端缺失或离线只影响该连接，禁止静默回退到本机执行。首版可让用户选择同版本离线组件包，校验版本和发布清单的摘要后部署；不要把 WSL 组件下载设为本机启动前置条件。[Microsoft：WSL 命令](https://learn.microsoft.com/en-us/windows/wsl/basic-commands)

**平台桌面。** Windows / macOS 复用现有 PySide6 面板和 Qt 托盘；浏览器、文件管理器、通知由宿主打开。日志导出位置属于宿主，日志来源属于目标环境。Linux 保留 ksni。将旧托盘隐含的自动启动、状态刷新和失败提示逐项指派给 supervisor 或桌面入口，不要因换托盘重复启动服务。Mac 处理 Finder 的 PATH 差异，可配置可信工具绝对路径；缺工具时展示实际查找位置。

## 5. 仓库改造清单

| 现有入口 | 要改什么 | 兼容约束 |
| --- | --- | --- |
| Cargo.toml、src/main.rs、tray.rs、icons.rs | ksni、Linux 桌面类型及其导入按目标编译；拆出无需托盘的 CLI。 | 配置引用的图标名等纯数据仍可跨平台使用。 |
| src/config.rs、cli.rs、tmux.rs、ops.rs、logs.rs、jobs.rs | 从 Target 中分离逻辑服务标识和 tmux 会话名；把状态、启动、停止、日志、就绪探活改为平台后端接口。Windows 单独实现 supervisor。 | 保留现有 status JSON 状态 token、日志筛选与 Linux 的会话命名；新增字段不替换已有类型。 |
| src/agents.rs、skills.rs、git.rs、disk.rs、clean.rs、terminal.rs | 抽出工具解析、用户目录、权限、链接和卷边界处理。检查 Unix 权限位、冒号切 PATH、/proc 和 xdg-open 假设。 | 未实现的操作返回明确原因；只读能力不因对应写能力缺失而被禁用。 |
| src/panel.rs、ui/panel.py、ui/git_panel.py | 替换“tmux_available 决定能否启动”为能力判断；统一带环境的请求入口，梳理目录按钮、文件选择器与单实例通信。 | 一个宿主面板可切换多个环境；唤醒请求携带目标环境。Linux 原面板启动方式继续可用。 |
| src/mcp.rs、审批路径 | MCP 会话默认绑定启动时的环境，不跟随 UI 切换；工具执行与结果使用同一目标。没有图形审批实现时，ask 策略拒绝写入。 | 旧调用无环境参数时按本机处理，stdout 只输出协议。失败不得改投其他后端。 |

建议先保留多个环境各自的配置文件和现有标识，再新增宿主环境登记；不批量重写用户配置。Linux 老用户升级后仍读原路径；旧 Windows WSL 试验配置若存在，仅可显式导入为 WSL 连接。

## 6. 实施顺序与发布门槛

| 交付单元 | 依赖 / 内容 | 可审查的通过证据 |
| --- | --- | --- |
| PR1 平台与环境契约 | 基于已有 CI 探针；拆开 Linux 编译依赖、共享类型与平台能力；定义 local 和环境绑定。 | Linux 原测试不回退；Windows MSVC x64 和 Mac arm64 实际编译 CLI。确认不是用空实现报告“支持”。 |
| PR2 原生采集 | 接 PR1；先做 Windows，再复用到 Mac。打通工具路径、版本、项目、Git 和 Agent 清单。 | 无 WSL、无 tmux 的 Windows 可以完成采集；故意在 PATH 放置会失败的 wsl.exe 哨兵，断言整个原生流程没有调用它。 |
| PR3 服务后端 | 接 PR1；Windows supervisor 与 macOS tmux 适配可并行。 | 真实启动“父进程 → 监听端口的子进程”；关闭再打开面板可找回，停止后端口释放；强制 supervisor 退出后无残留，日志可读。Mac 检查 tmux 重启前日志归档。 |
| PR4 桌面与原生预览包 | 接 PR2/3；设置默认本机，接回能力、托盘、通知；在对应 runner 构建并安装测试。 | Windows 无 WSL、无系统 Python，Mac 无系统 Python 且有 tmux，均可从安装位置完成采集和服务生命周期。 |
| PR5 可选 WSL 与切换 | 接 PR4；增加发行版连接、Linux 载荷、路径边界和隔离。 | 同名服务分别运行在 Windows 与 Ubuntu；切换时旧响应不污染新视图，重启操作只命中选中服务。禁用连接后 Windows 功能继续工作。 |

CI 保留现有 verify → linux-packages。分别增加 windows-native-check 和 macos-check；windows-package **只依赖原生校验，不依赖 linux-packages**。Windows 安装包名为 DevTray-<版本>-Windows-x64-Setup.exe，macOS 为 DevTray-<版本>-macOS-arm64.dmg。可选 WSL 组件另行取同一提交的 Linux 产物，独立测试和分发；真实 WSL2 的测试机不可用时标为“未验证”，不影响已验证的原生版发布。GUI 的模拟测试不能替代实际安装后检查。

首个原生预览包不等待 WSL、交互终端或跨平台磁盘清理。上传安装包、SHA-256、平台能力表、捆绑 Python/Qt/插件的第三方声明、Actions run 和安装验证记录；公开发行前再走签名、公证及最终包复验。验收的核心问题始终是：**显示的环境信息，是否就是执行这次操作的那套环境？**
