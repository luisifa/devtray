"""Check the public stdio MCP contract using temporary config/logs, without starting services."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get('DEVTRAY_TEST_BINARY', str(ROOT / 'target/debug/devtray'))
with tempfile.TemporaryDirectory(prefix='devtray-mcp-') as tmp:
    directory = Path(tmp)
    config = directory / 'config.toml'
    log = directory / 'service.log'
    log.write_text('INFO started\nWARN retry\nERROR connection failed\n')
    config.write_text(
        '[settings]\nmcp_writes = "deny"\n\n'
        f'[[project]]\nname="mcp-smoke"\n[[project.service]]\n'
        f'name="backend"\ncwd="{tmp}"\ncmd="true"\nlog_file="{log}"\n'
    )
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
    ]
    result = subprocess.run(
        [BINARY, 'mcp'], input=''.join(json.dumps(m) + '\n' for m in messages),
        env=dict(os.environ, DEVTRAY_CONFIG=str(config), XDG_STATE_HOME=tmp),
        capture_output=True, text=True, check=True, timeout=20,
    )
    responses = [json.loads(line) for line in result.stdout.splitlines()]
    assert [r['id'] for r in responses] == list(range(1, 10)), responses
    assert all(r['jsonrpc'] == '2.0' for r in responses)
    initialize, tools, status, logs, missing, disk, job_missing, clean_denied, ping = [
        r['result'] for r in responses
    ]
    version = subprocess.check_output([BINARY, '--version'], text=True).strip().split()[-1]
    assert initialize['serverInfo'] == {'name': 'devtray', 'version': version}
    assert initialize['protocolVersion'] == '2025-06-18'
    assert {t['name'] for t in tools['tools']} == {
        'devtray_status', 'devtray_logs', 'devtray_start', 'devtray_stop',
        'devtray_restart', 'devtray_open_preview', 'devtray_disk', 'devtray_job',
        'devtray_clean',
    }
    services = json.loads(status['content'][0]['text'])['services']
    assert len(services) == 1 and services[0]['name'] == 'mcp-smoke/backend'
    entries = json.loads(logs['content'][0]['text'])['entries']
    assert len(entries) == 1 and entries[0]['level'] == 'error', entries
    assert 'connection failed' in entries[0]['text']
    assert missing['isError'] is True
    disk_body = json.loads(disk['content'][0]['text'])
    assert disk_body['projects'] == [] and disk_body['scanned'] is False
    assert '资源页' in disk_body['hint']
    assert job_missing['isError'] is True
    assert clean_denied['isError'] is True
    assert '禁用' in clean_denied['content'][0]['text']
    assert ping == {}
    print('PASS: MCP handshake, nine tools, status, logs, disk cache, job/clean denial, ping')
