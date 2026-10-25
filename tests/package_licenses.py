"""Verify that every release format carries the reviewed license documents.

Requires bsdtar (libarchive-tools on Debian/Ubuntu). Run after package-release.sh.
"""
import hashlib
import io
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = re.search(r'^version = "([^"]+)"', (ROOT / 'Cargo.toml').read_text(), re.M)[1]
DIST = ROOT / 'dist'
NOTICE_FILES = [ROOT / 'LICENSE', ROOT / 'THIRD_PARTY_NOTICES.md'] + [
    path for path in (ROOT / 'licenses').rglob('*') if path.is_file()
]

with tempfile.TemporaryDirectory(prefix='devtray-package-licenses-') as tmp:
    base = Path(tmp)
    deb = DIST / f'devtray_{VERSION}-1_amd64.deb'
    archive = subprocess.check_output(['bsdtar', '-xOf', str(deb), 'data.tar.xz'])
    data = base / 'deb-data.tar.xz'
    data.write_bytes(archive)
    control = subprocess.check_output(['bsdtar', '-xOf', str(deb), 'control.tar.xz'])
    with tarfile.open(fileobj=io.BytesIO(control)) as records:
        contents = records.extractfile('./control').read().decode()
        assert f'Version: {VERSION}-1' in contents, contents
    formats = {
        'deb': data,
        'rpm': DIST / f'devtray-{VERSION}-1.x86_64.rpm',
        'tar': DIST / f'devtray-{VERSION}-x86_64-unknown-linux-musl.tar.gz',
    }
    for kind, artifact in formats.items():
        dest = base / kind
        dest.mkdir()
        subprocess.run(['bsdtar', '-xf', str(artifact), '-C', str(dest)], check=True)
        docs = dest / (f'devtray-{VERSION}-x86_64-unknown-linux-musl' if kind == 'tar' else 'usr/share/doc/devtray')
        for source in NOTICE_FILES:
            installed = docs / source.relative_to(ROOT)
            assert installed.is_file(), f'{kind}: missing {installed.relative_to(dest)}'
            assert installed.read_bytes() == source.read_bytes(), f'{kind}: stale notice {source}'
        if kind == 'deb':
            copyright_text = (docs / 'copyright').read_text()
            assert 'License: MIT' in copyright_text and 'Proprietary' not in copyright_text
        if kind == 'rpm':
            assert (dest / 'usr/share/licenses/devtray/LICENSE').read_bytes() == (ROOT / 'LICENSE').read_bytes()
        binary = docs / 'devtray' if kind == 'tar' else dest / 'usr/bin/devtray'
        assert subprocess.check_output([str(binary), '--version'], text=True).strip() == f'devtray {VERSION}'
        subprocess.run(
            ['python3', str(ROOT / 'tests/mcp.py')], check=True, cwd=ROOT,
            env=dict(os.environ, DEVTRAY_TEST_BINARY=str(binary)),
        )
        print(f'PASS: {kind} version, MCP, and {len(NOTICE_FILES)} complete license documents')
for line in (DIST / 'SHA256SUMS').read_text().splitlines():
    expected, filename = line.split()
    assert hashlib.sha256((DIST / filename).read_bytes()).hexdigest() == expected, filename
print('PASS: release SHA-256 checksums')
