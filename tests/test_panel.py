"""Qt interaction regressions. Run with QT_QPA_PLATFORM=offscreen."""
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
TEMP = tempfile.TemporaryDirectory(prefix='devtray-qt-')
os.environ['XDG_CONFIG_HOME'] = TEMP.name
os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
sys.path.insert(0, str(ROOT / 'ui'))
from panel import Panel, ServiceDialog
from quick_menu import QuickMenu
from PySide6.QtWidgets import QApplication, QDialog, QPushButton
from PySide6.QtCore import Qt, QPoint, QEvent
from PySide6.QtGui import QTextCursor
from PySide6.QtTest import QTest

APP = QApplication.instance() or QApplication([])


def wait_for(condition, timeout=5):
    end = time.monotonic() + timeout
    while not condition() and time.monotonic() < end:
        APP.processEvents()
        QTest.qWait(10)
    assert condition(), 'Qt operation timed out'


class PanelTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(dir=TEMP.name)
        self.root = Path(self.directory.name)
        self.log = self.root / 'service.log'
        self.log.write_text('INFO boot\nWARN old-warning\nERROR failure\n')
        self.config = self.root / 'config.toml'
        self.config.write_text(f'# preserve this comment\n[[project]]\nname="test"\n[[project.service]]\nname="api"\ncwd="{self.root}"\ncmd="sleep 100"\nlog_file="{self.log}"\n')
        self.panel = Panel(str(ROOT / 'target/debug/devtray'), str(self.config), poll=False)
        self.panel.show()
        self.panel.refresh()
        wait_for(lambda: len(self.panel.data['services']) == 1)
        wait_for(lambda: not self.panel.log_loading)

    def tearDown(self):
        for process in list(self.panel.processes):
            process.kill()
            process.waitForFinished(1000)
        self.panel.hide()
        self.panel.deleteLater()
        APP.processEvents()
        self.directory.cleanup()

    def test_real_filters_search_and_historical_logs(self):
        w = self.panel
        w.navigate(2)
        for name, control in w.level_buttons.items():
            control.setChecked(name == 'warn')
        w.log_changed()
        wait_for(lambda: not w.log_loading)
        self.assertEqual(w.log_view.toPlainText(), 'WARN old-warning')
        Path(str(self.log) + '.1').write_text('WARN historical-warning\nINFO history\n')
        w.log_source.setCurrentIndex(w.log_source.findData('all'))
        wait_for(lambda: not w.log_loading)
        self.assertIn('historical-warning', w.log_view.toPlainText())
        self.assertNotIn('INFO history', w.log_view.toPlainText())
        w.log_search.setText('historical')
        w.search_timer.stop()
        w.log_changed()
        wait_for(lambda: not w.log_loading)
        self.assertIn('historical-warning', w.log_view.toPlainText())
        self.assertNotIn('old-warning', w.log_view.toPlainText())

    def test_drag_selection_survives_poll_and_inflight_response(self):
        w = self.panel
        w.navigate(2)
        wait_for(lambda: not w.log_loading)
        viewport = w.log_view.viewport()
        QTest.mousePress(viewport, Qt.LeftButton, pos=QPoint(20, 20))
        QTest.mouseMove(viewport, QPoint(110, 20), delay=20)
        QTest.mouseRelease(viewport, Qt.LeftButton, pos=QPoint(110, 20))
        self.assertTrue(w.paused)
        self.assertTrue(w.log_view.textCursor().hasSelection(), 'mouse release cleared the selection')
        # Select a deterministic substring after verifying the real drag survives mouse release.
        cursor = w.log_view.textCursor()
        cursor.setPosition(0)
        cursor.setPosition(9, QTextCursor.KeepAnchor)
        w.log_view.setTextCursor(cursor)
        selected = w.log_view.textCursor().selectedText()
        self.log.write_text(self.log.read_text() + 'INFO arriving during selection\n')
        w.refresh()
        wait_for(lambda: not w.refreshing)
        self.assertEqual(w.log_view.textCursor().selectedText(), selected)
        self.assertNotIn('arriving', w.log_view.toPlainText())
        w.copy_selected()
        self.assertEqual(APP.clipboard().text(), selected)
        w.toggle_pause()
        wait_for(lambda: not w.log_loading)
        self.assertIn('arriving', w.log_view.toPlainText())
        # A force-refresh already in flight must also respect a newly started drag.
        callbacks = []
        w.request = lambda args, callback, *a, **k: callbacks.append(callback)
        w.refresh_logs(force=True)
        w.freeze_logs()
        cursor = w.log_view.textCursor()
        cursor.setPosition(0)
        cursor.setPosition(9, QTextCursor.KeepAnchor)
        w.log_view.setTextCursor(cursor)
        callbacks[-1]({'entries': [{'text': 'REPLACED', 'level': 'info'}], 'files': [], 'truncated': False}, None)
        self.assertEqual(w.log_view.textCursor().selectedText(), selected)

    def test_close_hides_without_destroying_state(self):
        self.panel.navigate(2)
        self.panel.close()
        self.assertFalse(self.panel.isVisible())
        self.panel.show()
        self.assertTrue(self.panel.isVisible())
        self.assertEqual(self.panel.page, 2)

    def test_native_form_saves_and_preserves_comments(self):
        dialog = ServiceDialog(self.panel)
        for key, text in {'project': 'test', 'service': 'worker', 'cwd': str(self.root), 'cmd': 'sleep 60'}.items():
            dialog.inputs[key].setText(text)
        dialog.save()
        wait_for(lambda: dialog.result() == QDialog.Accepted)
        text = self.config.read_text()
        self.assertIn('# preserve this comment', text)
        self.assertIn('worker', text)
        wait_for(lambda: len(self.panel.data['services']) == 2)
        dialog.deleteLater()

    def test_invalid_form_leaves_config_untouched(self):
        before = self.config.read_bytes()
        dialog = ServiceDialog(self.panel)
        for key, text in {'project': 'test', 'service': 'broken', 'cwd': '/nonexistent/devtray-test', 'cmd': 'sleep 60'}.items():
            dialog.inputs[key].setText(text)
        dialog.save()
        wait_for(lambda: dialog.buttons.isEnabled())
        self.assertEqual(before, self.config.read_bytes())
        self.assertIn('cwd', dialog.error.text())
        dialog.deleteLater()

    def make_menu(self):
        self.panel.quick_menu = QuickMenu(self.panel)
        self.panel.quick_menu.present([500, 30])
        APP.processEvents()
        return self.panel.quick_menu

    def test_live_terminal_and_browser_exist_in_both_card_layouts(self):
        service = dict(self.panel.data['services'][0], state='ready', url='http://localhost:5173/')
        requests = []
        self.panel.request = lambda args, *a, **kw: requests.append(args)
        for wide in (False, True):
            box, _ = self.panel.service_card(service, wide)
            buttons = {b.text(): b for b in box.findChildren(QPushButton)}
            self.assertIn('实时终端', buttons)
            self.assertIn('浏览器 ↗', buttons)
            buttons['实时终端'].click()
            buttons['浏览器 ↗'].click()
            self.assertEqual(requests[-2:], [['terminal', 'test/api'], ['open', 'test/api']])
            box.deleteLater()

    def test_small_panel_has_terminal_browser_and_classified_log_actions(self):
        self.panel.data['services'][0].update(state='ready', url='http://localhost:5173/')
        menu = self.make_menu()
        item = menu.projects['test'].rows['test/api']
        buttons = {b.text(): b for b in item.findChildren(QPushButton)}
        self.assertIn('实时终端', buttons)
        self.assertIn('浏览器 ↗', buttons)
        requests = []
        self.panel.request = lambda args, *a, **kw: requests.append(args)
        buttons['实时终端'].click()
        buttons['浏览器 ↗'].click()
        self.assertEqual(requests, [['terminal', 'test/api'], ['open', 'test/api']])
        self.assertTrue(item.logs.isEnabled())

    def test_mode_buttons_remember_selection_but_log_shortcut_does_not(self):
        menu = self.make_menu()
        menu.expand_button.click()
        self.assertTrue(self.panel.isVisible())
        self.assertFalse(menu.isVisible())
        self.assertEqual(self.panel.preferred_mode(), 'full')
        self.panel.compact_button.click()
        self.assertTrue(menu.isVisible())
        self.assertFalse(self.panel.isVisible())
        self.assertEqual(self.panel.preferred_mode(), 'small')
        menu.open_logs('test/api')
        self.assertTrue(self.panel.isVisible())
        self.assertEqual(self.panel.page, 2)
        self.assertEqual(self.panel.preferred_mode(), 'small')
        self.panel.hide_panel()
        self.panel.present()
        self.assertTrue(menu.isVisible())

    def test_exited_terminal_is_accessible_and_browser_updates_without_row_rebuild(self):
        menu = self.make_menu()
        item = menu.projects['test'].rows['test/api']
        self.assertFalse(item.terminal.isEnabled())
        self.assertFalse(item.preview.isEnabled())
        self.panel.data['services'][0].update(state='exited_failed', url='http://localhost:5173/')
        menu.sync(self.panel.data)
        self.assertIs(item, menu.projects['test'].rows['test/api'])
        self.assertTrue(item.terminal.isEnabled())
        self.assertTrue(item.preview.isEnabled())

    def test_quick_menu_preserves_rows_on_status_updates(self):
        menu = self.make_menu()
        row = menu.projects['test'].rows['test/api']
        self.panel.data['services'][0]['state'] = 'ready'
        menu.sync(self.panel.data)
        self.assertIs(row, menu.projects['test'].rows['test/api'])
        self.assertIn('就绪', row.state.text())
        self.assertEqual(row.start_stop.toolTip(), '停止服务')
        self.panel.action_pending.add('test/api')
        menu.sync(self.panel.data)
        self.assertFalse(row.start_stop.isEnabled())

    def test_quick_menu_search_and_collapse(self):
        menu = self.make_menu()
        project = menu.projects['test']
        menu.search.setText('not-a-match')
        self.assertTrue(project.isHidden())
        self.assertFalse(menu.empty.isHidden())
        menu.search.setText(str(self.root))
        self.assertFalse(project.isHidden())
        self.assertFalse(project.contents.isHidden())
        menu.search.clear()
        menu.collapsed.discard('test')
        project.filter('')
        QTest.mouseClick(project.title, Qt.LeftButton)
        self.assertTrue(project.contents.isHidden())
        self.assertIn('test', json.loads(self.panel.settings.value('quick_collapsed')))

    def test_quick_menu_actions_target_service_and_open_logs(self):
        menu = self.make_menu()
        menu.search.setText('api')
        row = menu.projects['test'].rows['test/api']
        actions = []
        self.panel.action = lambda op, name: actions.append((op, name))
        QTest.mouseClick(row.start_stop, Qt.LeftButton)
        self.assertEqual(actions, [('start', 'test/api')])
        QTest.mouseClick(row.restart, Qt.LeftButton)
        self.assertEqual(actions[-1], ('restart', 'test/api'))
        QTest.mouseClick(row.logs, Qt.LeftButton)
        self.assertTrue(menu.isHidden())
        self.assertEqual(self.panel.page, 2)
        self.assertEqual(self.panel.log_service.currentData(), 'test/api')

    def test_quick_menu_deactivation_hides_and_stops_polling(self):
        menu = self.make_menu()
        QTest.mouseClick(menu.pin, Qt.LeftButton)
        self.assertFalse(menu.pinned)
        menu.timer.start()
        QApplication.sendEvent(menu, QEvent(QEvent.WindowDeactivate))
        self.assertFalse(menu.isVisible())
        self.assertFalse(menu.timer.isActive())

    def test_pinned_float_survives_focus_change_and_escape_still_hides(self):
        menu = self.make_menu()
        self.assertTrue(menu.pinned)
        menu.timer.start()
        QApplication.sendEvent(menu, QEvent(QEvent.WindowDeactivate))
        self.assertTrue(menu.isVisible())
        self.assertTrue(menu.timer.isActive())
        QTest.keyClick(menu, Qt.Key_Escape)
        self.assertFalse(menu.isVisible())
        self.assertFalse(menu.timer.isActive())

    def test_float_drag_position_and_pin_preference_survive_reopening(self):
        menu = self.make_menu()
        menu.move(60, 30)
        QTest.mouseClick(menu.pin, Qt.LeftButton)
        menu.hide()
        menu.present([700, 100])
        self.assertEqual(menu.pos(), QPoint(60, 30))
        menu.hide()
        second = QuickMenu(self.panel)
        self.assertFalse(second.pinned)
        second.deleteLater()


if __name__ == '__main__':
    unittest.main()
