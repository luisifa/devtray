"""Real native executable fixtures; no WSL, shell, config or developer tools on probe PATH."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = str(Path(os.environ.get('DEVTRAY_TEST_BINARY', ROOT / 'target/debug/devtray')).resolve())
SUFFIX = '.exe' if os.name == 'nt' else ''

with tempfile.TemporaryDirectory(prefix='devtray-native-') as tmp:
    base = Path(tmp)
    bins = base / '工具 with spaces'
    bins.mkdir()
    source = base / 'fixture.rs'
    source.write_text(r'''
use std::{env, path::Path, time::Duration};
fn main() {
    let exe = env::args().next().unwrap();
    let name = Path::new(&exe).file_stem().unwrap().to_str().unwrap();
    match name {
        "git" => println!("git version 8.7.6"),
        "rustc" => eprintln!("rustc 7.6.5"),
        "cargo" => std::process::exit(7),
        "node" => println!("not a version"),
        "bun" => std::thread::sleep(Duration::from_secs(30)),
        "claude" => println!("9.9.9 (Claude Code)"),
        "opencode" => { use std::io::Write; let _ = std::io::stdout().write_all(&vec![b'x'; 100_000]); }
        "wsl" => { std::fs::write(env::var("WSL_SENTINEL").unwrap(), "called").unwrap(); std::process::exit(99); }
        _ => std::process::exit(2),
    }
}
''', encoding='utf-8')
    fixture = base / ('fixture' + SUFFIX)
    subprocess.run(['rustc', '--edition=2024', str(source), '-o', str(fixture)], check=True, timeout=60)
    for name in ['git', 'rustc', 'cargo', 'node', 'bun', 'claude', 'opencode', 'wsl']:
        shutil.copy2(fixture, bins / (name + SUFFIX))
    # Also catch direct calls to wsl.exe on Unix.
    if not SUFFIX:
        shutil.copy2(fixture, bins / 'wsl.exe')
    if os.name == 'nt':
        (bins / 'codex').write_text('# Unix npm script', encoding='utf-8')
        (bins / 'codex.cmd').write_text('@echo off', encoding='utf-8')
    marker = base / 'wsl-called'
    env = dict(os.environ, PATH=str(bins), HOME=str(base), USERPROFILE=str(base),
               APPDATA=str(base / 'roaming'), LOCALAPPDATA=str(base / 'local'),
               DEVTRAY_CONFIG=str(base / 'missing.toml'), WSL_SENTINEL=str(marker),
               PATHEXT='.COM;.EXE;.BAT;.CMD', NPM_CONFIG_PREFIX='', npm_config_prefix='',
               VOLTA_HOME='', BUN_INSTALL='', PNPM_HOME='')
    def run(*args, input=None):
        return subprocess.run([BINARY, *args], env=env, input=input,
                              capture_output=True, text=True, check=True, timeout=15)

    inventory = json.loads(run('inventory', '--json').stdout)
    tools = {t['id']: t for t in inventory['tools']}
    assert inventory['environment_id'] == 'local'
    assert tools['git']['version'] == '8.7.6'
    assert Path(tools['git']['path']).samefile(bins / ('git' + SUFFIX))
    assert tools['git']['source'] == 'path'
    assert tools['rust']['version'] == '7.6.5', tools['rust']
    for tool in ['cargo', 'node', 'bun', 'opencode']:
        assert tools[tool]['status'] == 'probe_failed', tools[tool]
        assert tools[tool]['path'] and tools[tool]['reason'], tools[tool]
    assert '超时' in tools['bun']['reason']
    assert '64 KiB' in tools['opencode']['reason']
    assert tools['claude-code']['version'] == '9.9.9'
    if os.name == 'nt':
        assert Path(tools['codex']['path']).samefile(bins / 'codex.cmd')
        assert tools['codex']['status'] == 'probe_failed'
        assert '包装脚本' in tools['codex']['reason']
    assert not marker.exists(), 'native inventory invoked WSL'
    response = json.loads(run('mcp', input=json.dumps({
        'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
        'params': {'name': 'devtray_inventory', 'arguments': {}}
    }) + chr(10)).stdout)
    assert not response.get('isError'), response
    assert json.loads(response['result']['content'][0]['text'])['environment_id'] == 'local'
    assert not marker.exists()
    assert not (base / 'missing.toml').exists(), 'inventory generated service config'
print('PASS: native paths, stderr versions, failure/timeout/output limits, MCP without config, no WSL')
