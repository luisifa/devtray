#!/usr/bin/env python3
"""Render the implemented custom tray menu with clearly synthetic service data."""
import os
import sys
import tempfile
from pathlib import Path
os.environ['QT_QPA_PLATFORM'] = 'offscreen'
root = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(root / 'ui'))
from panel import QApplication, Panel
from quick_menu import QuickMenu
from PySide6.QtTest import QTest
with tempfile.TemporaryDirectory(prefix='devtray-menu-preview-') as tmp:
    os.environ['XDG_CONFIG_HOME'] = tmp
    app = QApplication([])
    owner = Panel(str(root/'target/debug/devtray'), str(Path(tmp)/'config.toml'), poll=False)
    owner.request = lambda *a, **k: None
    services = []
    for project, name, state, port in [
        ('atelier', 'backend', 'ready', 3060), ('atelier', 'frontend', 'ready', 5173),
        ('notes', 'worker', 'exited_failed', None), ('sandbox', 'preview', 'not_started', 4173),
    ]:
        services.append(dict(project=project,service=name,name=project+'/'+name,state=state,port=port,
                             exit_code=1 if state=='exited_failed' else None, cwd=f'~/Workspace/{project}/{name}',
                             cmd='bun run dev',ready=None,log=f'/tmp/{name}.log',autostart=False,url=f"http://localhost:{port}/" if port else None))
    owner.data={'services':services,'config':str(Path(tmp)/'config.toml')}
    menu=QuickMenu(owner)
    owner.quick_menu=menu
    menu.collapsed.add('sandbox')
    menu.sync(owner.data)
    menu.present([600, 10])
    QTest.qWait(100)
    dest=root/'docs/screenshots/quick-menu.png'
    menu.grab().save(str(dest))
    print(dest)
    menu.hide()
