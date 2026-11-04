"""Real Qt keyboard delivery, persisted overrides, drafts and operation feedback."""
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

TEMP = tempfile.TemporaryDirectory(prefix='devtray-keyboard-')
os.environ['XDG_CONFIG_HOME'] = TEMP.name
os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'ui'))
from PySide6.QtCore import Qt, QSettings, QTimer
from PySide6.QtGui import QKeySequence
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication, QDialog, QPushButton, QWidget
from keyboard_shortcuts import DEFAULTS, SETTINGS_KEY, ShortcutController, validate
from panel import Panel

APP = QApplication.instance() or QApplication([])


class KeyboardTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=TEMP.name)
        self.config = Path(self.tmp.name)/'config.toml'
        self.config.write_text('')
        self.w = Panel(str(ROOT/'target/debug/devtray'), str(self.config), poll=False)
        self.requests = []
        self.w.request = lambda args, callback, **kwargs: self.requests.append((args, callback))
        self.fixture = {'services': [dict(project='demo', service='api', name='demo/api',
            state='ready', cwd=self.tmp.name, cmd='true', session='devtray-demo-api',
            log=str(Path(self.tmp.name)/'api.log'), autostart=False, url=None)]}
        self.w.apply_data(self.fixture)
        self.w.show()
        self.w.activateWindow()
        QTest.qWait(15)

    def tearDown(self):
        if hasattr(self.w, 'quick_menu'):
            self.w.quick_menu.hide()
            self.w.quick_menu.timer.stop()
        self.w.feedback.timer.stop()
        self.w.timer.stop()
        self.w.hide()
        self.w.deleteLater()
        APP.processEvents()
        self.tmp.cleanup()

    def press(self, key, modifiers=Qt.NoModifier, target=None):
        QTest.keyClick(target or self.w, key, modifiers)
        APP.processEvents()

    def test_default_linux_actions_deliver_and_text_copy_stays_native(self):
        self.press(Qt.Key_Comma, Qt.ControlModifier)
        self.assertEqual(self.w.page, 6)
        self.press(Qt.Key_6, Qt.ControlModifier)
        self.assertEqual(self.w.page, 5)
        self.press(Qt.Key_F, Qt.ControlModifier)
        self.assertTrue(self.w.log_search.hasFocus())
        self.w.log_search.setText('retained text')
        self.w.log_search.selectAll()
        self.press(Qt.Key_C, Qt.ControlModifier, self.w.log_search)
        self.assertEqual(APP.clipboard().text(), 'retained text')
        self.press(Qt.Key_F, Qt.ControlModifier | Qt.ShiftModifier)
        self.assertTrue(self.w.project_search.hasFocus())

    def test_recording_an_existing_shortcut_does_not_execute_it(self):
        calls = []
        self.w.add_service = lambda: calls.append('add')
        self.w.navigate(6)
        editor = self.w.shortcut_editors['search']
        editor.setFocus()
        self.press(Qt.Key_N, Qt.ControlModifier, editor)
        QTest.qWait(1100)
        self.assertEqual(editor.keySequence().toString(QKeySequence.PortableText), 'Ctrl+N')
        self.assertEqual(calls, [])
        self.assertFalse(self.w.shortcut_apply.isEnabled())
        self.assertIn('重复', self.w.shortcut_error.text())
        self.assertEqual(self.w.shortcut_manager.values['search'], 'Ctrl+F')

    def test_draft_survives_status_polling_and_navigation(self):
        self.w.navigate(6)
        editor = self.w.shortcut_editors['search']
        editor.setKeySequence(QKeySequence('Ctrl+G'))
        for _ in range(3):
            self.w.apply_data(self.fixture)
        self.w.navigate(0)
        self.w.navigate(6)
        self.assertIs(self.w.shortcut_editors['search'], editor)
        self.assertEqual(editor.keySequence().toString(), 'Ctrl+G')
        self.assertEqual(self.w.shortcut_manager.values['search'], 'Ctrl+F')
        self.assertTrue(self.w.shortcut_apply.isEnabled())
        self.w.shortcut_cancel.click()
        self.assertEqual(editor.keySequence().toString(), 'Ctrl+F')

    def test_applied_override_replaces_old_key_and_survives_new_controller(self):
        self.w.shortcut_editors['search'].setKeySequence(QKeySequence('Ctrl+G'))
        self.w.shortcut_apply.click()
        self.assertEqual(self.w.shortcut_manager.values['search'], 'Ctrl+G')
        self.w.navigate(5)
        self.w.refresh_button.setFocus()
        self.press(Qt.Key_F, Qt.ControlModifier)
        self.assertTrue(self.w.refresh_button.hasFocus(), 'old shortcut is still active')
        self.press(Qt.Key_G, Qt.ControlModifier)
        self.assertTrue(self.w.log_search.hasFocus())
        self.assertEqual(ShortcutController(self.w.settings, lambda *_: None).values['search'], 'Ctrl+G')
        other = QSettings('devtray-test', 'other-'+Path(self.tmp.name).name)
        self.assertEqual(ShortcutController(other, lambda *_: None).values['search'], 'Ctrl+F')

    def test_small_panel_obeys_the_same_hide_override_and_disabled_default(self):
        self.w.present('small')
        menu = self.w.quick_menu
        values = dict(self.w.shortcut_manager.values, hide='F8')
        self.assertEqual(self.w.shortcut_manager.save(values), {})
        self.assertEqual(len(self.w.shortcut_manager.bindings), 2)
        self.press(Qt.Key_Escape, target=menu)
        self.assertTrue(menu.isVisible(), 'hard-coded Escape still overrides configuration')
        self.press(Qt.Key_F8, target=menu)
        self.assertFalse(menu.isVisible())

    def test_disable_restore_defaults_and_storage_fallback(self):
        self.w.shortcut_editors['refresh'].clear()
        self.w.shortcut_apply.click()
        self.assertEqual(self.w.shortcut_manager.values['refresh'], '')
        self.assertEqual(ShortcutController(self.w.settings, lambda *_: None).values['refresh'], '')
        self.w.restore_shortcut_defaults()
        self.assertEqual(self.w.shortcut_manager.values['refresh'], '', 'reset applied before confirmation')
        self.w.shortcut_apply.click()
        self.assertEqual(self.w.shortcut_manager.values, DEFAULTS)
        self.w.settings.setValue(SETTINGS_KEY, '{broken')
        recovered = ShortcutController(self.w.settings, lambda *_: None)
        self.assertEqual(recovered.values, DEFAULTS)
        self.assertTrue(recovered.load_error)

    def test_duplicate_reserved_and_multistroke_bindings_are_rejected(self):
        for value in ['Ctrl+C', 'Alt+F4', 'Meta+R', 'Ctrl+Alt+F3', 'A', 'Tab', 'Ctrl+K, Ctrl+R']:
            with self.subTest(value=value):
                _values, errors = validate(dict(DEFAULTS, refresh=value))
                self.assertIn('refresh', errors)
        _values, errors = validate(dict(DEFAULTS, refresh='Ctrl+N'))
        self.assertIn('refresh', errors)
        self.assertIn('new_service', errors)

    def test_closing_feedback_does_not_acknowledge_services_or_shift_cards(self):
        self.w.navigate(1)
        APP.processEvents()
        geometry = self.w.services_layout.itemAt(1).widget().geometry()
        before = len(self.requests)
        self.w.notify('saved')
        APP.processEvents()
        self.assertEqual(self.w.feedback.property('feedbackKind'), 'success')
        self.assertTrue(self.w.feedback.timer.isActive())
        self.assertEqual(self.w.services_layout.itemAt(1).widget().geometry(), geometry)
        self.w.dismiss_button.click()
        self.assertTrue(self.w.banner.isHidden())
        self.assertEqual(len(self.requests), before)
        self.w.notify('failed', error=True)
        self.assertEqual(self.w.feedback.property('feedbackKind'), 'error')
        self.assertFalse(self.w.feedback.timer.isActive())
        self.assertTrue(self.w.banner.isVisible())

    def test_manual_refresh_and_service_busy_feedback(self):
        self.w.refresh_button.click()
        self.assertEqual(self.w.refresh_button.text(), '刷新中…')
        self.assertFalse(self.w.refresh_button.isEnabled())
        callback = next(callback for args, callback in self.requests if args == ['_panel-data'])
        callback(self.fixture, None)
        self.assertTrue(self.w.refresh_button.isEnabled())
        self.w.action('restart', 'demo/api')
        box = self.w.cards.itemAtPosition(0, 0).widget()
        self.assertTrue(box.property('busy'))
        controls = [b for b in box.findChildren(QPushButton) if b.text() == '重启中…']
        self.assertEqual(len(controls), 1)
        self.assertFalse(controls[0].isEnabled())
        before = len(self.requests)
        self.w.action('restart', 'demo/api')
        self.assertEqual(len(self.requests), before)
        callback = next(callback for args, callback in self.requests if args == ['restart', 'demo/api'])
        callback('', 'test failure')
        self.assertNotIn('demo/api', self.w.action_pending)
        self.assertNotIn('demo/api', self.w.pending_actions)
        self.assertEqual(self.w.feedback.property('feedbackKind'), 'error')

    def test_modal_dialog_blocks_window_shortcut_dispatch(self):
        calls = []
        self.w.add_service = lambda: calls.append('add')
        dialog = QDialog(self.w)
        dialog.setModal(True)
        dialog.show()
        APP.processEvents()
        try:
            self.w.shortcut_manager.dispatch('new_service', self.w)
            self.assertEqual(calls, [])
        finally:
            dialog.reject()
            dialog.deleteLater()

    def test_f1_shows_current_shortcut_help(self):
        seen = []
        def inspect_and_close():
            dialog = APP.activeModalWidget()
            if dialog is not None:
                seen.append(dialog.windowTitle())
                dialog.reject()
        QTimer.singleShot(60, inspect_and_close)
        self.press(Qt.Key_F1)
        QTest.qWait(80)
        self.assertEqual(seen, ['devtray · 快捷键说明'])

    def test_shortcuts_are_inactive_in_an_unrelated_window(self):
        calls = []
        self.w.add_service = lambda: calls.append('add')
        other = QWidget()
        other.show()
        other.activateWindow()
        QTest.qWait(15)
        try:
            self.press(Qt.Key_N, Qt.ControlModifier, other)
            self.assertEqual(calls, [])
        finally:
            other.hide()
            other.deleteLater()
        self.w.activateWindow()
        QTest.qWait(15)
        self.press(Qt.Key_N, Qt.ControlModifier)
        self.assertEqual(calls, ['add'])

    def test_scan_button_reports_busy_and_recovers_after_failure(self):
        self.w.resource_selected = 'demo'
        self.w.update_resource_controls()
        self.w.scan_project('demo', refresh=True)
        self.assertEqual(self.w.res_scan_current.text(), '扫描中…')
        self.assertFalse(self.w.res_scan_current.isEnabled())
        callback = next(callback for args, callback in self.requests if args[:2] == ['_disk-scan', 'demo'])
        callback({}, 'test scan error')
        self.assertEqual(self.w.res_scan_current.text(), '重新扫描')
        self.assertTrue(self.w.res_scan_current.isEnabled())
        self.assertEqual(self.w.feedback.property('feedbackKind'), 'error')


if __name__ == '__main__':
    unittest.main()
