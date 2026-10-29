# Changelog

## Unreleased

### Interaction fixes
- Make acknowledgement visible beside failed-service states and in project headings. Add a Log Center exception list with project/read-state filters, per-row log navigation and acknowledgement; preserve paused log selections and refresh after in-flight status reads.
- Fix acknowledgement of retained failed services: add per-service actions and `devtray ack`, persist acknowledgements for the failed process, and reject stale UI confirmations after a new failed run. Keep exit codes and logs while clearing unread tray attention.

- Implement the approved pine-green Qt workspace while keeping the global project/service tree, search, and collapse state; retain the light prototype as an alternative.
- Add three-column Resources with cross-project choices, combined preflight/confirmation, sequential cleanup, and stop-on-failure behavior.
- Invalidate stale choices after scans, keep tracking workers after query failures, and serialize cleanup/Skills write planning.
- Split embedded theme/resource modules, deliver the UI through stdin, and add workspace regression coverage to CI.

- Add notice dismissal and tray alert acknowledgement without changing service states; new failures alert again.
- Resolve project-relative selections before requesting a cleanup plan.
- Discover agents in user install directories, read stderr versions, bound probes, and detect Kimi/Hermes.
- Add Agent/Skills context and action menus; move copy details out of the main table and clarify update scope.
- Remove overview bulk start/stop buttons; keep bulk actions in service management.


### Resources and cleanup
- Add `devtray skills install`: GitHub/bare-name precheck, explicit agents, pinned CLI, and post-install verification; the panel can install from a source.
- Add `devtray skills restore`: restores files and copies by default; `--restore-lock` writes back only that skill's lock entry. The panel can pick a snapshot to restore.
- Skill upgrade/delete now use the write gate: pinned `skills@1.7.4`, full path enumeration and snapshots, drift refusal by default, post-run verification, jobs, and audit records.
- Add `devtray skills check` and a panel "check updates" action: GitHub tree sha comparison, six-hour cache, and explicit "cannot check" on rate limits/failures.
- Add a read-only Agent & Skills page and `devtray agents` / `devtray skills`: agent versions and provenance, skills lock reconciliation, and agent-copy drift.
- Add MCP `devtray_disk`, `devtray_job`, and `devtray_clean`: cleanup jobs require desktop yad approval by default (`mcp_writes`), return a job id immediately, and support polling/cancellation.
- Add `[[project]] root`; disk scanning uses the project root, inferring the common ancestor of `service.cwd` when absent.
- Add a Resources page and `devtray disk`: report allocated and logical size for `target`, `node_modules`, `.venv`, and other rebuildable directories, with open-directory, scan cache, and an overview asset summary.
- Add `devtray disk clean` and background cleanup jobs: allowlist plus evidence, git tracking, mount points, path fencing, and running-service checks, re-validated before and after taking the cross-process lock; deletions are audited and rerunnable.

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
