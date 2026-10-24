"""Real tmux regression, isolated socket and configuration; never touches user services."""
import json, os, pathlib, subprocess, tempfile, time
BINARY = str(pathlib.Path('target/debug/devtray').resolve())
with tempfile.TemporaryDirectory(prefix='devtray-regression-') as tmp:
    root = pathlib.Path(tmp)
    (root/'bin').mkdir()
    (root/'bin/tmux').write_text(f'#!/bin/sh\nexec /usr/bin/tmux -L devtray-test-{os.getpid()} -f /dev/null "$@"\n')
    (root/'bin/tmux').chmod(0o755)
    cfg = root/'config.toml'
    cfg.write_text(f'[[project]]\nname="regression"\n[[project.service]]\nname="logger"\ncwd="{tmp}"\ncmd="printf \'INFO first-run\\nWARN keep-me\\nERROR failure\\n\'; sleep 30"\n')
    env = dict(os.environ, DEVTRAY_CONFIG=str(cfg), XDG_STATE_HOME=tmp, PATH=f'{root}/bin:'+os.environ['PATH'])
    def run(*args):
        return subprocess.run([BINARY,*args],env=env,capture_output=True,text=True,check=True).stdout
    try:
        run('start','logger')
        log = root/'devtray/logs/regression-logger.log'
        for _ in range(100):
            if log.exists() and 'keep-me' in log.read_text(): break
            time.sleep(.05)
        assert 'keep-me' in log.read_text(), 'first output was lost'
        cfg.write_text(cfg.read_text().replace('first-run','second-run').replace('keep-me','new-warning'))
        run('restart','logger')
        for _ in range(100):
            if 'second-run' in log.read_text(): break
            time.sleep(.05)
        files = [f for f in log.parent.glob('regression-logger.log*') if f.suffix != '.lock']
        assert any('keep-me' in f.read_text() for f in files), 'restart erased previous logs'
        data = json.loads(run('logs','logger','--history','--level','warn','--json'))
        assert any('keep-me' in e['text'] for e in data['entries'])
        assert all(e['level']=='warn' for e in data['entries'])
        run('stop','logger')
        assert 'keep-me' in run('logs','logger','--history','--level','warn')
        print('PASS: first output, restart archives, level filtering, logs after stop')
    finally:
        subprocess.run(['tmux','kill-server'],env=env,capture_output=True)
