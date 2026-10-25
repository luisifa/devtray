# devtray v0.1.0 MIT 兼容性检查

检查日期：2026-10-25。检查对象：提交 `5ba0e06337fc7b324a6324cf3fb37bc40ede0b61`，以及本机 `dist/` 中对应的 v0.1.0 tar.gz、deb、rpm。

Cargo.lock SHA-256：`ce55fcb801aaa0b5fe5db90df30af3805eb6cf8a7d5cbace8898056e443fe3d7`。

## 结论与边界

从已解析的依赖许可、源码调用方式和发布包内容看，没有发现阻止 devtray 自有代码采用 MIT 的强制许可冲突。前提是项目权利人有权对这些源码和资源授予 MIT；扫描源码、文件头和 Git 历史不能证明完整的版权来源。

MIT 可以作为 devtray 自有代码的项目许可证。第三方依赖继续适用各自的许可证，不能将整个软件栈笼统声明为全部 MIT。

这是基于现有版本的工程检查，涵盖包元数据、关键许可正文、导入模块与分发方式；不是对每一行来源和所有运行时传递组件的法律鉴定。本文记录 v0.1.0 发布时的检查结果。后续 v0.1.1 已按项目权利人的授权切换 MIT，并通过生成的第三方声明和安装包许可文件处理下列分发问题。

## Rust 依赖

通过 `cargo metadata --locked --offline --format-version 1` 检查 Cargo.lock 的完整解析图：101 个 package，其中本项目 1 个、第三方 package 100 个。按名称与版本计数，包含构建依赖、过程宏和其他平台条件依赖，并非 100 个包都会进入 Linux 二进制。

- 98 个包有不附带其他强制组合许可的 MIT 路径，或本身就是 MIT。
- `unicode-ident 1.0.26` 为 `(MIT OR Apache-2.0) AND Unicode-3.0`，可选择 MIT，但仍须履行适用的 Unicode 数据许可条件。
- `ksni 0.3.6` 为 Unlicense，兼容本项目采用 MIT；第三方原授权仍应如实记录。
- `r-efi 6.0.0` 的 `MIT OR Apache-2.0 OR LGPL-2.1-or-later` 是三选一，可选择 MIT，不意味着必须使用 LGPL。它是条件依赖，当前 Linux 目标未使用该 UEFI 实现。本机 crate 包未附顶层许可正文，若将其纳入分发清单，应从相应上游版本补齐正文，不能只复制 SPDX 字段。
- 其余 crate 的本机源码包均找到顶层 LICENSE／UNLICENSE 文件；抽查了 ksni、unicode-ident、zbus、serde、regex、toml_edit 的许可正文。
- Cargo.toml 没有 `[patch]`、本地替换依赖或 Git 源码依赖。

双许可证表达式中的 `OR` 允许选择一条许可路径；`AND` 要求同时满足。选择 MIT 不会免除原版权声明及许可正文的保留要求。[MIT 正文](https://opensource.org/license/mit)、[Unicode License V3](https://www.unicode.org/license.txt)。

## Python 与 Qt

`ui/panel.py`、`ui/quick_menu.py` 使用 PySide6 的 QtCore、QtGui、QtWidgets、QtNetwork；测试／预览还使用 QtTest。未发现本项目导入 QtCharts、QtGraphs、QtNetworkAuth、QtVirtualKeyboard 等可能只有 GPL／商业许可路径的模块。

本机 PySide6 为 6.12.0，包元数据声明 `LGPL-3.0-only OR GPL-2.0-only OR GPL-3.0-only`。用于本项目的开源路径应选择 LGPL，而不是声称 PySide6 本身是 MIT。

Rust 二进制内嵌的是本项目的 Python 源码，通过外部 Python 解释器执行、动态导入用户安装的 PySide6。所检查的发布包未捆绑 PySide6、Shiboken、Qt 共享库、Python 解释器或字体文件。静态 musl 可执行文件不等于静态链接 Qt。

按这个实现，可在履行 LGPL 条件的前提下为自己的应用代码采用 MIT。应在项目第三方声明中说明 Qt／PySide6 的使用及 LGPL 路径，并提供适用的许可正文。如果以后改成 PyInstaller／Nuitka／AppImage 等捆绑 Qt 的分发方式，需重新清点实际带入的库、插件和第三方模块，满足相应的源码提供、声明、替换／重新链接等要求；仅提供一个上游首页链接并不能替代全部义务。

依据：[Qt LGPL 义务](https://www.qt.io/licensing/open-source-lgpl-obligations)、[Qt 总体许可与 GPL 模块范围](https://doc.qt.io/qt-6/licensing.html)、[Qt for Python 第三方许可](https://doc.qt.io/qtforpython-6/licenses.html)、[Qt Widgets](https://doc.qt.io/qt-6/qtwidgets-index.html)、[Qt Network](https://doc.qt.io/qt-6/qtnetwork-index.html)、[Qt GUI](https://doc.qt.io/qt-6/qtgui-index.html)。

## 外部程序与工具

| 组件 | 本机／上游许可证据 | 使用方式与判断 |
| --- | --- | --- |
| tmux | 宽松的 BSD／ISC 风格许可；本机包归类 BSD | 外部命令与会话宿主；兼容 MIT 应用 |
| Python | PSF-2.0 等 | 外部解释器；当前包不分发解释器 |
| xdg-utils | MIT | 通过 xdg-open 打开浏览器等 |
| libnotify | LGPL-2.1-or-later | 通过外部 notify-send 调用 |
| yad | GPL-3.0-or-later | 可选外部表单程序，通过普通命令行接口启动，未链接或捆绑；按此结构未发现要求 devtray 改为 GPL 的依据 |
| PyGObject | LGPL-2.1-or-later | scripts/menu.py 及托盘集成测试导入；不属于主面板依赖 |
| librsvg | LGPL-2.1-or-later | 通过 rsvg-convert 构建图标；生成的自有图标不会仅因使用该工具而变成 LGPL |

这些组件的包许可来自本机 `pacman -Qi`。tmux 的许可正文另核对了本机文件和[上游 COPYING](https://raw.githubusercontent.com/tmux/tmux/master/COPYING)。GPL 外部程序的判断基于当前普通子进程调用方式，而非“一切进程通信都豁免”；[GNU FAQ](https://www.gnu.org/licenses/gpl-faq.en.html#MereAggregation)要求结合通信机制和语义判断程序边界。

Rust 标准库及静态运行库不在 Cargo.lock 的这 100 个包内。Rust 主要采用 MIT／Apache-2.0，部分代码另有宽松许可；须按构建工具链的 `COPYRIGHT-library.html` 等记录保留适用归属。另已通过 HTTPS 读取 musl 上游 COPYRIGHT：整体为 MIT，部分第三方实现使用 BSD 等兼容宽松许可；上游还对公开头文件和特定启动文件提供通知豁免。当前产物为包含 libc 实现的静态二进制，不能将该豁免理解为全部 musl 都无需声明。仍应按实际版本整理 musl 及适用部分的归属，不能只审 Cargo.lock。[musl COPYRIGHT](https://git.musl-libc.org/cgit/musl/plain/COPYRIGHT)。[Rust 许可说明](https://rust-lang.org/policies/licenses/)、[Rust COPYRIGHT 对标准库清单的说明](https://github.com/rust-lang/rust/blob/main/COPYRIGHT)。

## 图标、截图和源码来源

四个 SVG 图标是仓库内的简单几何路径，没有外部图片链接或第三方图标包导入；PNG 和 ARGB 有对应的本地生成脚本。截图脚本使用演示数据和本项目界面，没有随包字体文件。未发现 vendored 第三方源码或额外的版权／许可文件头。

这些证据说明未发现明显的外来授权限制，但不能据此认证原创归属。如果图标、代码或文案曾从其他项目复制，应继续保留其许可并核对兼容性。文档对 ServBay／lnav 的功能参考链接本身不等于复制它们的源码或资源。

## v0.1.0 检查发现的问题（v0.1.1 已处理）

1. v0.1.0 当时未采用 MIT：Cargo.toml 的 package.license 和 RPM 元数据均为 Proprietary，中英文 README 同样保留专有许可说明，仓库没有顶层 LICENSE。
2. v0.1.0 的三种发布包缺少完整第三方归属声明：tar.gz／rpm 未发现 LICENSE／NOTICE；deb 的 copyright 文件仅列 ferychen 与 Proprietary。静态纳入的 MIT 依赖本身就要求保留其许可声明，此问题不只在改 MIT 时才需要处理。
3. v0.1.1 切换时添加顶层 MIT LICENSE，统一修改 Cargo 元数据、RPM 元数据和 README，生成第三方声明并纳入 tar／deb／rpm。声明至少覆盖实际使用的 Rust 依赖、适用 Unicode 内容、Rust 标准库／musl 以及 Qt／PySide6 运行时说明。
4. v0.1.1 重新制作发布包并核验许可文件实际进入包体。已发布的 v0.1.0 内容不会因只修改 main 分支而自动改变；后续发行需明确对应源码、许可及校验和。

## Rust 全量解析清单

下表来自锁定版本的 Cargo 元数据；许可原文和最终分发路径仍以相应上游版本为准。

| Package | 版本 | 声明的 SPDX 表达式 | 兼容路径 |
| --- | --- | --- | --- |
| `aho-corasick` | 1.1.5 | `Unlicense OR MIT` | MIT |
| `async-broadcast` | 0.7.2 | `MIT OR Apache-2.0` | MIT |
| `async-channel` | 2.5.0 | `Apache-2.0 OR MIT` | MIT |
| `async-executor` | 1.14.0 | `Apache-2.0 OR MIT` | MIT |
| `async-io` | 2.6.0 | `Apache-2.0 OR MIT` | MIT |
| `async-lock` | 3.4.2 | `Apache-2.0 OR MIT` | MIT |
| `async-process` | 2.5.0 | `Apache-2.0 OR MIT` | MIT |
| `async-recursion` | 1.1.1 | `MIT OR Apache-2.0` | MIT |
| `async-signal` | 0.2.14 | `Apache-2.0 OR MIT` | MIT |
| `async-task` | 4.7.1 | `Apache-2.0 OR MIT` | MIT |
| `async-trait` | 0.1.92 | `MIT OR Apache-2.0` | MIT |
| `atomic-waker` | 1.1.2 | `Apache-2.0 OR MIT` | MIT |
| `autocfg` | 1.5.1 | `Apache-2.0 OR MIT` | MIT |
| `bitflags` | 2.13.2 | `MIT OR Apache-2.0` | MIT |
| `blocking` | 1.7.0 | `Apache-2.0 OR MIT` | MIT |
| `bumpalo` | 3.20.3 | `MIT OR Apache-2.0` | MIT |
| `cfg-if` | 1.0.5 | `MIT OR Apache-2.0` | MIT |
| `concurrent-queue` | 2.5.0 | `Apache-2.0 OR MIT` | MIT |
| `crossbeam-utils` | 0.8.23 | `MIT OR Apache-2.0` | MIT |
| `endi` | 1.1.1 | `MIT` | MIT |
| `enumflags2` | 0.7.12 | `MIT OR Apache-2.0` | MIT |
| `enumflags2_derive` | 0.7.12 | `MIT OR Apache-2.0` | MIT |
| `equivalent` | 1.0.2 | `Apache-2.0 OR MIT` | MIT |
| `errno` | 0.3.14 | `MIT OR Apache-2.0` | MIT |
| `event-listener` | 5.4.2 | `Apache-2.0 OR MIT` | MIT |
| `event-listener-strategy` | 0.5.4 | `Apache-2.0 OR MIT` | MIT |
| `fastrand` | 2.5.0 | `Apache-2.0 OR MIT` | MIT |
| `futures-channel` | 0.3.34 | `MIT OR Apache-2.0` | MIT |
| `futures-core` | 0.3.34 | `MIT OR Apache-2.0` | MIT |
| `futures-io` | 0.3.34 | `MIT OR Apache-2.0` | MIT |
| `futures-lite` | 2.6.1 | `Apache-2.0 OR MIT` | MIT |
| `futures-macro` | 0.3.34 | `MIT OR Apache-2.0` | MIT |
| `futures-task` | 0.3.34 | `MIT OR Apache-2.0` | MIT |
| `futures-util` | 0.3.34 | `MIT OR Apache-2.0` | MIT |
| `getrandom` | 0.4.3 | `MIT OR Apache-2.0` | MIT |
| `hashbrown` | 0.17.1 | `MIT OR Apache-2.0` | MIT |
| `hermit-abi` | 0.5.3 | `MIT OR Apache-2.0` | MIT |
| `hex` | 0.4.3 | `MIT OR Apache-2.0` | MIT |
| `indexmap` | 2.14.2 | `Apache-2.0 OR MIT` | MIT |
| `itoa` | 1.0.18 | `MIT OR Apache-2.0` | MIT |
| `js-sys` | 0.3.105 | `MIT OR Apache-2.0` | MIT |
| `ksni` | 0.3.6 | `Unlicense` | Unlicense |
| `libc` | 0.2.189 | `MIT OR Apache-2.0` | MIT |
| `linux-raw-sys` | 0.12.1 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | MIT |
| `memchr` | 2.8.3 | `Unlicense OR MIT` | MIT |
| `memoffset` | 0.9.1 | `MIT` | MIT |
| `once_cell` | 1.21.4 | `MIT OR Apache-2.0` | MIT |
| `ordered-stream` | 0.2.0 | `MIT OR Apache-2.0` | MIT |
| `parking` | 2.2.1 | `Apache-2.0 OR MIT` | MIT |
| `pastey` | 0.2.3 | `MIT OR Apache-2.0` | MIT |
| `pin-project-lite` | 0.2.17 | `Apache-2.0 OR MIT` | MIT |
| `piper` | 0.2.5 | `MIT OR Apache-2.0` | MIT |
| `polling` | 3.11.0 | `Apache-2.0 OR MIT` | MIT |
| `proc-macro-crate` | 3.5.0 | `MIT OR Apache-2.0` | MIT |
| `proc-macro2` | 1.0.107 | `MIT OR Apache-2.0` | MIT |
| `quote` | 1.0.47 | `MIT OR Apache-2.0` | MIT |
| `r-efi` | 6.0.0 | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | MIT |
| `regex` | 1.13.1 | `MIT OR Apache-2.0` | MIT |
| `regex-automata` | 0.4.18 | `MIT OR Apache-2.0` | MIT |
| `regex-syntax` | 0.8.11 | `MIT OR Apache-2.0` | MIT |
| `rustix` | 1.1.5 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | MIT |
| `rustversion` | 1.0.23 | `MIT OR Apache-2.0` | MIT |
| `serde` | 1.0.229 | `MIT OR Apache-2.0` | MIT |
| `serde_core` | 1.0.229 | `MIT OR Apache-2.0` | MIT |
| `serde_derive` | 1.0.229 | `MIT OR Apache-2.0` | MIT |
| `serde_json` | 1.0.151 | `MIT OR Apache-2.0` | MIT |
| `serde_repr` | 0.1.21 | `MIT OR Apache-2.0` | MIT |
| `serde_spanned` | 1.1.1 | `MIT OR Apache-2.0` | MIT |
| `signal-hook-registry` | 1.4.8 | `MIT OR Apache-2.0` | MIT |
| `slab` | 0.4.12 | `MIT` | MIT |
| `syn` | 2.0.119 | `MIT OR Apache-2.0` | MIT |
| `syn` | 3.0.6 | `MIT OR Apache-2.0` | MIT |
| `task-local` | 0.1.1 | `MIT OR Apache-2.0` | MIT |
| `tempfile` | 3.27.0 | `MIT OR Apache-2.0` | MIT |
| `toml` | 1.1.6+spec-1.1.0 | `MIT OR Apache-2.0` | MIT |
| `toml_datetime` | 1.1.1+spec-1.1.0 | `MIT OR Apache-2.0` | MIT |
| `toml_edit` | 0.25.15+spec-1.1.0 | `MIT OR Apache-2.0` | MIT |
| `toml_parser` | 1.1.3+spec-1.1.0 | `MIT OR Apache-2.0` | MIT |
| `toml_writer` | 1.1.2+spec-1.1.0 | `MIT OR Apache-2.0` | MIT |
| `tracing` | 0.1.44 | `MIT` | MIT |
| `tracing-attributes` | 0.1.31 | `MIT` | MIT |
| `tracing-core` | 0.1.36 | `MIT` | MIT |
| `uds_windows` | 1.2.1 | `MIT` | MIT |
| `unicode-ident` | 1.0.26 | `(MIT OR Apache-2.0) AND Unicode-3.0` | MIT + Unicode-3.0 |
| `uuid` | 1.26.1 | `Apache-2.0 OR MIT` | MIT |
| `wasm-bindgen` | 0.2.128 | `MIT OR Apache-2.0` | MIT |
| `wasm-bindgen-macro` | 0.2.128 | `MIT OR Apache-2.0` | MIT |
| `wasm-bindgen-macro-support` | 0.2.128 | `MIT OR Apache-2.0` | MIT |
| `wasm-bindgen-shared` | 0.2.128 | `MIT OR Apache-2.0` | MIT |
| `windows-link` | 0.2.1 | `MIT OR Apache-2.0` | MIT |
| `windows-sys` | 0.61.2 | `MIT OR Apache-2.0` | MIT |
| `winnow` | 1.0.4 | `MIT` | MIT |
| `zbus` | 5.19.0 | `MIT` | MIT |
| `zbus_macros` | 5.19.0 | `MIT` | MIT |
| `zbus_names` | 4.3.4 | `MIT` | MIT |
| `zcheapstr` | 1.1.0 | `MIT` | MIT |
| `zmij` | 1.0.23 | `MIT` | MIT |
| `zvariant` | 5.15.0 | `MIT` | MIT |
| `zvariant_derive` | 5.15.0 | `MIT` | MIT |
| `zvariant_utils` | 4.2.0 | `MIT` | MIT |
