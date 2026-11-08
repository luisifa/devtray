"""Check the public stdio MCP contract using temporary config/logs, without starting services."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get('DEVTRAY_TEST_BINARY', str(ROOT / 'target/debug/devtray'))

TOOLS = {
    'devtray_inventory',
    'devtray_status', 'devtray_logs', 'devtray_start', 'devtray_stop',
    'devtray_restart', 'devtray_open_preview', 'devtray_disk', 'devtray_job',
    'devtray_clean', 'devtray_git_status', 'devtray_git_diff', 'devtray_git_commit',
    'devtray_git_push', 'devtray_git_pull',
}


def call(i, name, arguments):
    return {'jsonrpc': '2.0', 'id': i, 'method': 'tools/call',
            'params': {'name': name, 'arguments': arguments}}


def mcp(config, messages, env=None, timeout=20):
    payload = ''.join(json.dumps(m) + '\n' for m in messages)
    return subprocess.run(
        [BINARY, 'mcp'], input=payload, capture_output=True, text=True, check=True,
        timeout=timeout,
        env=dict(os.environ, DEVTRAY_CONFIG=str(config),
                 XDG_STATE_HOME=str(Path(config).parent / 'state'), **(env or {})),
    )


def results(run):
    return {r['id']: r['result'] for r in map(json.loads, run.stdout.splitlines())}


def body(result):
    return json.loads(result['content'][0]['text'])


def git(repo, *args):
    return subprocess.run(
        ['git', '-C', str(repo), *args], check=True, capture_output=True, text=True,
        env=dict(os.environ, GIT_TERMINAL_PROMPT='0', GIT_AUTHOR_NAME='tester',
                 GIT_AUTHOR_EMAIL='tester@example.com', GIT_COMMITTER_NAME='tester',
                 GIT_COMMITTER_EMAIL='tester@example.com'),
    ).stdout


with tempfile.TemporaryDirectory(prefix='devtray-mcp-') as tmp:
    directory = Path(tmp)
    repo = directory / 'repo'
    repo.mkdir()
    git(repo, 'init', '-q')
    (repo / 'a.txt').write_text('hello\n')
    git(repo, 'add', 'a.txt')
    git(repo, 'commit', '-qm', 'init')
    (repo / 'a.txt').write_text('hello\nworld\n')
    (repo / 'b.txt').write_text('new\n')
    branch = git(repo, 'branch', '--show-current').strip()

    log = directory / 'service.log'
    log.write_text(
        'INFO started\nWARN retry\nERROR connection failed\n'
        'error[E0308]: mismatched types\n  --> src/app.rs:12:5\n')

    def config(name, writes):
        path = directory / name
        path.write_text(
            f'[settings]\nmcp_writes = "{writes}"\n\n'
            f'[[project]]\nname="mcp-smoke"\nroot="{repo}"\n[[project.service]]\n'
            f'name="backend"\ncwd="{tmp}"\ncmd="true"\nlog_file="{log}"\n'
        )
        return path

    deny_config = config('deny.toml', 'deny')
    messages = [
        {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {
            'protocolVersion': '2025-06-18', 'capabilities': {},
            'clientInfo': {'name': 'devtray-release-test', 'version': '1.0'}}},
        {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
        {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'},
        {'jsonrpc': '2.0', 'id': 3, 'method': 'tools/call', 'params': {
            'name': 'devtray_status', 'arguments': {}}},
        {'jsonrpc': '2.0', 'id': 4, 'method': 'tools/call', 'params': {
            'name': 'devtray_logs', 'arguments': {
                'target': 'mcp-smoke/backend', 'level': 'error', 'search': 'CONNECTION'}}},
        {'jsonrpc': '2.0', 'id': 5, 'method': 'tools/call', 'params': {
            'name': 'devtray_logs', 'arguments': {'target': 'missing'}}},
        {'jsonrpc': '2.0', 'id': 6, 'method': 'tools/call', 'params': {
            'name': 'devtray_disk', 'arguments': {}}},
        {'jsonrpc': '2.0', 'id': 7, 'method': 'tools/call', 'params': {
            'name': 'devtray_job', 'arguments': {'job_id': 'missing'}}},
        {'jsonrpc': '2.0', 'id': 8, 'method': 'tools/call', 'params': {
            'name': 'devtray_clean', 'arguments': {
                'project': 'mcp-smoke', 'kinds': ['node_modules']}}},
        {'jsonrpc': '2.0', 'id': 9, 'method': 'ping'},
        call(10, 'devtray_git_status', {}),
        call(11, 'devtray_git_diff', {'project': 'mcp-smoke', 'path': 'a.txt'}),
        call(12, 'devtray_git_commit', {
            'project': 'mcp-smoke', 'message': 'test: deny', 'all': True}),
        call(13, 'devtray_git_pull', {'project': 'mcp-smoke'}),
        call(14, 'devtray_git_push', {'project': 'mcp-smoke'}),
        call(15, 'devtray_git_status', {'project': 'missing'}),
        call(16, 'devtray_git_commit', {'project': 'mcp-smoke', 'message': 'test: no paths'}),
        call(17, 'devtray_logs', {'target': 'mcp-smoke/backend', 'digest': True}),
    ]
    run = mcp(deny_config, messages)
    responses = [json.loads(line) for line in run.stdout.splitlines()]
    assert [r['id'] for r in responses] == list(range(1, 18)), responses
    assert all(r['jsonrpc'] == '2.0' for r in responses)
    got = results(run)
    initialize, tools = got[1], got[2]
    version = subprocess.check_output([BINARY, '--version'], text=True).strip().split()[-1]
    assert initialize['serverInfo'] == {'name': 'devtray', 'version': version}
    assert initialize['protocolVersion'] == '2025-06-18'
    assert {t['name'] for t in tools['tools']} == TOOLS

    services = body(got[3])['services']
    assert len(services) == 1 and services[0]['name'] == 'mcp-smoke/backend'
    entries = body(got[4])['entries']
    assert len(entries) == 1 and entries[0]['level'] == 'error', entries
    assert 'connection failed' in entries[0]['text']
    assert got[5]['isError'] is True
    disk_body = body(got[6])
    assert disk_body['projects'] == [] and disk_body['scanned'] is False
    assert '资源页' in disk_body['hint']
    assert got[7]['isError'] is True
    assert got[8]['isError'] is True
    assert '禁用' in got[8]['content'][0]['text']
    assert got[9] == {}

    status = body(got[10])['repos']
    assert len(status) == 1 and status[0]['is_repo'] is True
    assert status[0]['branch'] == branch and status[0]['dirty'] is True, status
    changes = {c['path']: c for c in status[0]['changes']}
    assert changes['a.txt']['unstaged'] and changes['b.txt']['untracked'], changes

    diff = body(got[11])
    assert diff['path'] == 'a.txt' and '+world' in diff['diff'], diff

    assert got[12]['isError'] is True and '禁用' in got[12]['content'][0]['text']
    assert got[13]['isError'] is True and '禁用' in got[13]['content'][0]['text']
    assert got[14]['isError'] is True and '禁用' in got[14]['content'][0]['text']
    assert got[15]['isError'] is True and '找不到' in got[15]['content'][0]['text']
    assert got[16]['isError'] is True and 'paths' in got[16]['content'][0]['text']

    # digest：压缩分诊保留错误码、位置与次数，不整段回吐日志
    digest = got[17]['content'][0]['text']
    assert got[17].get('isError') is not True, digest
    assert '扫描' in digest and 'E0308' in digest and 'src/app.rs:12:5' in digest, digest
    assert 'INFO started' not in digest, digest

    # allow：直接执行并返回提交结果
    allow = mcp(config('allow.toml', 'allow'), [
        call(1, 'devtray_git_commit', {
            'project': 'mcp-smoke', 'message': 'test: add b', 'paths': ['a.txt', 'b.txt']}),
        call(2, 'devtray_git_status', {}),
        call(3, 'devtray_git_commit', {
            'project': 'mcp-smoke', 'message': 'test: empty', 'paths': ['a.txt']}),
        call(4, 'devtray_git_diff', {'project': 'mcp-smoke', 'path': 'a.txt'}),
    ])
    allowed = results(allow)
    committed = body(allowed[1])
    assert committed['files'] == 2 and committed['subject'] == 'test: add b', committed
    assert committed['hash'] and committed['hash'] in git(repo, 'log', '-1', '--pretty=%H')
    assert body(allowed[2])['repos'][0]['dirty'] is False
    assert allowed[3]['isError'] is True and '不在变更列表' in allowed[3]['content'][0]['text']
    assert allowed[4]['isError'] is True and '变更列表' in allowed[4]['content'][0]['text']
    assert git(repo, 'log', '-1', '--pretty=%s').strip() == 'test: add b'

    # ask：创建 job，等用户桌面确认；yad 不可用时 fail closed
    # 只暴露 git、不暴露 yad 的 PATH：审批进程必须 fail closed，不能弹窗也不能执行
    shim = directory / 'shim-bin'
    shim.mkdir()
    (shim / 'git').symlink_to(shutil.which('git'))
    ask = mcp(config('ask.toml', 'ask'), [
        call(1, 'devtray_git_commit', {
            'project': 'mcp-smoke', 'message': 'test: ask', 'all': True}),
    ], env={'PATH': str(shim)})
    job = body(results(ask)[1])
    assert job['state'] == 'pending_approval' and job['job_id'], job
    state = None
    for _ in range(100):
        polled = json.loads(subprocess.run(
            [BINARY, '_job', job['job_id']], capture_output=True, text=True, check=True,
            env=dict(os.environ, DEVTRAY_CONFIG=str(deny_config),
                     XDG_STATE_HOME=str(directory / 'state')),
        ).stdout)
        state = polled['state']
        if state != 'pending_approval':
            break
        time.sleep(0.05)
    assert state == 'cancelled', polled
    assert 'yad' in (polled['error'] or ''), polled

print('PASS: MCP handshake, fifteen tools, status, logs, disk cache, job/clean denial, '
      'git read/write with allow/ask/deny')
