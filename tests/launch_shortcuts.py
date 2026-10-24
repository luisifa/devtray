"""Real CLI -> terminal -> tmux attach and browser dispatch, using an isolated tmux server."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = str(ROOT/'target/debug/devtray')
with tempfile.TemporaryDirectory(prefix='devtray-shortcuts-') as tmp:
    root = Path(tmp)
    bin_dir = root/'bin'
    bin_dir.mkdir()
    socket = f'devtray-shortcut-test-{os.getpid()}'
    (bin_dir/'tmux').write_text(f'#!/bin/sh\nexec /usr/bin/tmux -L {socket} -f /dev/null "$@"\n')
    # A terminal emulator test double creates an actual controlling PTY and
    # executes the exact command passed by devtray, including tmux attach.
    (bin_dir/'test-terminal').write_text('''#!/usr/bin/python3
import fcntl, json, os, pty, select, signal, struct, sys, termios, time
from pathlib import Path
root = Path(os.environ['SHORTCUT_TEST_ROOT'])
assert sys.argv[1] == '-e', sys.argv
args = sys.argv[2:]
(root/'terminal-args.json').write_text(json.dumps(args))
pid, fd = pty.fork()
if pid == 0:
    os.environ['TERM'] = 'xterm-256color'
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
    os.execvp(args[0], args)
raw = bytearray()
try:
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if select.select([fd], [], [], .1)[0]:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            raw.extend(chunk)
            if b'raw-terminal-marker' in raw or (b'Pane is dead' in raw and b'Local: http://localhost:5173/' in raw):
                break
finally:
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    os.waitpid(pid, 0)
    os.close(fd)
(root/'terminal-screen.bin').write_bytes(raw)
''')
    (bin_dir/'xdg-open').write_text('''#!/usr/bin/python3
import json, os, sys
from pathlib import Path
(Path(os.environ['SHORTCUT_TEST_ROOT'])/'browser-args.json').write_text(json.dumps(sys.argv[1:]))
''')
    for path in bin_dir.iterdir():
        path.chmod(0o755)
    config = root/'config.toml'
    config.write_text(f'''[settings]
terminal = "test-terminal"
[[project]]
name = "shortcut"
[[project.service]]
name = "frontend"
cwd = "{tmp}"
cmd = "printf 'INFO raw-terminal-marker\\nLocal: http://localhost:5173/\\n'; while [ ! -f finished ]; do sleep 0.05; done"
''')
    env = dict(os.environ, DEVTRAY_CONFIG=str(config), XDG_STATE_HOME=tmp,
               SHORTCUT_TEST_ROOT=tmp, PATH=f'{bin_dir}:'+os.environ['PATH'])
    env.pop('TMUX', None)

    def run(*args, check=True):
        return subprocess.run([BINARY,*args], env=env, capture_output=True, text=True, timeout=8, check=check)

    def wait_file(path):
        for _ in range(160):
            if path.exists():
                return path.read_bytes()
            time.sleep(.05)
        raise AssertionError(f'{path.name} not created')

    def pane_pid():
        return subprocess.check_output(['tmux','display-message','-p','-t','devtray-shortcut-frontend','#{pane_pid}'], env=env)

    try:
        assert run('terminal','frontend',check=False).returncode != 0
        run('start','frontend')
        original_pid = pane_pid()
        for _ in range(60):
            if 'raw-terminal-marker' in run('logs','frontend').stdout:
                break
            time.sleep(.05)
        run('terminal','frontend')
        assert json.loads(wait_file(root/'terminal-args.json')) == ['tmux','attach','-t','devtray-shortcut-frontend']
        assert b'raw-terminal-marker' in wait_file(root/'terminal-screen.bin'), 'attach did not show original pane output'
        assert pane_pid() == original_pid, 'Viewing terminal restarted the service'
        run('open','frontend')
        assert json.loads(wait_file(root/'browser-args.json')) == ['http://localhost:5173/']
        assert 'raw-terminal-marker' in run('logs','frontend','--level','info').stdout
        (root/'finished').touch()
        for _ in range(100):
            if json.loads(run('status','--json').stdout)['services'][0]['state'] == 'exited_ok':
                break
            time.sleep(.05)
        else:
            raise AssertionError('Service did not exit naturally into a retained pane')
        (root/'terminal-screen.bin').unlink()
        run('terminal','frontend')
        screen = wait_file(root/'terminal-screen.bin')
        assert b'Local: http://localhost:5173/' in screen and b'Pane is dead' in screen, repr(screen)
        history = subprocess.check_output(['tmux','capture-pane','-p','-S','-','-t','devtray-shortcut-frontend'], env=env)
        assert b'raw-terminal-marker' in history, 'Resizing the terminal lost the retained scrollback'
        print('PASS: real tmux attach shows original output, preserves service PID, exited pane access, browser URL discovery, classified logs')
    finally:
        subprocess.run(['tmux','kill-server'], env=env, capture_output=True)
