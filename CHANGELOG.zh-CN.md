# 更新日志

## 未发布

### 日志分诊
- 新增 `devtray logs --digest` 与 MCP `devtray_logs { digest: true }`：确定性分诊，把几百行日志压成若干条问题，每条带错误码、`文件:行:列`、出现次数和最多两条样例。
- 保留一行「运行结束上下文」（`The system will reboot now!`、`^C [ELIFECYCLE] ...`），并把请求端点（`path=/api/auth/login`、vite 代理路径）提进问题条目，避免把关机和接口报错误判成代码缺陷。
- 识别 Rust 编译错误、panic、端口占用、Python traceback、JavaScript/Bun 报错和包管理器失败（`[ELIFECYCLE]`、`npm ERR!`），其余回退到日志级别分类；摘要会剥掉 tracing 的 span 前缀；重复项按去掉数字、时间戳与 ULID/UUID/哈希后的内容合并，按出现次数排序，最多展示八类，其余只报数量。默认扫描最后 300 行，全程不调用模型。

### Git 接入 MCP
- 新增 `devtray_git_status`、`devtray_git_diff`、`devtray_git_commit`、`devtray_git_push`、`devtray_git_pull`，复用 Git 页与 `devtray git` 的同一套受控函数。
- 写操作按 `mcp_writes` 执行：`deny` 直接拒绝；`allow` 立即执行并返回提交哈希／推送摘要／拉取结果；`ask` 创建任务、在桌面弹 yad 确认（60 秒超时视为拒绝，`yad` 缺失按拒绝处理），结果通过 `devtray_job` 查询。
- 提交只接受当前变更列表里的文件并在暂存前重新读取状态；推送不做 force；拉取只做 `--ff-only`，默认跳过有未提交改动的仓库（可用 `allow_dirty` 放开）。

## 0.2.0 - 2026-11-04

### 交互修复
- 细化边框、按钮与焦点状态，增加进行中、成功和失败反馈；新增窗口内快捷键及设置覆盖、冲突校验、禁用、恢复默认与持久化，后台刷新保留编辑草稿。
- 将已读按钮放到服务异常状态旁，项目标题提供仅本项目的异常已读入口；日志中心新增异常处理列表、项目及已读状态筛选、逐条日志与已读操作；保留暂停日志选区，避免旧状态响应覆盖已读后的刷新。
- 修复保留的异常退出无法独立标记已读：服务卡片、小面板与托盘服务菜单提供入口，新增 `devtray ack`，按当前失败进程持久保存；新失败重新告警，过期界面确认不会误读新一轮失败。

- 落实选定的松绿色Qt工作台，保留全局项目/服务树、搜索与折叠状态；浅色原型留作备用。
- 资源页改为项目/目录/待清理清单三列，支持跨项目保留、移出、清空、统一预检确认与串行清理；失败停止后续任务。
- 扫描变化使不安全选择失效；任务查询失败保留job并重试；清理与Skills计划互斥。
- 将主题、资源UI与选择模型拆为内嵌模块，通过stdin启动界面，补充工作区回归和CI。

- 增加清除提示与托盘告警确认；保留真实服务状态，新失败重新告警。
- 修复资源勾选把相对路径传入清理接口导致“解析失败”。
- Agent 检测补查用户安装目录、读取 stderr 版本并限制探测时长；增加 Kimi、Hermes。
- Agent/Skills 支持右键与普通操作菜单；副本详情降为次级信息，明确更新检查范围。
- 移除总览项目批量启停按钮；服务管理保留批量操作。


### Git 工作区
- 新增完整面板「Git」页与 `devtray git`：按项目根识别仓库（同一仓库去重），查看分支、领先／落后、变更与最近提交，并预览 diff。
- 新增受控写操作：只提交勾选文件（后端执行前复核状态）、推送到已有上游（绝不 force）、`git pull --ff-only`；提交信息支持多行与 `Ctrl+Enter`。
- 新增「拉取全部项目」后台任务：逐仓库拉取，跳过有未提交改动、正在变基／合并、没有上游、非 Git 仓库或仓库根过宽的项目；完成后汇总「已更新 / 已最新 / 跳过 / 失败」并可查看逐项原因。
- 所有 Git 命令禁用交互提示（HTTPS 终端提示、SSH BatchMode）；仓库根是 `/`、家目录或 `~/Workspace` 时禁止批量拉取。
- 快捷键新增 Git（`Ctrl+4`）；Agent 与 Skills、日志中心顺延为 `Ctrl+5`、`Ctrl+6`。

### 资源与清理
- 新增 `devtray skills install`：GitHub/裸名字预检、显式 agent、固定 CLI、安装后校验；面板支持输入来源安装。
- 新增 `devtray skills restore`：默认恢复内容与副本，`--restore-lock` 才写回单条 lock；面板支持选择快照恢复。
- 技能升级/删除接入写门禁：固定 `skills@1.7.4`、全路径枚举与快照、漂移默认拒绝、执行后校验、job 与审计。
- 新增 `devtray skills check` 与面板「检查更新」：GitHub tree sha 对比、6 小时缓存、限流/失败显示无法检查。
- 新增「Agent 与 Skills」只读页与 `devtray agents` / `devtray skills`：Agent 版本与安装来源、Skills lock 对账、agent 副本漂移。
- 新增 MCP `devtray_disk` / `devtray_job` / `devtray_clean`：清理任务默认经桌面 yad 审批（`mcp_writes` 控制），立即返回 job id，可查询与取消。
- 新增 `[[project]] root`，磁盘扫描以项目根为准，缺省用 service.cwd 公共祖先推断。
- 新增「资源」页与 `devtray disk`：按项目统计 `target` / `node_modules` / `.venv` 等可重建目录的分配与逻辑空间，支持打开目录、扫描缓存与总览资产摘要。
- 新增 `devtray disk clean` 与后台清理任务：白名单 + 证据链 + git 跟踪/挂载点/路径围栏/运行状态六项检查，确认前与取锁后各校验一次；删除写入审计，失败可重跑。

## 0.1.1 - 2026-10-25

### 许可与分发
- devtray 自有源码、文档和项目自制资源正式采用 MIT 许可证。
- 补齐 Rust 依赖许可原文、Unicode 声明、Rust 标准库声明、musl 版权信息，以及独立安装的 Qt 运行时使用的 LGPLv3／GPLv3 正文。
- 按构建目标生成依赖声明，并随 tar、Debian 和 RPM 安装包分发。
- 说明 devtray 与外部 Python／Qt 及系统工具的许可边界。

## 0.1.0 - 2026-10-25

首个公开版本：让开发者和 AI Agent 共用可见的开发服务与日志。

### 功能

- 通过 Linux 系统托盘、小面板、完整桌面面板、CLI 或 MCP 管理持久化 tmux 开发服务。
- 支持服务启停／重启、日志正则或 TCP 端口就绪探测、可选失败桌面通知。
- 日志查看、搜索、级别筛选、复制与导出，支持跨重启历史和大小轮转。
- 快速打开原始 tmux 终端、项目目录和浏览器预览。
- 内置六个 stdio MCP 工具，让 Agent 与桌面程序共用服务管理逻辑。
- TOML 配置和图形化表单，保存前校验并保留注释。

### 文档与分发

- 中英文 README，说明安装、依赖、添加项目、日志、Agent 接入、构建与排障。
- 项目配置示例和 Linux ARM64 交叉编译说明。
- Linux x86_64 musl 可执行文件、Debian／RPM 安装包和 SHA-256 校验和。
- Rust、Qt、MCP、tmux、托盘集成及安装包构建的 CI 检查。

### 平台范围

- 目前支持 Linux，不包含原生 macOS／Windows 应用。
- 提供 ARM64 交叉编译步骤，目标桌面行为尚未验证。
- Python／PySide6 为面板可选依赖；CLI／MCP 服务管理需要 tmux，不需要 Qt。
