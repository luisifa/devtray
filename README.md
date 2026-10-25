# devtray

**Start development projects quickly. Let your agent run them. Keep the logs in your hands.**

[简体中文](README.zh-CN.md) · [Configuration example](examples/config.toml) · [Changelog](CHANGELOG.md)

A Linux tray app for managing development services such as `cargo run`, `bun run dev`, and `npm run dev`. Group services by project, start them from a desktop panel or an AI agent, and inspect the same processes and logs from either side.

Agents often leave development servers in background shells that are awkward to find or inspect. devtray puts those services in named tmux sessions and gives developers a visible place to check status, read errors, copy logs, and restart a service themselves. AI-assisted debugging and hands-on debugging can use the same workspace.

![Project overview — demo data](docs/screenshots/overview.png)

## What you get

- A compact quick panel, a full desktop panel, and a native tray context menu.
- Projects containing multiple services, with individual and project-wide start/stop/restart controls.
- Readiness checks using log patterns or a TCP port; failure indicators and optional desktop notifications.
- Searchable, filterable logs with restart history, rotation, selection/copy, and export.
- One-click access to the original tmux terminal, project directory, and browser preview.
- A CLI and built-in stdio MCP server for agents, sharing the same service management code as the desktop UI.

Services and log collectors live in tmux. Hiding the panel, quitting the tray, or restarting devtray does not stop them. A reboot or logout that terminates the user's processes still can.

**v0.1.1 targets Linux.** The interface and CLI messages are currently primarily Chinese; these instructions are available in English and Chinese. There is no native macOS or Windows app in this release.

## Dependencies

| Component | Required for |
| --- | --- |
| `tmux` and a POSIX shell (`sh`) | Starting and managing services, including through CLI/MCP |
| D-Bus session and a StatusNotifierItem/AppIndicator-compatible tray host | The tray; use a desktop/panel with SNI support |
| Python 3 + `PySide6` (`QtWidgets`, `QtNetwork`) | Compact and full desktop panels; Python 3.10+ is recommended |
| A terminal emulator | Opening an interactive tmux terminal |
| `xdg-open` (`xdg-utils`) | Opening preview URLs and project directories |
| `notify-send` | Optional failure notifications |
| `yad` | Optional tray/CLI service-editing forms; the Qt panel has its own forms |
| Rust stable + Cargo + a C linker | Building from source; v0.1.1 was tested with Rust 1.97.1 |
| Your project's runtimes and dependencies | Whatever the configured command needs: Rust, Node.js, Bun, Python, etc. |

CLI and MCP service management do not require the tray, Python, or Qt. devtray does not install dependencies for your projects. Rust dependencies are pinned in [Cargo.lock](Cargo.lock); no Node.js frontend or frontend build server is needed. Python UI source and generated tray icons are embedded in the Rust binary.

Example system dependencies:

```bash
# Debian / Ubuntu (install Rust separately using rustup)
sudo apt update
sudo apt install build-essential tmux dbus xdg-utils libnotify-bin python3 python3-venv

# Arch Linux
sudo pacman -S --needed base-devel rust tmux dbus xdg-utils libnotify python pyside6
```

If your distribution does not provide PySide6, use a virtual environment:

```bash
python3 -m venv "$HOME/.local/share/devtray/venv"
"$HOME/.local/share/devtray/venv/bin/pip" install PySide6
export DEVTRAY_PYTHON="$HOME/.local/share/devtray/venv/bin/python"
```

Persist `DEVTRAY_PYTHON` in your shell configuration for CLI launches, and in the systemd override below for the tray service. Qt also needs a working Wayland or X11 environment and the matching system libraries; virtualenvs do not replace those desktop dependencies.

## Install and start

### From source

```bash
git clone https://github.com/luisifa/devtray.git
cd devtray
./scripts/install-local.sh
export PATH="$HOME/.cargo/bin:$PATH"

# Start the tray now and on future logins
systemctl --user enable --now devtray.service

devtray panel --mode full
devtray check
```

The installer builds with `Cargo.lock` and installs the executable, icons, desktop launcher, and a user systemd unit. It expects Cargo's default installation location, `~/.cargo/bin`. First launching the tray or panel creates `~/.config/devtray/config.toml` with commented examples. An empty project list is expected until you add your services.

Without systemd, run `devtray` from a graphical session and add it to your desktop's startup applications. For **CLI/MCP only**, use `cargo install --locked --path .`; this installs just the executable, without the desktop launcher or systemd unit. Create an empty configuration before first use:

```bash
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/devtray"
# Create only if absent; preserve existing configuration.
touch "${XDG_CONFIG_HOME:-$HOME/.config}/devtray/config.toml"
devtray status --json
```

### From release packages

Download a matching artifact from [GitHub Releases](https://github.com/luisifa/devtray/releases). Verify it against `SHA256SUMS` from the same release.

```bash
# Debian / Ubuntu: apt also resolves package dependencies
sudo apt install ./devtray_0.1.1-1_amd64.deb

# Fedora / RPM-based systems
sudo dnf install ./devtray-0.1.1-1.x86_64.rpm

# After installing either package
mkdir -p "$HOME/.local/state/devtray"
systemctl --user daemon-reload
systemctl --user enable --now devtray.service
```

Alternatively, unpack the Linux tarball and install its `devtray` executable into a directory on `PATH`. Tarballs include documentation and desktop assets but do not install them automatically. PySide6 is optional and must be installed separately for all formats. If you previously installed from source, its user unit takes precedence over the packaged unit; use `systemctl --user cat devtray` to check which executable it starts.

## Add a project

**In the full panel:** open service management and choose “添加服务” (Add service). Enter a project name, service name, working directory, and command. A new project name creates a project; the same name adds another service to it. The form validates the configuration before saving and preserves TOML comments.

**In TOML:** edit `~/.config/devtray/config.toml`. Use [examples/config.toml](examples/config.toml) as a starting point, replacing the paths with existing directories on your machine:

```toml
[settings]
poll_ms = 2000
notify = true
notify_debounce_s = 60
# terminal = "kitty"

[[project]]
name = "my-app"

[[project.service]]
name = "backend"
cwd = "~/Workspace/my-app/backend"
cmd = "cargo run"
autostart = false
ready = "Listening on"
port = 3000
url = "http://localhost:3000/"
[project.service.env]
RUST_LOG = "debug"

[[project.service]]
name = "frontend"
cwd = "~/Workspace/my-app/frontend"
cmd = "bun run dev"
ready = "ready in"
url = "http://localhost:5173/"
```

Add another `[[project]]` block for another project, or more `[[project.service]]` blocks beneath the current project. Use ASCII project/service identifiers to avoid collisions in sanitized tmux session names.

| Field | Meaning |
| --- | --- |
| `name` | Required project/service identifier; targets use `project/service` |
| `cwd` | Required existing working directory; absolute paths and `~/...` are recommended |
| `cmd` | Required command, executed using `sh -lc` inside tmux |
| `autostart` | Default `false`; start when the tray starts if the service is not already running |
| `ready` | Optional regular expression matched against that service's tmux output |
| `port` | Optional local TCP readiness check; either `ready` **or** `port` passing means ready |
| `url` | Optional preview URL; otherwise inferred from logs, then from `port` |
| `env` | Optional environment variable table for this service |
| `log_file` | Optional separate log file; relative paths resolve against `cwd`; cannot be shared between services |

The tray reloads configuration automatically and retains the last valid configuration if a new edit is invalid. CLI/MCP calls read the file each time and report invalid configuration. Changing a running service's command/environment requires restarting that service; editing the file does not restart it automatically. `ready` is often more precise than `port`, because an unrelated process may already own that port.

## Everyday use

Left-click the tray icon to show/hide the last-used panel; the first launch uses the compact panel. Right-click for the native menu. Switch between compact/full modes inside the panels. Closing a panel or pressing Esc hides it; services keep running.

![Compact panel — demo data](docs/screenshots/quick-menu.png)

```bash
devtray                          # Run the tray
devtray panel --mode full         # Open and remember full mode
devtray panel --mode small        # Open and remember compact mode
devtray panel --toggle            # Show/hide
devtray panel --target my-app/backend  # Jump to service logs
devtray panel --quit              # Exit the UI process; keep services running

devtray status
devtray status --json
devtray start my-app/backend
devtray restart my-app/frontend
devtray stop my-app/backend
devtray terminal my-app/frontend  # Attach in your terminal emulator
devtray open my-app/frontend      # Open browser preview
devtray edit my-app/backend       # yad form, or editor fallback
devtray help
```

Targets accept `project/service`, a unique service name, or the tmux session name (`devtray-my-app-backend`). **`start` recreates an existing session, including a running service.** Check status before calling it; use `restart` when you intend to restart. Stop sends Ctrl-C, waits up to three seconds, then removes the session. In an attached tmux terminal, `Ctrl-b`, then `d` detaches without stopping the service.

| Machine state | Meaning |
| --- | --- |
| `not_started` | No matching tmux session; also shown after stop removes the session |
| `running` | Process alive, no readiness check configured |
| `starting` | Process alive, waiting for a readiness check |
| `ready` | Readiness check passed |
| `exited_ok` | Process exited with code 0; pane retained |
| `exited_failed` | Process exited with a nonzero code; pane retained |
| `exited_signal` | Pane records a signal exit |

The tray shows green for running/ready, yellow for starting, red for failure/configuration errors, and gray when inactive. Readiness is latched for a pane's lifetime. There is no continuous health check or crash auto-restart in v0.1.1.

## Logs you can inspect yourself

![Log center — demo data](docs/screenshots/logs.png)

```bash
devtray logs my-app/backend -n 200
devtray logs my-app/backend --level warn,error
devtray logs my-app/backend --level info --search request
devtray logs my-app/backend --history --level error
devtray logs my-app/backend --run 1 --json
```

Logs default to `${XDG_STATE_HOME:-~/.local/state}/devtray/logs/`. Each service keeps the current `.log` plus up to ten archives (`.log.1` is newest). Restart and approximately 10 MiB size rotation both create archives. Retention is about 110 MiB per service, with small write-block overshoot; pre-existing oversized files may exceed this. Archive numbers move as files rotate; they are not permanent run IDs.

Levels are `trace`, `debug`, `info`, `warn`, `error`, and `other`. Search is case-insensitive. Queries filter first and return the latest matching lines; they scan at most the last 2 MiB of each selected file. CLI results are capped at 10,000 lines, MCP at 5,000, and the panel at 2,000. Use the raw files for older content. JSON queries include truncation information.

In the panel, selecting text or scrolling up pauses refresh; resume explicitly when ready. Export saves the currently displayed filtered result. Logs remain readable after a service stops or crashes. The original tmux terminal stays available separately; devtray does not modify your global tmux keybindings.

## Connect an agent through MCP

The agent launches **`devtray mcp`** as a local stdio subprocess. No HTTP server, listening port, or separate daemon is needed. The tray does not have to be running. Use the same Unix user and configuration as the desktop app to see the same services. The agent needs access to tmux's local socket, project directories, and log files.

For Codex (syntax also available in `codex mcp add --help`):

```bash
codex mcp add devtray -- "$HOME/.cargo/bin/devtray" mcp
codex mcp list
```

For other stdio MCP clients, adapt this configuration to the client's settings format:

```json
{
  "mcpServers": {
    "devtray": {
      "command": "/home/YOUR_USER/.cargo/bin/devtray",
      "args": ["mcp"],
      "env": {
        "DEVTRAY_CONFIG": "/home/YOUR_USER/.config/devtray/config.toml"
      }
    }
  }
}
```

Use `/usr/bin/devtray` for a distribution package, or the actual absolute path reported by `command -v devtray`. Replace `YOUR_USER`; JSON values do not expand `~` or `$HOME`. Reload/reconnect your MCP client after registration. See the [official Codex MCP documentation](https://developers.openai.com/codex/mcp/) for client configuration.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `devtray_status` | None | List services, machine states, commands, logs, and preview URLs |
| `devtray_logs` | `target`; optional `lines`, `level`, `search`, `history` | Read current or archived logs; default 80 lines |
| `devtray_start` | `target` | Start/recreate a service, then return its new status |
| `devtray_stop` | `target` | Stop a service |
| `devtray_restart` | `target` | Stop then start a service |
| `devtray_open_preview` | `target` | Open the preview on the user's desktop |

Suggested agent workflow: **status → start if needed → wait and recheck readiness → read logs on failure**. A successful start call means the process was launched, not that compilation or startup has finished. The MCP status response can include `advice` for starting/failed services. The CLI's JSON status response contains `services`; its human-readable output includes advice when needed.

Example request to your agent:

> Use devtray to start my-app's backend and frontend. Check their status first and leave already-running services alone. Wait for readiness, and inspect error logs if startup fails. I will review the same logs in the desktop panel.

For agents without MCP, use `devtray status --json` and the CLI commands above. You can add the following to your project's `AGENTS.md`:

```text
Manage long-running development services through devtray (MCP or CLI).
Read status before starting; devtray start recreates existing sessions.
Keep servers in devtray so developers can inspect their terminals and logs.
While a service is starting, wait and check again before declaring failure.
Use devtray logs for diagnosis. Stop/restart only when the task calls for it.
Add service definitions in the devtray config or UI; MCP has no add-service tool.
```

## Build, package, and platform support

| Platform | v0.1.1 support |
| --- | --- |
| Linux x86_64 | Primary tested platform; GNU and musl builds |
| Linux ARM64 | Source/cross-compilation instructions below; needs validation on the target desktop |
| Windows | No native build; run the Linux CLI/MCP inside WSL2. GUI/tray support needs a suitable Linux desktop environment and is not validated |
| macOS | No supported native build; use a Linux VM for this release |

The current implementation uses SNI/D-Bus, tmux, `/proc`, `xdg-open`, Unix sockets, and Linux desktop integration. Adding a Rust target alone does not provide native Windows/macOS support. WSL/VM sessions manage services inside that environment, not host-native processes.

### Native Linux build

Install a current stable Rust toolchain using [rustup](https://rustup.rs/), plus your distribution's C build tools:

```bash
cargo build --release --locked
./target/release/devtray --version

# Packaging helpers (only needed to make packages)
cargo install cargo-deb --locked
cargo install cargo-generate-rpm --locked
rustup component add rust-docs
python3 scripts/generate-third-party-notices.py --target "$(rustc -vV | sed -n 's/^host: //p')"
cargo deb --no-build --locked                 # target/debian/
cargo generate-rpm                          # target/generate-rpm/
```

Build GNU/Linux binaries on the oldest distribution you intend to support; newer glibc builds may not run on older systems. For the distributable static Linux executable used by this release:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --locked --target x86_64-unknown-linux-musl
cargo deb --no-build --locked --target x86_64-unknown-linux-musl
cargo generate-rpm --target x86_64-unknown-linux-musl

# Build all release artifacts and SHA256SUMS in dist/
./scripts/package-release.sh
```

The release packaging notices are reviewed for Rust 1.97.1; use that toolchain to reproduce the packages. Updating the packaging toolchain requires reviewing the musl/LLVM notice snapshots.

A static Rust executable still needs tmux at runtime and Python/Qt for the panel. Generated icons are committed; rebuilding them additionally requires `rsvg-convert` and PySide6 via `scripts/build-icons.sh`.

### Cross-compile Linux ARM64 from Linux x86_64

Example on Debian/Ubuntu:

```bash
sudo apt install gcc-aarch64-linux-gnu binutils-aarch64-linux-gnu
rustup target add aarch64-unknown-linux-gnu
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
  cargo build --release --locked --target aarch64-unknown-linux-gnu
```

The executable is `target/aarch64-unknown-linux-gnu/release/devtray`. Copy it to an ARM64 Linux machine with the runtime dependencies, then verify CLI, desktop tray, and Qt behavior there. Install PySide6 for the **target** machine, not the build host. This build is not a macOS/Windows port. Rust's [cross-compilation guide](https://rust-lang.github.io/rustup/cross-compilation.html) explains targets and linkers.

### Verification

```bash
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked -- -D warnings
cargo build --locked
python3 tests/mcp.py
QT_QPA_PLATFORM=offscreen python3 tests/test_panel.py
python3 tests/panel_instance.py
python3 tests/log_restart.py
python3 tests/launch_shortcuts.py
dbus-run-session -- python3 tests/tray_context_menu.py
```

The integration tests use temporary configurations and isolated tmux/D-Bus sessions. They require local socket/PTY access; the tray test also needs Python's PyGObject (`gi`, e.g. `python3-gi`). Run them on Linux outside sandboxes that prohibit these facilities. CI checks Rust, Qt, MCP, and integration behavior. Architecture notes are in [docs/control-panel.md](docs/control-panel.md) (Chinese).

## Troubleshooting and paths

```bash
devtray check
systemctl --user status devtray.service
systemctl --user cat devtray.service
tail -n 100 "$HOME/.local/state/devtray/service.log"
```

The provided units redirect devtray's own stdout/stderr to `service.log`; `journalctl --user -u devtray.service` is useful for systemd lifecycle events. Managed project logs are in the separate `logs/` directory.

- **Command not found / exit 127:** the tray's systemd environment can differ from your shell. Use an absolute executable path or add runtime directories through `systemctl --user edit devtray.service`.
- **PySide6 unavailable:** point `DEVTRAY_PYTHON` to the virtualenv interpreter in both your shell and service environment.
- **No tray icon:** check that the graphical session has D-Bus and an SNI/AppIndicator host. A Qt panel or CLI can still be used independently.
- **Newly installed version not visible:** run `systemctl --user restart devtray.service`; close the old panel process with `devtray panel --quit` and reopen it. Existing services keep their current command and log collector until restarted.
- **Services die when restarting the tray:** preserve `KillMode=process` in the supplied unit. It prevents systemd from terminating tmux's processes along with the tray.
- **Headless CLI/MCP:** initialize the config explicitly; `check` does not create it. Desktop-only features need a graphical session.

Example systemd override (replace the Python path):

```ini
[Service]
Environment=DEVTRAY_PYTHON=/home/YOUR_USER/.local/share/devtray/venv/bin/python
Environment=PATH=%h/.bun/bin:%h/.cargo/bin:%h/.local/bin:/usr/local/bin:/usr/bin:/bin
```

Then run `systemctl --user daemon-reload` and `systemctl --user restart devtray.service`. Keep `Restart=on-failure` so choosing Quit does not immediately relaunch the tray.

| Variable | Purpose |
| --- | --- |
| `DEVTRAY_CONFIG` | Override the complete configuration file path |
| `XDG_CONFIG_HOME` | Base config directory; default `~/.config` |
| `XDG_STATE_HOME` | Base log/state directory; default `~/.local/state` |
| `DEVTRAY_PYTHON` | Interpreter for the embedded Qt panels; default `python3` |
| `TERMINAL` | Preferred terminal when `settings.terminal` is unset |

v0.1.1 does not provide dependency ordering, crash auto-restart, CPU/memory monitoring, remote multi-user control, or management of existing systemd services. It is a local development companion.

## License

devtray source code, documentation, and project-created assets are licensed under the [MIT License](LICENSE). Copyright (c) 2026 ferychen.

Third-party components retain their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and [licenses/](licenses/README.md). The optional PySide6/Qt interface uses LGPLv3 libraries installed separately by the user. This release does not bundle Python, PySide6, or Qt.

Release packages include the project license, dependency notices, applicable Unicode notices, Rust standard-library notices, and musl copyright information. Regenerate the notices after building the target with `python3 scripts/generate-third-party-notices.py --target x86_64-unknown-linux-musl`; the packaging script does this automatically.
