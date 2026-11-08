"""Start the actual Qt panel and exercise its single-instance IPC on any OS."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from PySide6.QtCore import QCoreApplication
from PySide6.QtNetwork import QLocalSocket

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'ui'))
from panel import instance_identity

app = QCoreApplication([])
with tempfile.TemporaryDirectory(prefix='devtray-panel-') as tmp:
    config = Path(tmp) / 'config.toml'
    config.write_text('[settings]\n', encoding='utf-8')
    name = 'devtray-panel-' + hashlib.sha256(
        (instance_identity() + ':' + str(config.resolve())).encode()
    ).hexdigest()[:24]
    env = dict(os.environ, QT_QPA_PLATFORM='offscreen', XDG_CONFIG_HOME=tmp)
    cmd = [sys.executable, str(ROOT / 'ui' / 'panel.py'), sys.executable, str(config)]
    log = Path(tmp) / 'panel.log'
    with log.open('w+b') as output:
        first = subprocess.Popen(cmd, env=env, stdout=output, stderr=output)

        def request(payload):
            sock = QLocalSocket()
            sock.connectToServer(name)
            if not sock.waitForConnected(100):
                return None
            sock.write(json.dumps(payload).encode() + b'\n')
            sock.flush()
            if not sock.waitForReadyRead(2000):
                raise AssertionError('panel did not acknowledge request')
            data = json.loads(bytes(sock.readAll()))
            sock.disconnectFromServer()
            return data

        try:
            for _ in range(100):
                if first.poll() is not None:
                    raise AssertionError('panel exited during startup')
                if request({'inspect': True}) is not None:
                    break
                time.sleep(.05)
            else:
                raise AssertionError('panel IPC did not start')
            second = subprocess.run(cmd + ['--mode', 'full'], env=env,
                                    stdout=output, stderr=output, timeout=10)
            assert second.returncode == 0
            result = request({'inspect': True})
            assert result is not None and result['mode'] == 'full', result
            assert request({'quit': True}) is not None
            assert first.wait(timeout=10) == 0
            print('PASS: panel initialization and second-launch IPC')
        except Exception:
            output.flush()
            output.seek(0)
            print(output.read().decode('utf-8', errors='replace'), file=sys.stderr)
            raise
        finally:
            if first.poll() is None:
                first.kill()
                first.wait(timeout=5)
