"""Shortcut editing and desktop keyboard actions, shared by both panel modes."""
from PySide6.QtCore import Qt
from PySide6.QtGui import QKeySequence
from PySide6.QtWidgets import (
    QAbstractItemView, QDialog, QDialogButtonBox, QHBoxLayout, QHeaderView, QLabel,
    QPushButton, QTableWidget, QTableWidgetItem, QVBoxLayout, QWidget,
)
from keyboard_shortcuts import DEFAULTS, SPECS, ShortcutController, ShortcutRecorder, validate


def text(value):
    result = QLabel(value)
    result.setTextFormat(Qt.PlainText)
    result.setWordWrap(True)
    return result


class ShortcutSettingsMixin:
    def init_shortcuts(self):
        self.shortcut_manager = ShortcutController(self.settings, self.run_shortcut)
        self.shortcut_manager.attach(self)

    def build_shortcut_settings(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setContentsMargins(14, 14, 14, 14)
        layout.setSpacing(12)
        caption = text('点击快捷键框后按下新组合，清空可禁用。仅在 devtray 窗口内生效；应用后立即生效并保存。')
        caption.setObjectName('muted')
        layout.addWidget(caption)
        self.shortcut_table = QTableWidget(len(SPECS), 4)
        self.shortcut_table.setAccessibleName('自定义快捷键')
        self.shortcut_table.setHorizontalHeaderLabels(['操作', '默认快捷键', '当前设置', ''])
        self.shortcut_table.verticalHeader().setVisible(False)
        self.shortcut_table.verticalHeader().setDefaultSectionSize(56)
        self.shortcut_table.setEditTriggers(QAbstractItemView.NoEditTriggers)
        self.shortcut_table.setSelectionMode(QAbstractItemView.NoSelection)
        self.shortcut_table.setAlternatingRowColors(True)
        self.shortcut_table.setColumnWidth(0, 205)
        self.shortcut_table.setColumnWidth(1, 140)
        self.shortcut_table.setColumnWidth(3, 92)
        self.shortcut_table.horizontalHeader().setSectionResizeMode(2, QHeaderView.Stretch)
        self.shortcut_editors = {}
        for index, (key, title, default) in enumerate(SPECS):
            self.shortcut_table.setItem(index, 0, QTableWidgetItem(title))
            self.shortcut_table.setItem(index, 1, QTableWidgetItem(QKeySequence(default).toString(QKeySequence.NativeText)))
            editor = ShortcutRecorder(self.shortcut_manager.values[key])
            editor.setAccessibleName('快捷键 ' + title)
            editor.setToolTip('按下新的组合；点击清空按钮可禁用此操作')
            editor.keySequenceChanged.connect(self.validate_shortcut_draft)
            self.shortcut_editors[key] = editor
            self.shortcut_table.setCellWidget(index, 2, editor)
            reset = QPushButton('恢复默认')
            reset.setAccessibleName('恢复默认 ' + title)
            reset.clicked.connect(lambda checked=False, key=key: self.shortcut_editors[key].setKeySequence(QKeySequence(DEFAULTS[key])))
            self.shortcut_table.setCellWidget(index, 3, reset)
        layout.addWidget(self.shortcut_table, 1)
        self.shortcut_error = text('')
        self.shortcut_error.setObjectName('validationMessage')
        self.shortcut_error.hide()
        layout.addWidget(self.shortcut_error)
        self.shortcut_draft_status = text('当前使用已保存的设置')
        self.shortcut_draft_status.setObjectName('muted')
        layout.addWidget(self.shortcut_draft_status)
        controls = QHBoxLayout()
        self.shortcut_restore = QPushButton('全部恢复默认')
        self.shortcut_restore.clicked.connect(self.restore_shortcut_defaults)
        controls.addWidget(self.shortcut_restore)
        controls.addStretch()
        self.shortcut_cancel = QPushButton('撤销修改')
        self.shortcut_cancel.clicked.connect(self.cancel_shortcut_changes)
        controls.addWidget(self.shortcut_cancel)
        self.shortcut_apply = QPushButton('应用快捷键')
        self.shortcut_apply.setObjectName('primary')
        self.shortcut_apply.clicked.connect(self.apply_shortcut_changes)
        controls.addWidget(self.shortcut_apply)
        layout.addLayout(controls)
        self.validate_shortcut_draft()
        return page

    def shortcut_draft(self):
        return {key: editor.keySequence().toString(QKeySequence.PortableText) for key, editor in self.shortcut_editors.items()}

    def validate_shortcut_draft(self, *_args):
        if not hasattr(self, 'shortcut_apply'):
            return
        values, errors = validate(self.shortcut_draft())
        for key, editor in self.shortcut_editors.items():
            invalid = key in errors
            if editor.property('invalid') != invalid:
                editor.setProperty('invalid', invalid)
                editor.style().unpolish(editor)
                editor.style().polish(editor)
            editor.setToolTip(errors.get(key) or '点击后按下新组合；清空可禁用')
        dirty = values != self.shortcut_manager.values or bool(self.shortcut_manager.load_error)
        self.shortcut_apply.setEnabled(dirty and not errors)
        self.shortcut_cancel.setEnabled(dirty)
        messages = [title + '：' + errors[key] for key, title, _default in SPECS if key in errors]
        self.shortcut_error.setText('\n'.join(messages) if messages else (self.shortcut_manager.load_error or ''))
        self.shortcut_error.setVisible(bool(messages or self.shortcut_manager.load_error))
        self.shortcut_draft_status.setText('修改尚未应用，当前快捷键继续生效' if dirty else '当前使用已保存的设置')

    def restore_shortcut_defaults(self):
        for key, editor in self.shortcut_editors.items():
            editor.setKeySequence(QKeySequence(DEFAULTS[key]))
        self.validate_shortcut_draft()

    def cancel_shortcut_changes(self):
        for key, editor in self.shortcut_editors.items():
            editor.setKeySequence(QKeySequence(self.shortcut_manager.values[key]))
        self.validate_shortcut_draft()

    def apply_shortcut_changes(self):
        errors = self.shortcut_manager.save(self.shortcut_draft())
        if errors:
            self.shortcut_error.setText('；'.join(errors.values()))
            self.shortcut_error.show()
            return
        self.update_shortcut_hints()
        self.validate_shortcut_draft()
        self.notify('快捷键已应用并保存')

    def update_shortcut_hints(self):
        for index, action in enumerate(['overview', 'services', 'resources', 'git', 'agents', 'logs', 'settings']):
            self.nav[index].setToolTip(self.nav[index].text() + ' · ' + self.shortcut_manager.display(action))
        self.refresh_button.setToolTip('刷新服务状态 · ' + self.shortcut_manager.display('refresh'))
        self.add_service_button.setToolTip('添加服务 · ' + self.shortcut_manager.display('new_service'))
        self.compact_button.setToolTip('切换大小面板 · ' + self.shortcut_manager.display('toggle_mode'))

    def run_shortcut(self, action, window):
        quick = window is getattr(self, 'quick_menu', None)
        pages = {'overview': 0, 'services': 1, 'resources': 2, 'git': 3, 'agents': 4, 'logs': 5, 'settings': 6}
        if action in pages:
            if quick:
                self.present('full', page=pages[action])
            else:
                self.navigate(pages[action])
        elif action == 'hide':
            window.hide() if quick else self.hide_panel()
        elif action == 'toggle_mode':
            self.present('full' if quick else 'small', remember=True)
        elif action in ('search', 'projects'):
            target = (window.search if quick else self.project_search)
            if not quick and action == 'search':
                target = {3: self.git_search, 4: self.skills_search, 5: self.log_search}.get(self.page, self.project_search)
            target.setFocus(Qt.ShortcutFocusReason)
            target.selectAll()
        elif action == 'refresh':
            self.refresh(manual=True)
        elif action == 'new_service':
            self.add_service()
        elif action == 'service_logs':
            target = self.log_service.currentData() if self.page == 5 and not quick else self.selected
            if target:
                self.open_logs(target)
            if quick:
                self.present('full', page=5)
        elif action == 'pause_logs':
            if not quick and self.page == 5:
                self.toggle_pause()
            else:
                self.notify('暂停／继续刷新仅用于日志中心')
        elif action == 'help':
            self.show_shortcut_help(window)

    def show_shortcut_help(self, parent=None):
        dialog = QDialog(parent or self)
        dialog.setWindowTitle('devtray · 快捷键说明')
        dialog.setStyleSheet(self.styleSheet())
        dialog.resize(620, 640)
        layout = QVBoxLayout(dialog)
        layout.addWidget(text('快捷键仅在当前 devtray 窗口内生效。可到「工作台设置 → 快捷键」修改或禁用。'))
        table = QTableWidget(len(SPECS), 2)
        table.setHorizontalHeaderLabels(['操作', '当前快捷键'])
        table.verticalHeader().setVisible(False)
        table.setEditTriggers(QAbstractItemView.NoEditTriggers)
        table.horizontalHeader().setSectionResizeMode(0, QHeaderView.Stretch)
        table.setColumnWidth(1, 190)
        for index, (key, title, _default) in enumerate(SPECS):
            table.setItem(index, 0, QTableWidgetItem(title))
            table.setItem(index, 1, QTableWidgetItem(self.shortcut_manager.display(key)))
        layout.addWidget(table, 1)
        close = QDialogButtonBox(QDialogButtonBox.Close)
        close.rejected.connect(dialog.reject)
        layout.addWidget(close)
        dialog.exec()
