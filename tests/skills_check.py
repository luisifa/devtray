"""Skills 更新检查（只读）：用假 gh 验证 tree sha 对比与 6h 缓存，不访问网络。"""
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get('DEVTRAY_TEST_BINARY', str(ROOT / 'target/debug/devtray'))

with tempfile.TemporaryDirectory(prefix='devtray-skillcheck-') as tmpdir:
    home = Path(tmpdir)
    for name in ('alpha', 'beta'):
        target = home / '.agents' / 'skills' / name
        target.mkdir(parents=True)
        (target / 'SKILL.md').write_text(f'---\nname: {name}\n---\n# {name}\n')
    (home / '.agents' / '.skill-lock.json').write_text(json.dumps({
        'version': 3,
        'skills': {
            'alpha': {
                'source': 'owner/repo', 'sourceType': 'github',
                'sourceUrl': 'https://github.com/owner/repo.git', 'ref': 'main',
                'skillPath': 'skills/alpha/SKILL.md',
                'skillFolderHash': 'a' * 40,
                'installedAt': '2026-01-01T00:00:00Z',
            },
            'beta': {
                'source': 'owner/repo', 'sourceType': 'github',
                'sourceUrl': 'https://github.com/owner/repo.git', 'ref': 'main',
                'skillPath': 'skills/beta/SKILL.md',
                'skillFolderHash': 'b' * 40,
                'installedAt': '2026-01-01T00:00:00Z',
            },
        },
    }))
    tree = {
        'sha': 'root-sha', 'truncated': False,
        'tree': [
            {'path': 'skills', 'type': 'tree', 'sha': 'skills-sha'},
            {'path': 'skills/alpha', 'type': 'tree', 'sha': 'b' * 40},
            {'path': 'skills/beta', 'type': 'tree', 'sha': 'b' * 40},
        ],
    }
    calls = home / 'gh-calls.log'
    fake = home / 'bin' / 'gh'
    fake.parent.mkdir()
    fake.write_text(
        '#!/bin/sh\n'
        f'echo "$@" >> "{calls}"\n'
        'case "$2" in\n'
        f'  *git/trees/main*) cat "{home / "tree.json"}" ;;\n'
        '  *) echo "{}" ;;\n'
        'esac\n'
    )
    fake.chmod(0o755)
    (home / 'tree.json').write_text(json.dumps(tree))
    config = home / 'config.toml'
    config.write_text('')
    env = dict(
        os.environ, HOME=str(home), PATH=f'{fake.parent}:{os.environ["PATH"]}',
        XDG_STATE_HOME=str(home / 'state'), DEVTRAY_CONFIG=str(config),
    )

    first = subprocess.run([BINARY, 'skills', 'check', '--json'], env=env,
                           capture_output=True, text=True, check=True)
    inventory = json.loads(first.stdout)
    summary = inventory['summary']
    assert summary['update_available'] == 1, summary
    assert summary['update_current'] == 1, summary
    alpha = next(s for s in inventory['skills'] if s['name'] == 'alpha')
    beta = next(s for s in inventory['skills'] if s['name'] == 'beta')
    assert alpha['update'] == 'available', alpha
    assert alpha['latest_sha'] == 'b' * 40, alpha
    assert beta['update'] == 'current', beta
    calls_after_first = calls.read_text().count('git/trees/main')

    # 6h 缓存内重复 check 不应再次请求上游
    subprocess.run([BINARY, 'skills', 'check', '--json'], env=env,
                   capture_output=True, text=True, check=True)
    assert calls.read_text().count('git/trees/main') == calls_after_first

    # --refresh 必须绕过缓存重新请求
    subprocess.run([BINARY, 'skills', 'check', '--refresh', '--json'], env=env,
                   capture_output=True, text=True, check=True)
    assert calls.read_text().count('git/trees/main') > calls_after_first

print('PASS: tree sha comparison, available/current, 6h cache, refresh bypass')
