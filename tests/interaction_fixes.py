"""Regression checks for desktop detection, project-relative cleanup, and dismissing notices."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get('DEVTRAY_TEST_BINARY', str(ROOT / 'target/debug/devtray'))
os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
os.environ['PYTHONDONTWRITEBYTECODE'] = '1'
TEST_SETTINGS = tempfile.TemporaryDirectory(prefix='devtray-test-settings-')
os.environ['XDG_CONFIG_HOME'] = TEST_SETTINGS.name
sys.path.insert(0, str(ROOT / 'ui'))
from PySide6.QtCore import Qt
from PySide6.QtWidgets import QApplication, QMessageBox
from panel import Panel
APP = QApplication.instance() or QApplication([])

with tempfile.TemporaryDirectory(prefix='devtray-interaction-') as directory:
    tmp = Path(directory)
    home = tmp / 'home'
    (home / 'bin').mkdir(parents=True)
    pi = home / 'bin/pi'
    pi.write_text('#!/bin/sh\necho "pi 1.4.0" >&2\n')
    pi.chmod(0o755)
    (home / '.kimi-code/bin').mkdir(parents=True)
    kimi = home / '.kimi-code/bin/kimi'
    kimi.write_text('#!/bin/sh\necho "kimi 1.2.3"\n')
    kimi.chmod(0o755)
    (home / '.local/bin').mkdir(parents=True)
    hermes = home / '.local/bin/hermes'
    hermes.write_text('#!/bin/sh\nexec sleep 20\n')
    hermes.chmod(0o755)
    env = dict(os.environ, HOME=str(home), PATH='/usr/bin:/bin', XDG_STATE_HOME=str(tmp / 'state'))
    result = subprocess.run([BINARY, 'agents', '--json'], env=env, capture_output=True, text=True, timeout=15)
    detected = json.loads(result.stdout)
    found = next((a for a in detected if a['id'] == 'pi'), None)
    assert found and found['version'] == '1.4.0', ('misses user bin or stderr version', found)
    assert next(a for a in detected if a['id'] == 'kimi-cli')['version'] == '1.2.3'
    assert next(a for a in detected if a['id'] == 'hermes')['version'] is None
    print('PASS: desktop PATH/user bins, stderr version, Kimi/Hermes registry, bounded hung probe')

    project = tmp / 'project'
    app = project / 'frontend'
    (app / 'node_modules/pkg').mkdir(parents=True)
    (app / 'node_modules/pkg/blob').write_text('keep until confirmed')
    (app / 'package.json').write_text('{}')
    (app / 'bun.lock').touch()
    config = tmp / 'config.toml'
    config.write_text(f'[[project]]\nname="demo"\nroot="{project}"\n[[project.service]]\nname="web"\ncwd="{app}"\ncmd="true"\n')
    os.environ['XDG_STATE_HOME'] = str(tmp / 'state')
    os.environ['DEVTRAY_CONFIG'] = str(config)
    panel = Panel(BINARY, str(config), poll=False)
    panel.resource_selected = 'demo'
    panel.resource_usages = {'demo': {
        'project':'demo','root':str(project),'inferred':False,'cleaning_allowed':True,
        'scanned_at':1,'total_allocated':4096,'total_logical':100,'complete':True,
        'artifacts':[{'path':'frontend/node_modules','kind':'node_modules','allocated':4096,
            'logical':100,'files':1,'deletable':True,'evidence':{'notes':[]}}],
    }}
    panel.render_resource_detail()
    panel.res_table.item(0,0).setCheckState(Qt.Checked)
    calls=[]
    panel.request=lambda args, callback, **kwargs: calls.append(args)
    panel.clean_selected()
    assert calls[0] == ['_clean-plan','demo','--path',str(app/'node_modules')], calls
    assert (app/'node_modules').exists()
    print('PASS: real panel clean selection passes absolute project path to backend')
    panel.notify('无法生成清理计划：fixture', error=True)
    panel.dismiss_notice()
    assert panel.banner.isHidden()
    assert calls[-1] == ['_dismiss-alerts'], calls
    print('PASS: dismiss clears banner and requests tray acknowledgement')
    panel.deleteLater()
    APP.processEvents()

# Exercise the same absolute --path against the real backend with an isolated tmux stub.
with tempfile.TemporaryDirectory(prefix='devtray-clean-plan-') as directory:
    tmp = Path(directory)
    root = tmp / 'project'
    target = root / 'frontend/node_modules/pkg'
    target.mkdir(parents=True)
    (target / 'blob').write_text('do not delete')
    (root / 'frontend/package.json').write_text('{}')
    (root / 'frontend/bun.lock').touch()
    config = tmp / 'config.toml'
    config.write_text(f'[[project]]\nname="demo"\nroot="{root}"\n[[project.service]]\nname="web"\ncwd="{root}/frontend"\ncmd="true"\n')
    bindir = tmp / 'bin'
    bindir.mkdir()
    tmux = bindir / 'tmux'
    tmux.write_text('#!/bin/sh\necho "no server running" >&2\nexit 1\n')
    tmux.chmod(0o755)
    env = dict(os.environ, DEVTRAY_CONFIG=str(config), XDG_STATE_HOME=str(tmp/'state'), PATH=f'{bindir}:/usr/bin:/bin')
    result = subprocess.run([BINARY, '_clean-plan', 'demo', '--path', str(target.parent)], env=env, capture_output=True, text=True, timeout=10)
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)['candidates'][0]['path'] == str(target.parent)
    refused = subprocess.run([BINARY, '_clean-plan', 'demo', '--path', str(tmp)], env=env, capture_output=True, text=True, timeout=10)
    assert refused.returncode != 0 and (target/'blob').is_file(), refused
    result = subprocess.run([BINARY, '_dismiss-alerts'], env=env, capture_output=True, text=True, timeout=10)
    assert json.loads(result.stdout)['dismissed']
    assert len(list((tmp/'state/devtray/alerts').glob('*.json'))) == 1
    print('PASS: backend plan JSON, outside-root refusal, non-destructive acknowledgement signal')

with tempfile.TemporaryDirectory(prefix='devtray-menus-') as directory:
    config = Path(directory) / 'config.toml'
    config.write_text('')
    panel = Panel(BINARY, str(config), poll=False)
    agent = {'name':'Pi', 'id':'pi', 'path':'/tmp/pi', 'version':'1.4.0', 'provenance':'unknown', 'notes':[]}
    panel.render_agents([agent])
    menu = panel.make_agent_menu(agent)
    actions = {a.text():a for a in menu.actions()}
    assert not actions['复制建议升级命令'].isEnabled()
    actions['复制可执行文件路径'].trigger()
    assert APP.clipboard().text() == '/tmp/pi'
    calls=[]
    panel.skill_action = lambda action, name=None: calls.append((action, name))
    skill = {'name':'alpha','dir':'/tmp/alpha','dir_exists':True,'managed':True,'mirrors':[]}
    menu = panel.make_skill_menu(skill)
    actions = {a.text():a for a in menu.actions()}
    actions['升级此技能'].trigger()
    assert calls == [('upgrade', 'alpha')]
    panel.active_job = 'pending'
    menu = panel.make_skill_menu(skill)
    assert not next(a for a in menu.actions() if a.text() == '删除此技能').isEnabled()
    assert panel.skills_table.isColumnHidden(4)
    assert len(panel.bulk_buttons) == 3, 'overview must not duplicate bulk operations'
    print('PASS: context actions copy paths, dispatch existing writes, disable while busy, overview scope')
    panel.deleteLater()
    APP.processEvents()
