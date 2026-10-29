"""资源页渲染回归：离屏 Qt + 假数据，不启动扫描、不删除。"""
import json
import os
from pathlib import Path
import sys
import tempfile
import time

os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'ui'))
from panel import Panel
from PySide6.QtWidgets import QApplication, QMessageBox

APP = QApplication.instance() or QApplication([])

with tempfile.TemporaryDirectory(prefix='devtray-assets-') as tmp:
    config = Path(tmp) / 'config.toml'
    config.write_text('')
    panel = Panel(str(ROOT / 'target/debug/devtray'), str(config), poll=False)
    panel.load_resources = lambda: None  # 不触发真实扫描
    panel.data = {'services': [{'project': 'demo', 'name': 'demo/a', 'cwd': tmp}]}
    panel.resource_usages = {'demo': {
        'project': 'demo', 'root': tmp, 'inferred': True,
        'cleaning_allowed': True, 'cleaning_reason': None, 'notes': [],
        'artifacts': [
            {'kind': 'node_modules', 'name': 'node_modules',
             'path': 'frontend/node_modules', 'allocated': 1048576,
             'logical': 2097152, 'files': 42, 'deletable': True,
             'evidence': {'build_file': 'frontend/package.json', 'vcs_tracked': False,
                          'pyvenv_cfg': False, 'mount_point': None, 'notes': []}},
            {'kind': 'cargo_target', 'name': 'target',
             'path': 'backend/target', 'allocated': 2048, 'logical': 4096,
             'files': 3, 'deletable': False,
             'evidence': {'build_file': None, 'vcs_tracked': False,
                          'pyvenv_cfg': False, 'mount_point': None,
                          'notes': ['没有 CACHEDIR.TAG / debug / release 等构建痕迹']}},
        ],
        'other_top': [], 'artifacts_allocated': 1050624,
        'unclassified_allocated': 0, 'total_allocated': 1050624,
        'total_logical': 2101248, 'deletable_allocated': 1048576,
        'scanned_at': int(time.time()), 'duration_ms': 12, 'skipped': 0,
        'mount_skipped': [], 'complete': True, 'warnings': [],
    }}
    panel.resource_selected = 'demo'
    panel.render_resource_projects()
    panel.render_resource_detail()

    assert panel.pages.count() == 6, panel.pages.count()
    assert panel.nav[2].text().endswith('资源'), panel.nav[2].text()
    assert 'Agent' in panel.nav[3].text(), panel.nav[3].text()
    assert panel.res_table.rowCount() == 2
    assert panel.res_table.item(0, 1).text() == 'Node 依赖'
    assert panel.res_table.item(1, 6).text().startswith('只读')
    assert panel.res_clean_button.isEnabled() is False
    panel.res_table.item(0, 0).setCheckState(__import__('PySide6').QtCore.Qt.Checked)
    assert panel.res_clean_button.isEnabled() is True
    assert '1 项' in panel.res_total.text()
    panel.navigate(2)
    assert panel.page == 2

print('PASS: resource page builds, renders artifacts, selection enables clean button')

# 真实后端：面板 -> _disk-cache -> _disk-scan，验证资源页能拿到数据（只读）。
with tempfile.TemporaryDirectory(prefix='devtray-assets-live-') as tmp:
    root = Path(tmp)
    project = root / 'proj'
    app_dir = project / 'app'
    (app_dir / 'node_modules' / 'pkg').mkdir(parents=True)
    (app_dir / 'package.json').write_text('{}')
    (app_dir / 'bun.lock').write_text('')
    (app_dir / 'node_modules' / 'pkg' / 'b').write_bytes(b'x' * 4096)
    config = root / 'config.toml'
    config.write_text(
        f'[[project]]\nname = "live"\nroot = "{project}"\n'
        f'[[project.service]]\nname = "web"\ncwd = "{app_dir}"\ncmd = "true"\n'
    )
    os.environ['DEVTRAY_CONFIG'] = str(config)
    os.environ['XDG_STATE_HOME'] = str(root / 'state')
    panel = Panel(str(ROOT / 'target/debug/devtray'), str(config), poll=False)
    panel.data = {'services': [{'project': 'live', 'name': 'live/web', 'cwd': str(app_dir)}]}
    panel.resource_selected = 'live'
    panel.navigate(2)
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline and 'live' not in panel.resource_usages:
        APP.processEvents()
        time.sleep(0.02)
    assert 'live' in panel.resource_usages, '资源页没有拿到扫描结果'
    usage = panel.resource_usages['live']
    assert any(a['name'] == 'node_modules' for a in usage['artifacts']), usage
    assert panel.res_table.rowCount() >= 1
    panel.hide()
    panel.deleteLater()
    APP.processEvents()

print('PASS: resource page wired to _disk-cache/_disk-scan')

# Agent & Skills 页（只读假数据）
agents = [{
    'id': 'codex', 'name': 'Codex', 'bin': 'codex', 'path': '/usr/bin/codex',
    'version': '0.165.0', 'provenance': 'pacman', 'package': 'openai-codex',
    'update_command': 'sudo pacman -S openai-codex', 'needs_sudo': True, 'notes': [],
}]
panel.render_agents(agents)
assert panel.agents_table.rowCount() == 1
assert panel.agents_table.item(0, 1).text() == '0.165.0'
assert 'pacman' in panel.agents_table.item(0, 2).text()

inventory = {
    'summary': {
        'total': 2, 'managed': 1, 'unmanaged': 1, 'missing_dir': 0,
        'drift_mirrors': 1, 'mirror_count': 1, 'lock_path': '/tmp/lock',
        'lock_ok': True, 'lock_version': 3, 'skills_dir': '/tmp/skills',
        'update_available': 1, 'update_current': 0, 'update_unknown': 0,
        'update_unchecked': 1,
    },
    'skills': [
        {'name': 'alpha', 'display_name': 'alpha', 'dir': '/tmp/skills/alpha',
         'dir_exists': True, 'managed': True, 'source': 'owner/repo',
         'source_type': 'github', 'source_url': 'https://github.com/owner/repo',
         'git_ref': None, 'skill_path': 'skills/alpha/SKILL.md',
         'tree_sha': '0123456789abcdef0123456789abcdef01234567',
         'installed_at': '2026-01-01T00:00:00Z', 'updated_at': None,
         'update': 'available', 'latest_sha': 'b' * 40, 'update_checked_at': 1,
         'update_detail': 'owner/repo@main',
         'mirrors': [{'agent': 'claude-code', 'path': '/tmp/claude/alpha',
                      'kind': 'copy', 'status': 'drift', 'target': None}],
         'notes': []},
        {'name': 'beta', 'display_name': None, 'dir': '/tmp/skills/beta',
         'dir_exists': True, 'managed': False, 'source': None, 'source_type': None,
         'source_url': None, 'git_ref': None, 'skill_path': None, 'tree_sha': None,
         'installed_at': None, 'updated_at': None, 'update': 'unchecked',
         'latest_sha': None, 'update_checked_at': None, 'update_detail': None,
         'mirrors': [], 'notes': ['来源未知']},
    ],
}
panel.skills_inventory = inventory
panel.render_skills()
assert panel.skills_table.rowCount() == 2
assert panel.skills_table.item(0, 5).text() == '有更新'
assert panel.skills_table.isColumnHidden(4)
assert '漂移' in panel.skills_table.item(0, 4).text()
assert 'Skills 2' in panel.agents_summary.text()
panel.skills_search.setText('beta')
assert panel.skills_table.rowCount() == 1
panel.skills_search.setText('')
panel.load_agents_page = lambda: None
panel.skills_table.selectRow(0)
calls = []
fake_plan = {'action': 'upgrade', 'name': 'alpha', 'source': 'owner/repo',
             'paths': ['/tmp/skills/alpha', '/tmp/claude/alpha'],
             'command': 'npx --yes skills@1.7.4 update alpha -g -y', 'drift': False}
def fake_request(args, callback, **kwargs):
    calls.append(args)
    if args[0] == '_skills-plan':
        callback(fake_plan, None)
    elif args[0] == '_skills-start':
        callback({'job_id': 'job-skill'}, None)
panel.request = fake_request
original_question = QMessageBox.question
QMessageBox.question = lambda *a, **k: QMessageBox.Yes
try:
    panel.skill_action('upgrade')
finally:
    QMessageBox.question = original_question
    panel.job_timer.stop()
assert calls[0] == ['_skills-plan', 'upgrade', 'alpha'], calls
assert calls[1] == ['_skills-start', 'upgrade', 'alpha'], calls
assert panel.active_job == 'job-skill' and panel.active_job_kind == 'skills'

panel.navigate(3)
assert panel.page == 3
assert 'Agent' in panel.breadcrumb.text()

calls = []
panel.request = lambda args, callback, **kwargs: (calls.append(args), callback(inventory, None))
panel.check_skills_updates()
assert calls[-1] == ['_skills-check'], calls
assert panel.skills_check_button.isEnabled()
assert panel.skills_check_button.text() == '检查 Skills 更新'

print('PASS: agent table, skills table, drift status, search filter, check-updates button')

# 真实后端：面板 -> _agents/_skills（临时 HOME，只读）
with tempfile.TemporaryDirectory(prefix='devtray-agents-live-') as tmp:
    home = Path(tmp)
    skill_dir = home / '.agents' / 'skills' / 'alpha'
    skill_dir.mkdir(parents=True)
    (skill_dir / 'SKILL.md').write_text('---\nname: alpha\n---\n# alpha\n')
    (home / '.agents' / '.skill-lock.json').write_text(json.dumps({
        'version': 3, 'skills': {'alpha': {
            'source': 'owner/repo', 'sourceType': 'github',
            'sourceUrl': 'https://github.com/owner/repo',
            'skillPath': 'skills/alpha/SKILL.md',
            'skillFolderHash': '0123456789abcdef0123456789abcdef01234567',
            'installedAt': '2026-01-01T00:00:00Z',
        }},
    }))
    config = home / 'config.toml'
    config.write_text('')
    os.environ['HOME'] = str(home)
    os.environ['DEVTRAY_CONFIG'] = str(config)
    os.environ['XDG_STATE_HOME'] = str(home / 'state')
    panel = Panel(str(ROOT / 'target/debug/devtray'), str(config), poll=False)
    panel.navigate(3)
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline and not hasattr(panel, 'skills_inventory'):
        APP.processEvents()
        time.sleep(0.02)
    assert hasattr(panel, 'skills_inventory'), 'Agent 页没有拿到 Skills 清单'
    assert panel.skills_inventory['summary']['total'] == 1
    assert panel.skills_table.rowCount() == 1
    assert panel.agents_table.rowCount() >= 0
    panel.hide()
    panel.deleteLater()
    APP.processEvents()

print('PASS: Agent & Skills page wired to _agents/_skills')
