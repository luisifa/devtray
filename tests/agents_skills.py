"""Agent 检测与 Skills 清单（只读）回归：临时 HOME，不访问网络，不改真实技能。"""
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get('DEVTRAY_TEST_BINARY', str(ROOT / 'target/debug/devtray'))

with tempfile.TemporaryDirectory(prefix='devtray-agents-') as tmpdir:
    home = Path(tmpdir)
    skills = home / '.agents' / 'skills'
    for name, body in [('alpha', 'alpha'), ('beta', 'beta')]:
        target = skills / name
        target.mkdir(parents=True)
        (target / 'SKILL.md').write_text(f'---\nname: {name}\n---\n# {body}\n')
    (home / '.claude' / 'skills' / 'alpha').mkdir(parents=True)
    (home / '.claude' / 'skills' / 'alpha' / 'SKILL.md').write_text(
        '---\nname: alpha\n---\n# alpha\n'
    )
    (home / '.claude' / 'skills' / 'beta').mkdir(parents=True)
    (home / '.claude' / 'skills' / 'beta' / 'SKILL.md').write_text(
        '---\nname: beta\n---\n# changed\n'
    )
    (home / '.agents' / '.skill-lock.json').write_text(json.dumps({
        'version': 3,
        'skills': {'alpha': {
            'source': 'owner/repo', 'sourceType': 'github',
            'sourceUrl': 'https://github.com/owner/repo',
            'skillPath': 'skills/alpha/SKILL.md',
            'skillFolderHash': '0123456789abcdef0123456789abcdef01234567',
            'installedAt': '2026-01-01T00:00:00Z',
            'updatedAt': '2026-01-01T00:00:00Z',
        }},
    }))
    bin_dir = home / 'bin'
    bin_dir.mkdir()
    claude = bin_dir / 'claude'
    claude.write_text('#!/bin/sh\necho "9.9.9 (Claude Code)"\n')
    claude.chmod(0o755)
    env = dict(os.environ, HOME=str(home), PATH=f'{bin_dir}:{os.environ["PATH"]}',
               DEVTRAY_CONFIG=str(home / 'config.toml'))

    agents = subprocess.run([BINARY, 'agents', '--json'], env=env,
                            capture_output=True, text=True, check=True)
    detected = json.loads(agents.stdout)
    claude_info = next(a for a in detected if a['id'] == 'claude-code')
    assert claude_info['version'] == '9.9.9', claude_info
    assert claude_info['provenance'] == 'unknown', claude_info
    assert claude_info['update_command'] is None

    skills_result = subprocess.run([BINARY, 'skills', '--json'], env=env,
                                  capture_output=True, text=True, check=True)
    inventory = json.loads(skills_result.stdout)
    summary = inventory['summary']
    assert summary['total'] == 2, summary
    assert summary['managed'] == 1 and summary['unmanaged'] == 1, summary
    assert summary['lock_ok'] is True and summary['lock_version'] == 3
    alpha = next(s for s in inventory['skills'] if s['name'] == 'alpha')
    beta = next(s for s in inventory['skills'] if s['name'] == 'beta')
    assert alpha['managed'] and alpha['update'] == 'unchecked'
    assert not beta['managed']
    assert beta['mirrors'] and beta['mirrors'][0]['status'] == 'drift', beta
    assert summary['drift_mirrors'] == 1, summary

print('PASS: agent version/provenance, skills lock reconciliation, unmanaged, mirror drift')
