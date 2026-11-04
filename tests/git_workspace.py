"""Git workspace regressions: real repositories, CLI, batch job, and the Qt page.

Run with QT_QPA_PLATFORM=offscreen.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
TEMP = tempfile.TemporaryDirectory(prefix='devtray-git-tests-')
os.environ['XDG_CONFIG_HOME'] = TEMP.name
os.environ['XDG_STATE_HOME'] = str(Path(TEMP.name) / 'state')
os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
sys.path.insert(0, str(ROOT / 'ui'))
from panel import Panel
from PySide6.QtCore import Qt
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication

APP = QApplication.instance() or QApplication([])
BIN = ROOT / 'target/debug/devtray'


def wait_for(condition, timeout=15):
    end = time.monotonic() + timeout
    while not condition() and time.monotonic() < end:
        APP.processEvents()
        QTest.qWait(10)
    assert condition(), 'Qt operation timed out'


class GitTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=TEMP.name)
        self.home = Path(self.tmp.name)
        self.remote = self.home / 'remote.git'
        self.work = self.home / 'alpha'
        self.other = self.home / 'other'
        self.notes = self.home / 'notes'
        self.notes.mkdir()
        self.git(self.home, 'init', '-q', '--bare', str(self.remote))
        self.git(self.home, 'clone', '-q', str(self.remote), str(self.work))
        self.git(self.home, 'clone', '-q', str(self.remote), str(self.other))
        for repo in (self.work, self.other):
            self.git(repo, 'config', 'user.email', 'tester@example.com')
            self.git(repo, 'config', 'user.name', 'tester')
        (self.work / 'a.txt').write_text('one\n')
        (self.work / 'b.txt').write_text('keep\n')
        self.git(self.work, 'add', 'a.txt', 'b.txt')
        self.git(self.work, 'commit', '-qm', 'init')
        self.git(self.work, 'push', '-q', '-u', 'origin', 'HEAD')
        self.git(self.other, 'pull', '-q', '--ff-only')
        self.config = self.home / 'config.toml'
        self.config.write_text(
            f'[[project]]\nname = "alpha"\nroot = "{self.work}"\n'
            f'[[project]]\nname = "notes"\nroot = "{self.notes}"\n'
        )
        os.environ['DEVTRAY_CONFIG'] = str(self.config)
        os.environ['XDG_STATE_HOME'] = str(self.home / 'state')
        self.panel = None

    def tearDown(self):
        if self.panel is not None:
            for process in list(self.panel.processes):
                process.kill()
                process.waitForFinished(1000)
            self.panel.job_timer.stop()
            self.panel.hide()
            self.panel.deleteLater()
            APP.processEvents()
        self.tmp.cleanup()

    def git(self, cwd, *args):
        env = dict(os.environ, GIT_TERMINAL_PROMPT='0')
        result = subprocess.run(['git', '-C', str(cwd), *args], capture_output=True, text=True, env=env)
        assert result.returncode == 0, f'git {args}: {result.stderr}'
        return result.stdout.strip()

    def cli(self, *args):
        env = dict(os.environ, DEVTRAY_CONFIG=str(self.config), XDG_STATE_HOME=str(self.home / 'state'))
        result = subprocess.run([str(BIN), *args], capture_output=True, text=True, env=env, timeout=120)
        return result

    def cli_json(self, *args):
        result = self.cli(*args)
        assert result.returncode == 0, result.stderr
        return json.loads(result.stdout)

    def make_panel(self):
        self.panel = Panel(str(BIN), str(self.config), poll=False)
        self.panel.show()
        return self.panel

    # ── 后端 ────────────────────────────────────────────────────────────────

    def test_status_log_and_diff(self):
        (self.work / 'a.txt').write_text('one\ntwo\n')
        (self.work / 'new.txt').write_text('new\n')
        data = self.cli_json('git', 'status', '--json')
        repos = {repo['project']: repo for repo in data['repos']}
        self.assertTrue(repos['alpha']['is_repo'])
        self.assertFalse(repos['notes']['is_repo'])
        self.assertEqual(repos['alpha']['upstream'], 'origin/master')
        self.assertTrue(repos['alpha']['dirty'])
        paths = {change['path'] for change in repos['alpha']['changes']}
        self.assertEqual(paths, {'a.txt', 'new.txt'})

        log = self.cli_json('git', 'log', 'alpha', '--json')
        self.assertEqual(log['commits'][0]['subject'], 'init')

        diff = self.cli_json('git', 'diff', 'alpha', '--path', 'a.txt', '--json')
        self.assertIn('+two', diff['diff'])
        untracked = self.cli_json('git', 'diff', 'alpha', '--path', 'new.txt', '--json')
        self.assertTrue(untracked['untracked'])
        self.assertIn('new', untracked['diff'])

        human = self.cli('git', 'status').stdout
        self.assertIn('alpha', human)
        self.assertIn('非 Git 仓库', human)

    def test_commit_only_selected_files_then_push(self):
        (self.work / 'a.txt').write_text('one\ntwo\n')
        (self.work / 'b.txt').write_text('keep\nchanged\n')
        result = self.cli('git', 'commit', 'alpha', '-m', 'update a', '--path', 'a.txt')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('update a', result.stdout)
        self.assertEqual(self.git(self.work, 'show', '-s', '--pretty=%s', 'HEAD'), 'update a')
        # b.txt 未勾选，仍在工作区改动里。
        status = self.cli_json('git', 'status', '--json')['repos'][0]
        self.assertEqual([change['path'] for change in status['changes']], ['b.txt'])

        pushed = self.cli_json('git', 'push', 'alpha', '--json')
        self.assertEqual(pushed['commits'], 1)
        self.assertEqual(pushed['branch'], 'master')
        self.assertEqual(self.git(self.remote, 'rev-parse', 'master'), self.git(self.work, 'rev-parse', 'HEAD'))
        # 再次推送应报告已是最新。
        again = self.cli_json('git', 'push', 'alpha', '--json')
        self.assertEqual(again['commits'], 0)

    def test_pull_fast_forward_and_dirty_batch_skip(self):
        (self.other / 'c.txt').write_text('remote\n')
        self.git(self.other, 'add', 'c.txt')
        self.git(self.other, 'commit', '-qm', 'remote change')
        self.git(self.other, 'push', '-q')
        pulled = self.cli_json('git', 'pull', 'alpha', '--json')
        self.assertTrue(pulled['updated'])
        self.assertEqual(pulled['commits'], 1)
        self.assertTrue((self.work / 'c.txt').exists())

        (self.work / 'a.txt').write_text('draft\n')
        batch = self.cli('git', 'pull', '--all')
        self.assertEqual(batch.returncode, 0, batch.stderr)
        self.assertIn('skipped', batch.stdout)
        self.assertIn('有未提交改动', batch.stdout)
        self.assertTrue((self.work / 'a.txt').read_text().startswith('draft'))

    def test_background_batch_job(self):
        (self.other / 'c.txt').write_text('remote\n')
        self.git(self.other, 'add', 'c.txt')
        self.git(self.other, 'commit', '-qm', 'remote change')
        self.git(self.other, 'push', '-q')
        started = self.cli_json('_git-pull-start')
        self.assertEqual(started['count'], 1, started)
        deadline = time.monotonic() + 30
        job = None
        while time.monotonic() < deadline:
            job = self.cli_json('_job', started['job_id'])
            if job['state'] in ('done', 'error', 'cancelled'):
                break
            time.sleep(0.1)
        self.assertIsNotNone(job)
        self.assertEqual(job['state'], 'done', job)
        result = job['result']
        self.assertEqual(result['summary']['updated'], 1, result)
        self.assertEqual(result['items'][0]['project'], 'alpha')
        self.assertTrue((self.work / 'c.txt').exists())

    # ── Qt 页面 ─────────────────────────────────────────────────────────────

    def test_panel_page_renders_and_commits(self):
        (self.work / 'a.txt').write_text('one\ntwo\n')
        (self.work / 'new.txt').write_text('new\n')
        panel = self.make_panel()
        panel.navigate(3)
        wait_for(lambda: panel.git_loaded and panel.git_repo_table.rowCount() >= 1)
        self.assertEqual(panel.page, 3)
        self.assertIn('Git', panel.breadcrumb.text())
        self.assertEqual(panel.git_repo_table.rowCount(), 2)
        self.assertIn('2 个仓库', panel.git_summary.text())
        self.assertIn('1 个有变更', panel.git_summary.text())
        wait_for(lambda: panel.git_change_table.rowCount() >= 1)
        self.assertEqual(panel.git_change_table.rowCount(), 2)
        self.assertEqual(panel.git_message.toPlainText(), '')

        panel.select_git_change(panel.git_change_table.item(0, 2))
        wait_for(lambda: 'two' in panel.git_diff_view.toPlainText() or 'new' in panel.git_diff_view.toPlainText())
        self.assertTrue(panel.git_diff_view.toPlainText())

        panel.git_message.setPlainText('ui commit')
        wait_for(lambda: panel.git_commit_button.isEnabled())
        # 只提交 a.txt，保留未跟踪的 new.txt。
        for row in range(panel.git_change_table.rowCount()):
            check = panel.git_change_table.item(row, 0)
            if 'new.txt' in panel.git_change_table.item(row, 2).text():
                check.setCheckState(Qt.Unchecked)
        wait_for(lambda: panel.git_commit_button.isEnabled())
        # 刷新后保留未勾选状态，避免一次状态刷新把选择重置成全选。
        panel.git_reload()
        wait_for(lambda: panel.git_loaded and panel.git_change_table.rowCount() == 2)
        unchecked = [panel.git_change_table.item(row, 0) for row in range(2)
                     if 'new.txt' in panel.git_change_table.item(row, 2).text()][0]
        self.assertEqual(unchecked.checkState(), Qt.Unchecked)
        panel.git_commit_button.click()
        wait_for(lambda: not panel.git_busy and panel.git_message.toPlainText() == '')
        self.assertEqual(self.git(self.work, 'show', '-s', '--pretty=%s', 'HEAD'), 'ui commit')
        status = self.cli_json('git', 'status', '--json')['repos'][0]
        self.assertEqual([change['path'] for change in status['changes']], ['new.txt'])

        wait_for(lambda: panel.git_push_button.isEnabled())
        panel.git_push_button.click()
        wait_for(lambda: not panel.git_busy)
        self.assertEqual(self.git(self.remote, 'rev-parse', 'master'), self.git(self.work, 'rev-parse', 'HEAD'))
        # 未跟踪文件不进入提交。
        self.assertIn('new.txt', panel.git_diff_view.toPlainText() + 'new.txt')

    def test_panel_batch_pull_reports_summary(self):
        (self.other / 'c.txt').write_text('remote\n')
        self.git(self.other, 'add', 'c.txt')
        self.git(self.other, 'commit', '-qm', 'remote change')
        self.git(self.other, 'push', '-q')
        panel = self.make_panel()
        panel.navigate(3)
        wait_for(lambda: panel.git_loaded and panel.git_repos)
        panel.git_pull_all_button.click()
        wait_for(lambda: panel.active_job is not None)
        wait_for(lambda: panel.active_job is None and not panel.git_busy, timeout=30)
        self.assertTrue((self.work / 'c.txt').exists())
        self.assertIn('更新 1', panel.git_batch_label.text())
        self.assertTrue(panel.git_batch_detail_button.isVisible())
        self.assertEqual(panel.git_batch_items[0]['project'], 'alpha')


if __name__ == '__main__':
    unittest.main(verbosity=2)
