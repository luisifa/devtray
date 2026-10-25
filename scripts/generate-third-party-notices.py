#!/usr/bin/env python3
"""Collect target-specific crate licenses and the matching Rust library notices.

Run after cargo build, so Cargo's source cache is populated. No dependencies or
network are needed. Deliberately reject unreviewed license expressions rather
than silently applying the project's MIT license to third-party code.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
REVIEWED_LICENSES = {
    'MIT': 'MIT',
    'MIT OR Apache-2.0': 'MIT',
    'Apache-2.0 OR MIT': 'MIT',
    'Unlicense OR MIT': 'MIT',
    'Unlicense': 'Unlicense',
    '(MIT OR Apache-2.0) AND Unicode-3.0': 'MIT AND Unicode-3.0',
    'Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT': 'MIT',
}


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True)


def generate(target):
    metadata = json.loads(command(
        'cargo', 'metadata', '--format-version', '1', '--locked', '--offline',
        '--filter-platform', target,
    ))
    root_id = metadata['resolve']['root']
    ids = {node['id'] for node in metadata['resolve']['nodes']}
    packages = sorted(
        (p for p in metadata['packages'] if p['id'] in ids and p['id'] != root_id),
        key=lambda p: (p['name'], p['version']),
    )
    library_notices = Path(command('rustc', '--print', 'sysroot').strip()) / 'share/doc/rust/COPYRIGHT-library.html'
    if not library_notices.is_file():
        raise SystemExit('Rust library notices missing. Run: rustup component add rust-docs')
    rust_version = command('rustc', '--version').strip()
    if rust_version != 'rustc 1.97.1 (8bab26f4f 2026-07-14)':
        raise SystemExit(
            'Runtime license snapshots were reviewed for Rust 1.97.1. '
            'Use that toolchain or review/update the musl and LLVM notices first.'
        )
    license_dir = ROOT / 'licenses'
    required = (
        'LGPL-3.0-only.txt', 'GPL-3.0-only.txt', 'musl-COPYRIGHT.txt',
        'musl-THIRD-PARTY.txt', 'llvm-libunwind-LICENSE.txt', 'llvm-runtime-LICENSE.txt',
    )
    for name in required:
        if not (license_dir / name).is_file():
            raise SystemExit(f'Missing runtime notice: licenses/{name}')
    sections = [
        'Third-party Rust crate notices for devtray\n',
        f'Target: {target}\nToolchain: {rust_version}\n',
        'The following upstream files are copied verbatim, including alternative\n'
        'licenses and supplementary copyright notices. License choices are listed\n'
        'in THIRD_PARTY_NOTICES.md. Build dependencies are included conservatively.\n',
    ]
    table = []
    for package in packages:
        expression = package.get('license')
        if expression not in REVIEWED_LICENSES:
            raise SystemExit(f'License review required: {package["name"]} {expression!r}')
        directory = Path(package['manifest_path']).parent
        files = sorted(
            f for f in directory.iterdir()
            if f.is_file() and any(
                word in f.name.lower()
                for word in ('license', 'unlicense', 'copying', 'copyright', 'notice')
            )
        )
        if not files:
            raise SystemExit(f'No license texts found for {package["name"]}')
        contents = [(f.name, f.read_text()) for f in files]
        combined = '\n'.join(text for _, text in contents)
        if 'MIT' in REVIEWED_LICENSES[expression] and 'Permission is hereby granted' not in combined:
            raise SystemExit(f'MIT permission text missing for {package["name"]}')
        if 'Unicode-3.0' in expression and 'UNICODE LICENSE V3' not in combined:
            raise SystemExit(f'Unicode license missing for {package["name"]}')
        name = f'{package["name"]} {package["version"]}'
        sections.extend([
            '\n' + '=' * 78 + '\n', name + '\n',
            f'Declared license: {expression}\n',
            f'License choice: {REVIEWED_LICENSES[expression]}\n',
            'Authors: ' + '; '.join(package.get('authors', [])) + '\n',
            f'Source: https://crates.io/crates/{package["name"]}/{package["version"]}\n',
        ])
        for filename, text in contents:
            sections.extend(['\n--- ' + filename + ' ---\n\n', text, '\n'])
        table.append(f'| `{name}` | `{expression}` | `{REVIEWED_LICENSES[expression]}` |')
    (license_dir / 'rust').mkdir(exist_ok=True)
    shutil.copyfile(library_notices, license_dir / 'rust/COPYRIGHT-library.html')
    (license_dir / 'rust/TOOLCHAIN.txt').write_text(rust_version + '\n')
    (license_dir / 'rust-crates.txt').write_text(''.join(sections))
    lock_hash = hashlib.sha256((ROOT / 'Cargo.lock').read_bytes()).hexdigest()
    header = f'''# Third-party notices / 第三方许可声明

The MIT license in LICENSE applies to devtray's own code, documentation, and
project-created assets. Third-party components retain their own copyright and
license terms. / 项目 MIT 许可不替代第三方组件的原有许可。

## Rust dependencies

- Target: `{target}`
- Toolchain: `{rust_version}`
- Cargo.lock SHA-256: `{lock_hash}`
- {len(packages)} third-party packages in the target's resolved dependency graph,
  including build-time tools and procedural macros. This is an inclusive list;
  it does not mean every package's code is present in the executable.
- Full original license and copyright files: [licenses/rust-crates.txt](licenses/rust-crates.txt).
- Where a crate offers alternative licenses, the choice below is used for this
  distribution. Reproducing the other upstream texts does not change that choice.
- `unicode-ident` also requires Unicode-3.0; the applicable Unicode text is included.

| Package and version | Upstream SPDX expression | License choice |
| --- | --- | --- |
'''
    footer = '''
## Static runtime components

- Rust standard library: MIT OR Apache-2.0, with separately attributed portions.
  The toolchain's complete [library copyright notices](licenses/rust/COPYRIGHT-library.html)
  are reproduced, including their embedded license texts and platform-specific
  dependencies. [Toolchain version](licenses/rust/TOOLCHAIN.txt).
- Linux musl builds also statically include musl 1.2.5 (with the Rust toolchain's
  patches). See [musl copyright](licenses/musl-COPYRIGHT.txt) and
  [source-file notices](licenses/musl-THIRD-PARTY.txt). The latter conservatively
  includes notices from all architectures of upstream musl 1.2.5.
- The supplied Rust toolchain uses LLVM 22.1.6. Runtime/unwind notices:
  [LLVM project](licenses/llvm-runtime-LICENSE.txt) and
  [libunwind](licenses/llvm-libunwind-LICENSE.txt), under Apache-2.0 WITH
  LLVM-exception and the upstream historical license terms reproduced there.
- These musl/LLVM snapshots were reviewed for the v0.1.1 Linux release toolchain.
  Review them again when changing the target or Rust runtime version; regenerating
  crate notices alone does not update these upstream snapshots.

## Optional Python / PySide6 / Qt runtime

The desktop panel uses PySide6/Qt Core, Gui, Widgets, and Network through an
external Python interpreter. Development tests also use QtTest. This release
does not bundle Python, PySide6, Shiboken, or Qt libraries. Users install them
separately and can replace their interpreter/library installation through
DEVTRAY_PYTHON and normal Python package management. There is no Qt static link.

The open-source Qt/PySide6 runtime uses the LGPLv3 option. Copies of
[LGPLv3](licenses/LGPL-3.0-only.txt) and the incorporated
[GPLv3](licenses/GPL-3.0-only.txt) are supplied. Third-party parts of Qt retain
their respective terms. The MIT project license imposes no restriction on
replacing, modifying, or debugging these libraries, including reverse engineering
for debugging such modifications. No Qt/PySide6 library modifications are supplied.

If redistributing Qt or a frozen Python/Qt bundle, provide the complete
corresponding source or another license-compliant source delivery method,
required notices, and the ability to replace/relink the LGPL components. A link
to the upstream home page alone is not a substitute for these obligations.

- [PySide source](https://code.qt.io/pyside/pyside-setup.git/)
- [Qt source](https://code.qt.io/qt/qtbase.git/)
- [Qt for Python license details](https://doc.qt.io/qtforpython-6/licenses.html)
- [Qt LGPL requirements](https://www.qt.io/licensing/open-source-lgpl-obligations)

## External programs and development tools

The following are independently installed programs/libraries, not bundled in the
release packages: Python (PSF-2.0 and associated notices), tmux (ISC/BSD-style),
xdg-utils (MIT), libnotify/notify-send (LGPL-2.1-or-later), optional yad
(GPL-3.0-or-later), PyGObject (LGPL-2.1-or-later, used by development scripts),
and librsvg/rsvg-convert (LGPL-2.1-or-later, used to generate project artwork).
Their own distributions contain their applicable licenses and source information.

## 中文说明

devtray 自有代码、文档和自制资源采用 MIT。Rust 依赖许可原文、Unicode 声明及
静态运行库声明随包提供；第三方组件继续适用原有许可。清单包含构建时依赖，
不表示所有列出的源码都会链接进可执行文件。PySide6／Qt 为独立安装的 LGPLv3
运行时，未随本包捆绑；若自行制作捆绑包，需要另行履行源码提供、许可声明和
允许用户替换／重新链接库等义务。

Regenerate after building the target:
`python3 scripts/generate-third-party-notices.py --target x86_64-unknown-linux-musl`.
'''
    (ROOT / 'THIRD_PARTY_NOTICES.md').write_text(header + '\n'.join(table) + '\n' + footer)
    print(f'Collected license texts for {len(packages)} packages ({target})')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', default='x86_64-unknown-linux-musl')
    generate(parser.parse_args().target)
