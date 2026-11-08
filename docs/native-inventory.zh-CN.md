# 本机环境采集

实现状态：CLI、MCP 与 Qt 面板已接入。Windows / macOS 的交叉检查和对应 runner 的任务已准备；交叉检查不代表目标系统已经运行验证。服务托管、环境登记和 WSL 切换仍按 platform-adaptation.zh-CN.md 的后续阶段实施。

## 使用

- CLI：`devtray inventory --json`；省略 --json 输出人可读清单。
- MCP：`devtray_inventory`，参数为 {}。无需 config.toml；stdout 保持 MCP 协议格式。
- 面板：设置 → 本机环境 → 采集本机环境。采集结果独立于服务状态，不随每两秒的服务刷新重复执行。再次点击重新采集；失败保留表格，清楚标明失败。

目前只处理 local 环境。覆盖 Git、rustc、Cargo、Node、Bun 和 src/agents.rs 中登记的 Agent；不启动 tmux、WSL、服务或自动安装工具。不读取全量环境变量，不查询远程最新版本，不将工具的任意原始输出写入 JSON。

## 返回契约

顶层 schema_version=1，包含 environment_id、platform、architecture、collected_at（Unix 秒）、capabilities 和 tools。capabilities 表示已实现／当前可用能力，不能把工具已安装解释为服务托管已实现。

每个工具始终有一行：

| 字段 | 含义 |
| --- | --- |
| id | 稳定工具 ID；Rust 编译器使用 rust，命令为 rustc。 |
| status | found：找到并解析版本；not_found：查找位置未找到；probe_failed：找到但未能确认版本。 |
| path | 实际传给版本命令的路径；未找到为 null。保留 shim / 包装脚本路径，不用符号链接目标替换调用入口。 |
| version | 从版本输出中提取的版本号；未知为 null，不能显示成未安装。 |
| source | path 或 user_directory（补充查找目录），未找到为 null。后者不表示该工具已经加入服务的 PATH。 |
| reason | 探测失败原因；不返回任意 CLI 原始输出。 |

PATH 优先，使用宿主系统的路径分隔规则。Windows 按 PATHEXT 查找；补充现有 Agent 用户工具目录，并处理 Windows npm、WinGet 与 macOS Homebrew 常见目录。补充目录的发现仅是清单证据；后续编排应显式采用同一已确认路径，不能假定所有外部命令都已改用补充目录。

## 探测边界

最多四组并行工作，每个工具最多等待两秒；读取最多 64 KiB 输出并轮询大小，超限或超时结束直接探测进程。临时输出读取后删除；stdout 和 stderr 都可提供版本。超时控制目前针对直接子进程，不承诺终止工具自行启动的整棵后台树；完整 Windows Job Object 托管另行实现。

Windows 的 .cmd / .bat 包装脚本目前能发现路径，但以 probe_failed 和明确原因返回，暂不执行脚本。可执行程序的版本探测不受此限制。后续应增加固定参数的批处理探测、完整进程树回收与 Windows 实测，不能把此限制当成工具未安装。

清单不自动持久化；面板仅保留当前进程的上一次快照，并显示采集时间。环境隔离缓存、切换、项目和 Skills 的聚合快照尚未接入；现有 Agent 与 Skills 页面继续提供原接口。

## 验证

tests/native_inventory.py 用真正的本机 Rust 可执行文件充当工具，隔离 PATH、用户目录和配置，覆盖真实路径、stderr 版本、非零退出、无法解析、超时、输出超限，以及无配置情况下的 MCP 请求；wsl 哨兵验证不会调用 WSL。

Linux verify 中执行该测试；desktop-probe 中 Windows / macOS 在原生构建后执行同一脚本。tests/test_panel.py 另检查清单渲染、错误状态与失败时保留上次结果。远程运行前不能把 workflow 配置等同于已通过的实机证据。
