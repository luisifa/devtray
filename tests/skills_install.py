"""Skills 安装管线：假 gh + 假 npx，验证预检、选择、agent 显式传参与安装后校验。"""
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get('DEVTRAY_TEST_BINARY', str(ROOT / 'target/debug/devtray'))

with tempfile.TemporaryDirectory(prefix='devtray-skillinstall-') as tmpdir:
    home = Path(tmpdir)
    tree = {
        'sha': 'root-sha', 'truncated': False,
        'tree': [
            {'path': 'skills', 'type': 'tree', 'sha': 't1'},
            {'path': 'skills/alpha/SKILL.md', 'type': 'blob', 'sha': 'b1'},
            {'path': 'skills/beta/SKILL.md', 'type': 'blob', 'sha': 'b2'},
        ],
    }
    (home / 'tree.json').write_text(json.dumps(tree))
    bin_dir = home / 'bin'
    bin_dir.mkdir()
    gh = bin_dir / 'gh'
    gh.write_text(f'''#!/bin/sh
case "$2" in
  *git/trees/*) cat "{home / 'tree.json'}" ;;
  *repos/*) echo '{{"default_branch":"main"}}' ;;
  *) echo '{{}}' ;;
esac
''')
    gh.chmod(0o755)
    npx = bin_dir / 'npx'
    npx.write_text('''#!/usr/bin/env python3
import json, os, shutil, sys
from pathlib import Path
args = sys.argv[1:]
if 'add' not in args:
    sys.exit(2)
skill = args[args.index('--skill') + 1]
source = args[args.index('add') + 1]
agents = [args[i + 1] for i, a in enumerate(args) if a == '-a']
home = Path(os.environ['HOME'])
target = home / '.agents' / 'skills' / skill
target.mkdir(parents=True, exist_ok=True)
(target / 'SKILL.md').write_text('---\\nname: ' + skill + '\\n---\\n# installed\\n')
lock_path = home / '.agents' / '.skill-lock.json'
lock = json.loads(lock_path.read_text()) if lock_path.exists() else {'version': 3, 'skills': {}}
lock['skills'][skill] = {
    'source': 'owner/repo', 'sourceType': 'github',
    'sourceUrl': 'https://github.com/owner/repo.git', 'ref': 'main',
    'skillPath': 'skills/' + skill + '/SKILL.md', 'skillFolderHash': 'd' * 40,
    'installedAt': '2026-01-01T00:00:00Z',
}
lock_path.parent.mkdir(parents=True, exist_ok=True)
lock_path.write_text(json.dumps(lock))
for agent in agents:
    base = {'claude-code': '.claude', 'codex': '.codex'}.get(agent)
    if base:
        mirror = home / base / 'skills' / skill
        mirror.mkdir(parents=True, exist_ok=True)
        (mirror / 'SKILL.md').write_text('---\\nname: ' + skill + '\\n---\\n# installed\\n')
''')
    npx.chmod(0o755)
    config = home / 'config.toml'
    config.write_text('')
    env = dict(
        os.environ, HOME=str(home), PATH=f'{bin_dir}:{os.environ["PATH"]}',
        XDG_STATE_HOME=str(home / 'state'), DEVTRAY_CONFIG=str(config),
    )

    plan = subprocess.run(
        [BINARY, 'skills', 'install', 'owner/repo', '--skill', 'alpha',
         '--agent', 'claude-code', '--dry-run', '--json'],
        env=env, capture_output=True, text=True, check=True)
    planned = json.loads(plan.stdout)
    assert planned['skill'] == 'alpha' and planned['agents'] == ['claude-code'], planned
    assert 'beta' in planned['candidates'], planned
    assert '-a claude-code' in planned['command'], planned

    installed = subprocess.run(
        [BINARY, 'skills', 'install', 'owner/repo', '--skill', 'alpha',
         '--agent', 'claude-code', '--yes', '--json'],
        env=env, capture_output=True, text=True, check=True)
    assert '安装 alpha 完成' in json.loads(installed.stdout)['message']
    assert (home / '.agents' / 'skills' / 'alpha' / 'SKILL.md').is_file()
    assert 'alpha' in json.loads((home / '.agents' / '.skill-lock.json').read_text())['skills']
    assert (home / '.claude' / 'skills' / 'alpha' / 'SKILL.md').is_file()

    again = subprocess.run(
        [BINARY, 'skills', 'install', 'owner/repo', '--skill', 'alpha',
         '--agent', 'claude-code', '--yes'],
        env=env, capture_output=True, text=True)
    assert again.returncode != 0 and '已经存在' in again.stderr, again.stderr

print('PASS: install plan, candidate selection, explicit agents, install verification, existing refusal')
