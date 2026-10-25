# Third-party license texts

These files preserve third-party licensing; the project's MIT LICENSE does not
relicense the software described here. See [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

- `rust-crates.txt`: upstream license and copyright files for the target's Cargo
  graph, collected verbatim by `scripts/generate-third-party-notices.py`.
- `rust/COPYRIGHT-library.html`: the build toolchain's complete Rust standard
  library notice document, including embedded third-party license texts.
- `rust/TOOLCHAIN.txt`: compiler version associated with that document.
- `musl-COPYRIGHT.txt`: musl 1.2.5 upstream COPYRIGHT.
- `musl-THIRD-PARTY.txt`: copyright/license comment blocks from upstream musl
  1.2.5 source files, including other architectures for conservative attribution.
  Source: https://musl.libc.org/releases/musl-1.2.5.tar.gz
  The archive's SHA-256 is recorded in that notice file.
- `llvm-runtime-LICENSE.txt` and `llvm-libunwind-LICENSE.txt`: LLVM 22.1.6 license
  documents, as used by the Rust 1.97.1 toolchain.
  Sources: https://github.com/llvm/llvm-project/tree/llvmorg-22.1.6
- `LGPL-3.0-only.txt` and `GPL-3.0-only.txt`: unchanged license texts provided with
  Qt 6.12 on the build machine. The GPL text is included because LGPLv3 incorporates
  it; its presence does not mean devtray is licensed under GPL.

Rust documents are regenerated from the active compiler, while the musl/LLVM
snapshots need a new review when the runtime versions change. For v0.1.1 the
Rust build script identifies musl 1.2.5 plus Rust-maintained patches:
https://github.com/rust-lang/rust/blob/8bab26f4f68e0e26f0bb7960be334d5b520ea452/src/ci/docker/scripts/musl.sh

Install the compiler's notice document if missing with:

```bash
rustup component add rust-docs
```

自有项目采用 MIT，第三方组件保留原有许可。上述文件与安装包一同分发；升级依赖、
编译工具链或改成捆绑 Qt 的分发方式时，请同步更新声明。
