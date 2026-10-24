"""Exercise embedded UI, mode persistence and the real single-instance protocol."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
from PySide6.QtCore import QCoreApplication
from PySide6.QtNetwork import QLocalSocket

ROOT = Path(__file__).resolve().parents[1]
app = QCoreApplication([])
with tempfile.TemporaryDirectory(prefix='devtray-instance-') as tmp:
    config = Path(tmp) / 'config.toml'
    config.write_text('[settings]\n')
    env = dict(os.environ, QT_QPA_PLATFORM='offscreen', XDG_CONFIG_HOME=tmp, DEVTRAY_CONFIG=str(config))
    name = 'devtray-panel-' + hashlib.sha256((str(os.getuid()) + ':' + str(config.resolve())).encode()).hexdigest()[:24]

    def connect():
        sock = QLocalSocket()
        sock.connectToServer(name)
        return sock if sock.waitForConnected(100) else None

    def send(payload):
        sock = connect()
        assert sock, 'panel socket unavailable'
        sock.write(json.dumps(payload).encode()+b'\n')
        sock.waitForBytesWritten(1000)
        assert sock.waitForReadyRead(2000), 'panel did not acknowledge'
        result = json.loads(bytes(sock.readAll()))
        sock.disconnectFromServer()
        return result

    def wait_server(expected=True):
        for _ in range(80):
            sock = connect()
            if sock:
                sock.disconnectFromServer()
            if bool(sock) == expected:
                return
            time.sleep(.05)
        raise AssertionError('panel socket did not ' + ('start' if expected else 'close'))

    def launch(*args):
        with (Path(tmp)/'launcher.log').open('ab') as output:
            command = subprocess.run([str(ROOT/'target/debug/devtray'), *args], env=env,
                                     stdout=output, stderr=output, timeout=5)
        assert command.returncode == 0
        wait_server()

    def state(small, full, preference):
        result = send({'inspect': True})
        assert (result['menu_visible'], result['visible'], result['mode']) == (small, full, preference), result

    try:
        launch('panel')
        state(True, False, 'small')  # New users start compact.
        send({'toggle': True})
        state(False, False, 'small')
        send({'toggle': True})
        state(True, False, 'small')
        send({'mode': 'full'})
        state(False, True, 'full')
        send({'quit': True})
        wait_server(False)
        launch('panel')
        state(False, True, 'full')  # Survives UI process restart.
        send({'mode': 'small'})
        state(True, False, 'small')
        send({'target': 'example/api'})
        state(False, True, 'small')  # A log shortcut does not change preference.
        send({'quit': True})
        wait_server(False)
        launch('panel')
        state(True, False, 'small')
        # A second launcher must address the existing UI, not create another one.
        launch('panel', '--mode', 'full')
        for _ in range(50):
            if send({'inspect': True})['mode'] == 'full':
                break
            time.sleep(.05)
        state(False, True, 'full')
        send({'quit': True})
        wait_server(False)
        launch('menu')  # Explicit compact-mode alias.
        state(True, False, 'small')
        print('PASS: compact/full mode persistence across process restarts, temporary log view, toggling, shared embedded UI')
    finally:
        if connect():
            send({'quit': True})
            wait_server(False)
