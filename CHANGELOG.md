# Changelog

## 0.1.1 - 2026-10-25

### Licensing and distribution
- License devtray source code, documentation, and project-created assets under MIT.
- Include third-party Rust license texts, Unicode notices, Rust standard-library notices, musl copyright information, and LGPLv3/GPLv3 texts for the separately installed Qt runtime.
- Generate dependency notices for the selected build target and include them in tar, Debian, and RPM packages.
- Document the license boundary between devtray and external Python/Qt and system tools.

## 0.1.0 - 2026-10-25

Initial public release: visible development services for developers and AI agents.

### Features

- Manage project services in persistent tmux sessions from a Linux system tray, compact panel, full desktop panel, CLI, or MCP client.
- Start, stop, and restart services; inspect readiness through log patterns or TCP ports; receive optional failure notifications.
- Read, search, filter, copy, and export logs, with restart history and size-based rotation.
- Open the original tmux terminal, project directories, and browser previews.
- Connect agents through six built-in stdio MCP tools sharing the desktop application's service management logic.
- Configure projects and services in TOML or validated graphical forms that preserve comments.

### Documentation and distribution

- English and Simplified Chinese READMEs covering installation, dependencies, project setup, logs, agent integration, builds, and troubleshooting.
- Example project configuration and Linux ARM64 cross-compilation instructions.
- Linux x86_64 musl executable, Debian/RPM packages, and SHA-256 checksums.
- CI for Rust, Qt, MCP, tmux, tray integration, and package builds.

### Platform scope

- Linux is the supported platform; native macOS and Windows applications are not included.
- ARM64 cross-compilation instructions are provided; target desktop behavior is not yet verified.
- Python/PySide6 is optional for the panels; CLI/MCP service management requires tmux but not Qt.
