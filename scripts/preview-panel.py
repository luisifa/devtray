#!/usr/bin/env python3
"""Render the real Qt views with explicitly synthetic data, without contacting services."""
import os
from pathlib import Path
import sys
import tempfile
os.environ['QT_QPA_PLATFORM'] = 'offscreen'
with tempfile.TemporaryDirectory(prefix='devtray-preview-') as config:
    os.environ['XDG_CONFIG_HOME'] = config
    root = Path(__file__).resolve().parents[1]
    sys.path.insert(0, str(root / 'ui'))
    from panel import QApplication, Panel, ServiceDialog
    from PySide6.QtTest import QTest
    app = QApplication([])
    panel = Panel(str(root / 'target/debug/devtray'), '/tmp/devtray-preview.toml', poll=False)
    panel.request = lambda *a, **k: None
    services = []
    for project, name, state, command, port in [
        ('atelier', 'api', 'ready', 'cargo run', 3060),
        ('atelier', 'web', 'ready', 'bun run dev', 5173),
        ('notes', 'worker', 'starting', 'python worker.py', None),
        ('notes', 'database', 'not_started', 'docker compose up postgres', 5432),
        ('devtray', 'preview', 'exited_failed', 'bun run preview', 4173),
    ]:
        services.append(dict(project=project, service=name, name=f'{project}/{name}', state=state,
                             cwd=f'~/Workspace/{project}/{name}', cmd=command, port=port, ready=None,
                             autostart=False, log=f'~/.local/state/devtray/logs/{project}-{name}.log',
                             url=f'http://localhost:{port}' if port else None, session=f'devtray-{project}-{name}'))
    panel.apply_data({'services': services, 'config': '~/.config/devtray/config.toml', 'logs_dir': '~/.local/state/devtray/logs', 'tmux_available': True})
    dest = root / 'docs/screenshots'
    dest.mkdir(parents=True, exist_ok=True)
    panel.show()
    QTest.qWait(60)
    app.processEvents()
    panel.grab().save(str(dest / 'overview.png'))
    panel.resize(1040, 720)
    QTest.qWait(60)
    app.processEvents()
    panel.grab().save('/tmp/devtray-compact.png')
    panel.resize(1340, 880)
    panel.navigate(2)
    samples = [('info', 'Listening on 127.0.0.1:3060'), ('info', 'GET /api/projects 200  12ms'),
               ('warn', 'Connection pool nearing capacity; retrying'), ('error', 'Request failed: database connection timeout'),
               ('debug', 'Reconnect attempt 1 of 3'), ('info', 'Connection restored')]
    panel.apply_logs({'entries': [{'level': level, 'text': f'2026-10-23 20:31:{i:02d}  {level.upper():5}  {text}', 'source': 0} for i, (level, text) in enumerate(samples * 5)],
                      'files': [{'index': 0, 'bytes': 6242, 'modified': 1792701000}, {'index': 1, 'bytes': 53242, 'modified': 1792700000}], 'truncated': False})
    QTest.qWait(60)
    app.processEvents()
    panel.grab().save(str(dest / 'logs.png'))
    dialog = ServiceDialog(panel)
    dialog.show()
    QTest.qWait(60)
    app.processEvents()
    dialog.grab().save('/tmp/devtray-form.png')
    print(dest)
