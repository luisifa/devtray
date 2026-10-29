"""Skills 写门禁：假 npx + 临时 HOME，验证快照、漂移拒绝、升级与删除。"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get('DEVTRAY_TEST_BINARY', str(ROOT / 'target/debug/devtray'))

with tempfile.TemporaryDirectory(prefix='devtray-skillswrite-') as tmpdir:
    home = Path(tmpdir)
    skill = home / '.agents' / 'skills' / 'alpha'
    skill.mkdir(parents=True)
    (skill / 'SKILL.md').write_text('---\nname: alpha\n---\n# old\n')
    mirror = home / '.claude' / 'skills' / 'alpha'
    mirror.mkdir(parents=True)
    (mirror / 'SKILL.md').write_text('---\nname: alpha\n---\n# old\n')
    lock = home / '.agents' / '.skill-lock.json'
    lock.write_text(json.dumps({
        'version': 3,
        'skills': {'alpha': {
            'source': 'owner/repo', 'sourceType': 'github',
            'sourceUrl': 'https://github.com/owner/repo.git', 'ref': 'main',
            'skillPath': 'skills/alpha/SKILL.md',
            'skillFolderHash': 'a' * 40,
            'installedAt': '2026-01-01T00:00:00Z',
        }},
    }))
    fake = home / 'bin' / 'npx'
    fake.parent.mkdir()
    fake.write_text('''#!/usr/bin/env python3
import json, os, shutil, sys
from pathlib import Path
args = sys.argv[1:]
action = 'update' if 'update' in args else 'remove'
name = args[args.index(action) + 1]
home = Path(os.environ['HOME'])
skill = home / '.agents' / 'skills' / name
lock_path = home / '.agents' / '.skill-lock.json'
lock = json.loads(lock_path.read_text())
if action == 'update':
    (skill / 'SKILL.md').write_text('---\\nname: ' + name + '\\n---\\n# updated\\n')
    lock['skills'][name]['skillFolderHash'] = 'c' * 40
else:
    shutil.rmtree(skill)
    for p in [home / '.claude' / 'skills' / name, home / '.codex' / 'skills' / name]:
        shutil.rmtree(p, ignore_errors=True)
    lock['skills'].pop(name, None)
lock_path.write_text(json.dumps(lock))
''')
    fake.chmod(0o755)
    config = home / 'config.toml'
    config.write_text('')
    env = dict(
        os.environ, HOME=str(home), PATH=f'{fake.parent}:{os.environ["PATH"]}',
        XDG_STATE_HOME=str(home / 'state'), DEVTRAY_CONFIG=str(config),
    )

    # 无漂移：plan 正常，dry-run 不执行
    plan = subprocess.run([BINARY, 'skills', 'upgrade', 'alpha', '--dry-run', '--json'],
                          env=env, capture_output=True, text=True)
    if plan.returncode != 0:
        raise SystemExit('plan failed: ' + plan.stderr)
    planned = json.loads(plan.stdout)
    assert planned['action'] == 'upgrade' and planned['drift'] is False
    assert (skill / 'SKILL.md').read_text().endswith('# old\n')

    # 制造漂移：默认拒绝，显式 --allow-drift 才放行
    (mirror / 'SKILL.md').write_text('---\nname: alpha\n---\n# drifted\n')
    refused = subprocess.run([BINARY, 'skills', 'upgrade', 'alpha', '--dry-run'],
                             env=env, capture_output=True, text=True)
    assert refused.returncode != 0 and '不一致' in refused.stderr, refused.stderr
    allowed = subprocess.run(
        [BINARY, 'skills', 'upgrade', 'alpha', '--dry-run', '--allow-drift', '--json'],
        env=env, capture_output=True, text=True, check=True)
    assert json.loads(allowed.stdout)['drift'] is True

    # 实际升级：快照 + 假 npx 执行 + 校验 lock
    upgrade = subprocess.run(
        [BINARY, 'skills', 'upgrade', 'alpha', '--yes', '--allow-drift', '--json'],
        env=env, capture_output=True, text=True)
    if upgrade.returncode != 0:
        logs = list((home / 'state' / 'devtray' / 'jobs').glob('*.log'))
        raise SystemExit('upgrade failed: ' + upgrade.stderr + (logs[0].read_text() if logs else ''))
    result = json.loads(upgrade.stdout)
    assert result['job_id']
    assert '# updated' in (skill / 'SKILL.md').read_text()
    lock_data = json.loads(lock.read_text())
    assert lock_data['skills']['alpha']['skillFolderHash'] == 'c' * 40
    backups = list((home / 'state' / 'devtray' / 'skills-backup' / 'alpha').iterdir())
    assert backups and (backups[0] / 'manifest.json').is_file()
    assert (backups[0] / 'canonical' / 'SKILL.md').is_file()
    assert (backups[0] / 'agents' / 'claude-code' / 'SKILL.md').is_file()

    # 面板端点：_skills-plan / _skills-start 走同一个后台 worker
    planned_again = subprocess.run(
        [BINARY, '_skills-plan', 'upgrade', 'alpha', '--allow-drift'],
        env=env, capture_output=True, text=True, check=True)
    assert json.loads(planned_again.stdout)['name'] == 'alpha'
    started = subprocess.run(
        [BINARY, '_skills-start', 'upgrade', 'alpha', '--allow-drift'],
        env=env, capture_output=True, text=True, check=True)
    job_id = json.loads(started.stdout)['job_id']
    deadline = time.time() + 10
    job = {}
    while time.time() < deadline:
        job = json.loads((home / 'state' / 'devtray' / 'jobs' / f'{job_id}.json').read_text())
        if job['state'] in ('done', 'error'):
            break
        time.sleep(0.05)
    assert job['state'] == 'done', job

    # 删除：假 npx 移除目录与 lock 条目
    removed = subprocess.run(
        [BINARY, 'skills', 'remove', 'alpha', '--yes', '--allow-drift', '--json'],
        env=env, capture_output=True, text=True, check=True)
    assert json.loads(removed.stdout)['job_id']
    assert not skill.exists()
    assert not mirror.exists()
    assert 'alpha' not in json.loads(lock.read_text())['skills']
    backups_dir = home / 'state' / 'devtray' / 'skills-backup' / 'alpha'
    assert len(list(backups_dir.iterdir())) == 3

    # 恢复快照：默认只恢复内容与副本，不改 lock
    listed = subprocess.run([BINARY, '_skills-backups', 'alpha'], env=env,
                            capture_output=True, text=True, check=True)
    snapshots = json.loads(listed.stdout)
    assert len(snapshots) >= 2 and snapshots[0]['has_lock'], snapshots
    latest = snapshots[0]['id']
    restored = subprocess.run(
        [BINARY, 'skills', 'restore', 'alpha', '--snapshot', latest, '--yes', '--json'],
        env=env, capture_output=True, text=True, check=True)
    assert '已恢复' in json.loads(restored.stdout)['message']
    assert (skill / 'SKILL.md').is_file()
    assert 'alpha' not in json.loads(lock.read_text())['skills']
    assert mirror.is_dir()

    # --restore-lock 写回该技能的单条 lock 记录
    restored_lock = subprocess.run(
        [BINARY, 'skills', 'restore', 'alpha', '--snapshot', latest,
         '--restore-lock', '--yes', '--json'],
        env=env, capture_output=True, text=True, check=True)
    assert '已恢复' in json.loads(restored_lock.stdout)['message']
    assert json.loads(lock.read_text())['skills']['alpha']['skillFolderHash'] == 'c' * 40

print('PASS: plan, drift refusal, snapshot, upgrade, remove, restore content/lock')
