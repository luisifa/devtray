# devtray 仓库管理速记

当前功能与验证结果见 [开发状态（2026-10-29）](development-status.md)。

记录日期：2026-10-25。供本地开发、维护和发布时查阅。

## 仓库概况

- GitHub：[luisifa/devtray](https://github.com/luisifa/devtray)，公开仓库。
- 远程：`origin`，SSH 地址为 `git@github.com:luisifa/devtray.git`。
- 默认分支：`main`；当前发布版本：**v0.1.1**。
- 自有代码、文档和自制资源采用 [MIT](../LICENSE)，第三方组件保留原有许可。
- 主要支持 Linux；Rust 核心负责托盘、CLI、MCP 和 tmux 管理，Python／PySide6 提供可选桌面面板。
- 目标：让开发服务便于启动、便于 Agent 调用，也让开发者随时能查看终端和日志。

## 主要文件在哪里

| 路径 | 用途 |
| --- | --- |
| `src/` | Rust 核心；`cli.rs` 是操作底座，`mcp.rs` 提供 Agent 工具，`tmux.rs` 托管进程，`logs.rs` 管理日志 |
| `ui/` | 小面板和完整 Qt 面板，构建时内嵌进 Rust 二进制 |
| `assets/` | 图标、桌面启动器和预生成图标资源 |
| `scripts/`、`packaging/` | 本地安装、构建发布包、生成许可声明及 systemd 单元 |
| `tests/`、`.github/workflows/ci.yml` | 本地测试及 GitHub Actions 检查 |
| `Cargo.toml`、`Cargo.lock` | 版本、依赖、打包元数据及锁定的依赖版本 |
| `README*.md`、`CHANGELOG*.md` | 中英文使用说明和更新日志 |
| `THIRD_PARTY_NOTICES.md`、`licenses/` | 第三方许可说明和原文，需随安装包一起分发 |

## 日常修改

在 `codex/<主题>` 分支修改并提交，建议通过 PR 合并到 `main`。开始前先查看工作区，避免混入未完成的改动：

```bash
git status
git fetch origin
git switch -c codex/your-change origin/main
```

修改命令、配置字段或 MCP 工具时，同步更新中英文 README。修改功能时记录到中英文更新日志。

常用验证：

```bash
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked -- -D warnings
cargo build --locked
python3 tests/mcp.py
```

涉及面板、tmux 或托盘行为时，再运行 [README 中的集成测试](../README.zh-CN.md#验证)。这些测试使用临时配置和隔离会话。

## 构建与发布

发布包使用 **Rust 1.97.1** 和 `x86_64-unknown-linux-musl` 目标。另需 Python 3、`rust-docs`、`cargo-deb`、`cargo-generate-rpm`；安装包检查需要 `bsdtar`。环境准备见 [构建说明](../README.zh-CN.md#构建打包与跨平台说明)。

```bash
./scripts/package-release.sh
python3 tests/package_licenses.py
```

输出在 `dist/`：Linux tar.gz、deb、rpm，以及 `SHA256SUMS`。脚本自动生成第三方声明；检查程序会解包核对许可文件、版本号、MCP 和校验和。

发布顺序：

1. 修改 `Cargo.toml` 版本，运行 Cargo 命令同步 `Cargo.lock`，更新双语日志和 README 中的下载版本。
2. 完成本地检查和打包，确认许可证、第三方声明随包提供。
3. 提交修改并合并到 `main`，为该提交创建 `v版本号` 的附注标签。
4. 推送代码和标签，用 `gh release create` 发布对应版本、发布说明及该版本的四个附件。
5. 检查 GitHub Actions 结果和 Release 附件。CI 会测试并打包，但**不会自动创建 GitHub Release**。

不要重新指向已发布的标签；后续修订发布新版本。`dist/` 可能保留多个版本，上传时应明确选择当前版本文件。

Git 推送使用 SSH；创建仓库或管理 Release 使用已登录的 `gh`。可用 `gh auth status` 检查登录状态。

## 许可证维护

- 保留 [LICENSE](../LICENSE)、[第三方声明](../THIRD_PARTY_NOTICES.md) 及其许可原文。
- 升级依赖后重新生成声明；遇到未审查的许可表达式，生成脚本会停止并提示检查。
- 升级发布工具链时，重新核对 musl／LLVM 许可快照。当前脚本要求已审查的 Rust 1.97.1。
- PySide6／Qt 由用户独立安装，采用适用的 LGPL 条款；若未来改成捆绑 Qt 的安装包，需要重新检查分发义务。

v0.1.0 保留历史源码和二进制，并补充了第三方许可附件；正式 MIT 发布从 v0.1.1 开始。检查依据见 [许可审查记录](license-audit.md)。

## 本地数据与安装

`target/`、`dist/`、`.venv/`、`.env*` 和 Python 缓存已由 `.gitignore` 排除。用户的真实配置、日志、密钥和 token 不应加入仓库。

应用配置默认在 `~/.config/devtray/config.toml`，服务日志在 `~/.local/state/devtray/logs/`；它们与源码分开管理。修改本仓库不会自动更新已安装的程序。

需要更新本机安装时：

```bash
./scripts/install-local.sh
systemctl --user restart devtray.service
devtray panel --quit
devtray panel
```

托盘和面板重启后，已有 tmux 服务继续运行；服务本身的命令或环境变更，需要另行重启对应服务。
