"""清理门禁的端到端回归：临时项目 + 真实二进制，不触碰用户目录。

覆盖：扫描 JSON、dry-run 不删、--yes 删除 + 审计 + job、符号链接拒绝、
git 跟踪拒绝、ops 锁互斥、_clean 任务执行。
"""
import fcntl
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get('DEVTRAY_TEST_BINARY', str(ROOT / 'target/debug/devtray'))


def env_for(tmp: Path, config: Path) -> dict:
    return dict(os.environ, DEVTRAY_CONFIG=str(config), XDG_STATE_HOME=str(tmp / 'state'))


def run(args, env, input_text=None):
    return subprocess.run(
        [BINARY, *args], env=env, input=input_text,
        capture_output=True, text=True, timeout=30,
    )


with tempfile.TemporaryDirectory(prefix='devtray-disk-clean-') as tmpdir:
    tmp = Path(tmpdir)
    project = tmp / 'proj'
    app = project / 'app'
    (app / 'node_modules' / 'pkg').mkdir(parents=True)
    (app / 'package.json').write_text('{}')
    (app / 'bun.lock').write_text('')
    (app / 'node_modules' / 'pkg' / 'blob').write_bytes(b'x' * 8192)
    config = tmp / 'config.toml'
    config.write_text(
        f'[settings]\nclean_extra_dirs = []\nmcp_writes = "allow"\n\n'
        f'[[project]]\nname = "tmp"\nroot = "{project}"\n\n'
        f'[[project.service]]\nname = "frontend"\ncwd = "{app}"\ncmd = "true"\n'
    )
    env = env_for(tmp, config)

    scan = run(['disk', 'tmp', '--json'], env)
    assert scan.returncode == 0, scan.stderr
    usage = json.loads(scan.stdout)[0]
    artifact = next(a for a in usage['artifacts'] if a['name'] == 'node_modules')
    assert artifact['deletable'] is True, artifact
    assert usage['complete'] is True

    dry = run(['disk', 'clean', 'tmp', '--kind', 'node_modules', '--dry-run'], env)
    assert dry.returncode == 0, dry.stderr
    assert '将清理' in dry.stdout and (app / 'node_modules').is_dir()

    clean = run(['disk', 'clean', 'tmp', '--kind', 'node_modules', '--yes'], env)
    assert clean.returncode == 0, clean.stderr
    assert not (app / 'node_modules').exists()
    audit = tmp / 'state' / 'devtray' / 'audit.jsonl'
    assert audit.is_file()
    entry = json.loads(audit.read_text().strip().splitlines()[-1])
    assert entry['action'] == 'clean' and entry['result'] == 'ok'
    jobs = list((tmp / 'state' / 'devtray' / 'jobs').glob('*.json'))
    assert jobs and json.loads(jobs[0].read_text())['state'] == 'done'

    # 符号链接：扫描不会把它当依赖目录，清理命令必须找不到可删目标
    outside = tmp / 'outside'
    outside.mkdir()
    (outside / 'keep').write_text('keep')
    (app / 'node_modules').symlink_to(outside, target_is_directory=True)
    linked = run(['disk', 'clean', 'tmp', '--kind', 'node_modules', '--yes'], env)
    assert linked.returncode != 0
    assert '没有找到' in linked.stderr, linked.stderr
    assert (outside / 'keep').exists()
    (app / 'node_modules').unlink()

    # git 跟踪的 target：候选存在但不可删
    (project / 'target' / 'debug').mkdir(parents=True)
    (project / 'Cargo.toml').write_text('[package]\nname="x"\nversion="0.1.0"\n')
    (project / 'target' / 'debug' / 'tracked.bin').write_bytes(b'y' * 4096)
    subprocess.run(['git', '-C', str(project), 'init', '-q'], check=True)
    subprocess.run(['git', '-C', str(project), 'add', 'target/debug/tracked.bin'], check=True)
    tracked = run(['disk', 'clean', 'tmp', '--kind', 'target', '--yes'], env)
    assert tracked.returncode != 0
    assert '不满足清理条件' in tracked.stderr, tracked.stderr
    assert (project / 'target').is_dir()

    # ops 锁互斥：拿住锁时清理必须失败，且目录保持原样
    (app / 'node_modules' / 'pkg').mkdir(parents=True)
    state = tmp / 'state' / 'devtray'
    state.mkdir(parents=True, exist_ok=True)
    with open(state / 'ops.lock', 'w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        blocked = run(['disk', 'clean', 'tmp', '--kind', 'node_modules', '--yes'], env)
    assert blocked.returncode != 0
    assert '正在进行' in blocked.stderr, blocked.stderr
    assert (app / 'node_modules').is_dir()

    # _clean 任务入口：按 job 记录执行，完成后状态为 done
    job_id = 'e2e-job'
    job = {
        'id': job_id, 'action': 'clean', 'state': 'pending_approval', 'source': 'panel',
        'project': 'tmp', 'paths': [str(app / 'node_modules')], 'created_at': 1,
        'started_at': None, 'finished_at': None, 'progress': 0.0,
        'message': None, 'error': None, 'result': None,
    }
    (state / 'jobs').mkdir(parents=True, exist_ok=True)
    (state / 'jobs' / f'{job_id}.json').write_text(json.dumps(job))
    done = run(['_clean', job_id], env)
    assert done.returncode == 0, done.stderr
    assert not (app / 'node_modules').exists()
    final = json.loads((state / 'jobs' / f'{job_id}.json').read_text())
    assert final['state'] == 'done', final

    # _approve + mcp_writes=allow：跳过弹窗，后台执行并完成
    (app / 'node_modules' / 'pkg').mkdir(parents=True)
    allow_id = 'approve-allow'
    allow_job = {
        'id': allow_id, 'action': 'clean', 'state': 'pending_approval', 'source': 'mcp',
        'project': 'tmp', 'paths': [str(app / 'node_modules')], 'total': 0,
        'created_at': 1, 'started_at': None, 'finished_at': None, 'progress': 0.0,
        'message': None, 'error': None, 'result': None,
    }
    (state / 'jobs' / f'{allow_id}.json').write_text(json.dumps(allow_job))
    approved = run(['_approve', allow_id], env)
    assert approved.returncode == 0, approved.stderr
    deadline = time.time() + 10
    loaded = allow_job
    while time.time() < deadline:
        loaded = json.loads((state / 'jobs' / f'{allow_id}.json').read_text())
        if loaded['state'] == 'done':
            break
        time.sleep(0.1)
    assert loaded['state'] == 'done', loaded
    assert not (app / 'node_modules').exists()

    # mcp_writes=deny：审批进程必须拒绝且不删除
    (app / 'node_modules' / 'pkg').mkdir(parents=True)
    config.write_text(config.read_text().replace('mcp_writes = "allow"', 'mcp_writes = "deny"'))
    deny_id = 'approve-deny'
    deny_job = dict(allow_job, id=deny_id, state='pending_approval')
    deny_job['paths'] = [str(app / 'node_modules')]
    (state / 'jobs' / f'{deny_id}.json').write_text(json.dumps(deny_job))
    denied = run(['_approve', deny_id], env)
    assert denied.returncode == 0, denied.stderr
    after = json.loads((state / 'jobs' / f'{deny_id}.json').read_text())
    assert after['state'] == 'cancelled', after
    assert 'deny' in (after.get('error') or '')
    assert (app / 'node_modules').is_dir()
    last = json.loads(audit.read_text().strip().splitlines()[-1])
    assert last['result'] == 'denied', last

print('PASS: scan/dry-run, clean, audit/job, symlink, git, ops lock, _clean, approve allow/deny')
