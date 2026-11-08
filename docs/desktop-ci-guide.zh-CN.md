# DevTray：从 Linux 发起 macOS / Windows 构建

> **设计已更新：** 本文保留早期 CI 探针和打包示例。Windows 默认原生运行、WSL2 可选；后端、作业依赖、产物名和验收要求以[跨平台开发设计](platform-adaptation.zh-CN.md)为准，不直接照搬下文的 WSL 专用发布流程。

给实施 agent：扩展 `.github/workflows/ci.yml`，保留现有 `verify → linux-packages`、触发事件、只读权限和 Linux 产物名。首个可用版本优先做 **macOS arm64 + tmux**。Windows 示例按 **x64 桌面 + WSL2 内的 Linux CLI** 编写；若用户要求直接管理 PowerShell 中的服务，使用文末的原生分支。两条路线不要同时实现。下文脚本名是待创建的交付物，不是仓库现有命令。

## 第一份 PR：让 CI 提供有用的移植反馈

现有 Linux job 装了 apt 包、D-Bus、tmux，且测试假设 Linux 文件布局；不要把 `verify` 直接改成三系统矩阵。在现有 `jobs:` 下追加下面的 job。第一阶段仅手动运行，有移植错误就保留失败及日志；不要用 `continue-on-error`。macOS CLI 通过之后，将它单独拆成 push / PR 必跑的 job；Windows 检查同理。这样其中一个平台仍在移植时不会挡住另一个平台。

```yaml
  desktop-probe:
    if: github.event_name == 'workflow_dispatch'
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: macos-15
            label: macos-arm64-cli
          - os: windows-2025
            label: windows-x64-panel
    name: probe / ${{ matrix.label }}
    runs-on: ${{ matrix.os }}
    timeout-minutes: 25
    steps:
      - uses: actions/checkout@v4
      - if: runner.os == 'macOS'
        run: |
          rustup toolchain install 1.97.1 --profile minimal
          rustc +1.97.1 -vV
          cargo +1.97.1 check --locked --all-targets --target aarch64-apple-darwin
      - if: runner.os == 'Windows'
        uses: actions/setup-python@v5
        with:
          python-version: '3.12'
          architecture: x64
      - if: runner.os == 'Windows'
        shell: pwsh
        env:
          QT_QPA_PLATFORM: offscreen
        run: |
          python -m pip install -r packaging/desktop/requirements-win.lock
          if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
          python tests/desktop_panel_smoke.py
```

此片段在原工作流已有的 `workflow_dispatch:` 下即可使用。与它同一份 PR 必须交付 Windows 锁文件和 smoke 脚本：从现有 UI 创建 QApplication，实际走面板初始化、单实例键生成和第二次启动的 IPC，设置超时并清理进程；允许用返回固定 JSON 的测试 CLI，不把这一步标为通过 WSL 集成。macOS 探针应暴露真实编译错误，后续用单独的适配提交修复。上例沿用仓库的 Rust／checkout 版本；Python、PySide6、PyInstaller 及传递依赖在两目标架构上验证并固定，不使用未锁定的最新依赖做发布包。[runner 标签与架构](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)

## 第二份 PR：让应用在目标系统上工作

| 位置 | 实施内容 | 完成证据 |
| --- | --- | --- |
| `Cargo.toml`、`src/main.rs`、`src/tray.rs`、`src/icons.rs` | 将 ksni 依赖及其类型、托盘循环限定为 Linux；Mac CLI 与 MCP 不初始化 D-Bus。检查原托盘承担的状态刷新和告警，在新桌面入口接回首版承诺的功能。 | Mac 上实际 build 并运行 CLI；Linux 原校验全绿。 |
| `src/tmux.rs`、`src/terminal.rs`、`src/disk.rs`、`src/clean.rs` | Mac 保留 tmux，适配可执行文件查找、终端、用户目录、实例检测。磁盘清理依赖 Linux 的挂载边界；未移植前在后端拒绝，UI 和 MCP 明示能力不可用。 | 在隔离 tmux socket、临时配置和状态目录下启停服务并检查日志；不要照搬 `tests/log_restart.py` 的 `/usr/bin/tmux`。 |
| `src/panel.rs`、`ui/panel.py` | 新增可直接打包的 Python 桌面入口，用 Qt 托盘打开现有面板。修正 Windows 没有 `os.getuid()`；从安装目录定位后端和资源，而非依赖工作目录。Linux 原来从 Rust 注入 Python 的启动方式继续可用。 | 从不同工作目录启动；关闭再打开面板后仍能看到同一服务；用户配置路径含空格和中文也能使用。 |
| Windows WSL 适配 | GUI 选择发行版，经 `wsl.exe -d <发行版> -- <Linux CLI 绝对路径> …` 调用，使用参数数组；明确配置和项目路径属于哪一侧。不要假定 Windows 的 `DEVTRAY_CONFIG` 会自动进入 WSL。首版要求发行版已装 tmux。 | WSL 缺失或发行版不匹配时显示可操作的提示；用真实 WSL2 验证启动、状态、日志、重启、停止。 |

Mac 的运行测试先定位或安装 tmux，再用与仓库相同工具链运行 `cargo test --locked`、`cargo build --release --locked --target aarch64-apple-darwin`。对真正 Linux 专属的测试按平台限定，不要整套跳过。两端至少用安装布局中的 CLI 验证 `--version`、`status --json`、MCP stdio 的 `initialize`；将运行日志与 JSON stdout 分开。Windows WSL 路线在选定发行版里做这些 CLI 检查，Windows 侧再测试通过适配器调用。若托管 runner 无法运行 WSL2，把真实 WSL 测试放到受控的 Windows 测试机或明确记录人工结果；不要把模拟测试写成真机结果。

## 第三份 PR：生成并验收预览安装包

继续在同一个 `jobs:` 下加两个 job，各自 checkout 并准备工具链。普通 push / PR 跑平台校验；初期只用手动触发打预览包，避免每次改代码都打包。推荐依赖如下：

| Job | Runner、条件与依赖 | 需要实现的脚本及输出 |
| --- | --- | --- |
| `macos-package` | `macos-15`；`if: github.event_name == 'workflow_dispatch'`；`needs: [verify, macos-check]` | `scripts/package-macos.sh`：构建 arm64 CLI，在原生 Python 环境运行 PyInstaller spec，生成含 CLI、Qt 平台插件和图标的 DevTray.app，再用 hdiutil 生成 `dist/macos/DevTray-<版本>-macOS-arm64.dmg`。 |
| `windows-package` | `windows-2025`；同样仅手动；`needs: [verify, windows-check, linux-packages]` | `scripts/package-windows.ps1`：下载**本次运行**的 Linux x86_64 artifact，核对 SHA256SUMS，从 tar.gz 取 CLI，作为 WSL 载荷。用原生 Windows Python + PyInstaller onedir 构建桌面目录，再用固定版本的 Inno Setup 编译 `packaging/windows/devtray.iss`，输出 `dist/windows/DevTray-<版本>-Windows-x64-WSL2-Setup.exe`。 |

`macos-check` 和 `windows-check` 是第二阶段拆出的 job ID，必须先存在；不要把占位 ID 直接提交。各 job 的文件系统独立；Mac 在打包 job 重新构建，Windows 使用已有 artifact，不从 latest release 或 runner 的偶然残留取 CLI。Windows 消费 Linux artifact 的步骤：

```yaml
      - uses: actions/download-artifact@v4
        with:
          name: devtray-linux-x86_64
          path: staging/linux
```

版本从 Cargo 元数据读取；包内 CLI 版本必须一致。每个平台只把安装包和它的 `SHA256SUMS` 放入自己的产物目录。先挂载 DMG 并复制应用、或在临时目录静默安装 EXE，从安装位置运行检查，再上传。例如 Mac job 的最后一步（Windows 改名和目录即可）：

```yaml
      - uses: actions/upload-artifact@v4
        with:
          name: devtray-macos-arm64
          path: dist/macos/
          if-no-files-found: error
          retention-days: 14
```

验收以**下载到用户手里的包**为准：

- Mac：从 Finder 打开复制出的应用，能找到 Homebrew 安装的 tmux 和配置中的开发命令；启动一个输出已知文本、监听临时端口的服务，关闭面板后服务仍在；重新打开能读到日志，停止后端口释放。用没有开发环境 Python 的机器做首次启动检查。
- Windows：在无系统 Python 的环境安装，开始菜单能打开托盘，首次启动把匹配的 Linux CLI 部署到**所选发行版的用户目录**。验证相同的服务生命周期；卸载不破坏用户项目。托盘点击、菜单与真实 WSL 结果应和无界面的 CI 检查分开记录。
- 两包都核对版本、架构、校验和与实际捆绑组件。既有 `tests/package_licenses.py` 只检查 Linux 三种格式；按 `docs/license-audit.md` 对新增 Python、Qt、PySide6、Shiboken 和插件补齐对应声明与分发材料，不要把 Linux 清单直接当成新包的完整清单。

如果 Windows 要管理**本机 PowerShell 服务**：保留 Windows GUI 和安装包步骤，将 WSL 适配改为原生进程后端，`windows-check` 增加 MSVC 目标编译、进程树停止、关闭面板后重新接管的测试；`windows-package` 去掉对 `linux-packages` 的依赖，改为构建并携带 `x86_64-pc-windows-msvc` CLI，产物名去掉 WSL2。

## 从 Linux 发起、下载和交付

工作流文件需已存在于默认分支，且被选分支包含新 job 和脚本。开发者有仓库 Actions 运行权限时，可在网页 **Actions → CI → Run workflow** 选分支；或在本机已登录的 GitHub CLI 执行：

```bash
gh workflow run ci.yml --ref <实现分支>
gh run list --workflow ci.yml --branch <实现分支> --event workflow_dispatch --limit 3
gh run watch <run-id> --exit-status
gh run download <run-id> -n devtray-macos-arm64 -D dist/download/macos
gh run download <run-id> -n devtray-windows-x64-wsl2 -D dist/download/windows
```

上面 Windows 名称也是上传 job 应使用的 artifact 名。用 run ID 和提交 SHA 对应构建结果。[手动触发条件](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#workflow_dispatch)；[artifact 行为](https://github.com/actions/upload-artifact)。

本阶段只上传 **Actions 预览产物**，不创建 GitHub Release。公开发布另做受保护的发布流程：在可信 tag 上校验版本，对 macOS 完成 Developer ID 签名、公证及装订，对 Windows 按发布策略签名，重新计算最终文件校验和；PR 校验不读取发布凭据。M 系列测试包也要保留所需的 ad-hoc 签名，不将测试包称作已公证版。打包后的使用体验必须在对应桌面系统确认。[PyInstaller 支持的平台](https://pyinstaller.org/en/stable/index.html)；[Apple Developer ID](https://developer.apple.com/developer-id/)。

**agent 交付格式：** 列出改动文件、两个平台的后端选择、Actions run 链接／提交 SHA、安装包名与 SHA-256、通过的安装后检查、仍需桌面人工验收的项目。没有远程运行证据时，标明“未在该平台验证”。
