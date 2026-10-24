"""devtray desktop workspace. Embedded by Rust; PySide6 is an optional runtime dependency."""
import hashlib
import json
import os
import sys
from datetime import datetime
from pathlib import Path

from PySide6.QtCore import Qt, QTimer, QProcess, QSettings, QUrl, QLockFile, QDir, QEvent
from PySide6.QtGui import QColor, QFont, QSyntaxHighlighter, QTextCharFormat, QDesktopServices, QKeySequence, QShortcut, QIcon
from quick_menu import QuickMenu

from PySide6.QtNetwork import QLocalServer, QLocalSocket
from PySide6.QtWidgets import (
    QApplication, QMainWindow, QWidget, QFrame, QLabel, QPushButton, QLineEdit,
    QVBoxLayout, QHBoxLayout, QGridLayout, QStackedWidget, QTreeWidget, QTreeWidgetItem,
    QScrollArea, QPlainTextEdit, QComboBox, QCheckBox, QFileDialog, QMessageBox,
    QDialog, QDialogButtonBox, QFormLayout, QSplitter, QSizePolicy,
)

STYLE = """
* { font-family: "Noto Sans CJK SC", "Noto Sans", sans-serif; font-size: 13px; }
QMainWindow, QDialog { background: #f4f5f1; color: #263a35; }
QWidget { color: #263a35; }
QFrame#sidebar { background: #1d302b; border: none; }
QFrame#sidebar QLabel { color: #dce7e0; }
QLabel#brand { font-size: 27px; font-weight: 800; letter-spacing: -1px; }
QLabel#eyebrow { color: #8fa99d; font-size: 10px; letter-spacing: 2px; }
QLabel#sideCaption { color: #92a79c; font-size: 11px; }
QPushButton { background: #ffffff; border: 1px solid #dbe1d9; border-radius: 7px; padding: 8px 13px; color: #354b42; }
QPushButton:hover { background: #edf3eb; border-color: #9eb8a8; }
QPushButton:pressed { background: #dce9dc; }
QPushButton:disabled { color: #a5afa9; background: #f1f3ef; border-color: #e5e8e1; }
QPushButton#primary { background: #285e48; color: white; border-color: #285e48; font-weight: 600; }
QPushButton#primary:hover { background: #34785c; }
QPushButton#danger { color: #af5045; }
QPushButton#nav { background: transparent; border: none; color: #aebfb4; text-align: left; padding: 11px 16px; font-weight: 500; }
QPushButton#nav:checked { background: #344b3e; color: #eef6dd; border-left: 3px solid #bed492; border-radius: 5px; }
QPushButton#nav:hover { background: #2a4035; color: #ffffff; }
QTreeWidget { background: transparent; border: none; outline: none; color: #c3d0c6; }
QTreeWidget::item { padding: 7px 3px; border-radius: 5px; }
QTreeWidget::item:selected { background: #344b3e; color: #eff5e8; }
QTreeWidget::item:hover { background: #2a4035; }
QFrame#card { background: #ffffff; border: 1px solid #dfe5dc; border-radius: 11px; }
QFrame#hero { background: #e9eee1; border: 1px solid #dbe3d1; border-radius: 12px; }
QLabel#title { font-size: 27px; font-weight: 700; letter-spacing: -1px; }
QLabel#sectionTitle { font-size: 16px; font-weight: 700; }
QLabel#muted { color: #819087; font-size: 12px; }
QLabel#number { font-size: 31px; font-weight: 650; }
QLabel#mono { font-family: "JetBrains Mono", "DejaVu Sans Mono", monospace; font-size: 11px; color: #718078; }
QLabel#badge { border-radius: 5px; padding: 3px 7px; font-size: 11px; }
QLabel#errorBanner { background: #fff0e7; color: #9f482d; padding: 10px; border-radius: 7px; }
QLineEdit, QComboBox { background: #ffffff; border: 1px solid #d7dfd5; border-radius: 7px; padding: 8px 10px; selection-background-color: #b7d0bc; }
QLineEdit:focus, QComboBox:focus { border-color: #50795c; }
QComboBox::drop-down { border: none; width: 22px; }
QComboBox QAbstractItemView { background: white; color: #263a35; selection-background-color: #e1ebdd; }
QPlainTextEdit { background: #fcfdfa; border: 1px solid #dfe5dc; border-radius: 8px; padding: 10px; selection-background-color: #c4dcc4; selection-color: #1c3429; }
QPlainTextEdit#logView { background: #172721; color: #cbd8ce; border: 1px solid #263d31; border-radius: 9px; font-family: "JetBrains Mono", "DejaVu Sans Mono", monospace; font-size: 12px; padding: 16px; selection-background-color: #476c56; selection-color: #ffffff; }
QScrollArea { border: none; background: transparent; }
QScrollArea > QWidget > QWidget { background: transparent; }
QScrollBar:vertical { width: 8px; background: transparent; margin: 2px; }
QScrollBar::handle:vertical { background: #bdcabe; border-radius: 3px; min-height: 24px; }
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical { height: 0; }
QSplitter::handle { background: transparent; }
QCheckBox { spacing: 7px; }
QCheckBox::indicator { width: 15px; height: 15px; border: 1px solid #b7c5b6; border-radius: 4px; background: #fff; }
QCheckBox::indicator:checked { background: #427456; border-color: #427456; }
QToolTip { background: #263d31; color: white; padding: 6px; border: none; }
"""

STATES = {
    'ready': ('就绪', '#3c8060', '#edf5eb'),
    'running': ('运行中', '#3c8060', '#edf5eb'),
    'starting': ('启动中', '#b48027', '#fcf4df'),
    'exited_failed': ('异常退出', '#b3584e', '#f9eeea'),
    'not_started': ('未启动', '#8a9790', '#f0f2ed'),
    'exited_ok': ('已结束', '#8a9790', '#f0f2ed'),
    'exited_signal': ('已停止', '#8a9790', '#f0f2ed'),
}
LIVE = {'ready', 'running', 'starting'}


def label(text, name=None, wrap=False):
    w = QLabel(text)
    w.setTextFormat(Qt.PlainText)
    if name:
        w.setObjectName(name)
    w.setWordWrap(wrap)
    return w


def button(text, callback, name=None):
    b = QPushButton(text)
    if name:
        b.setObjectName(name)
    b.setCursor(Qt.PointingHandCursor)
    b.clicked.connect(callback)
    return b


def card(name='card'):
    box = QFrame()
    box.setObjectName(name)
    layout = QVBoxLayout(box)
    layout.setContentsMargins(20, 18, 20, 18)
    layout.setSpacing(12)
    return box, layout


def row(*widgets):
    layout = QHBoxLayout()
    layout.setSpacing(9)
    for w in widgets:
        if w is None:
            layout.addStretch()
        else:
            layout.addWidget(w)
    return layout


def clear_layout(layout):
    while layout.count():
        item = layout.takeAt(0)
        if item.widget():
            item.widget().deleteLater()
        elif item.layout():
            clear_layout(item.layout())


class LogEditor(QPlainTextEdit):
    """Freeze before mouse dragging; polling never replaces an active selection."""
    def __init__(self, on_read):
        super().__init__()
        self.on_read = on_read
        self.setReadOnly(True)
        self.setObjectName('logView')
        self.setLineWrapMode(QPlainTextEdit.NoWrap)

    def mousePressEvent(self, event):
        if event.button() == Qt.LeftButton:
            self.on_read()
        super().mousePressEvent(event)

    def wheelEvent(self, event):
        if event.angleDelta().y() > 0:
            self.on_read()
        super().wheelEvent(event)

    def keyPressEvent(self, event):
        if event.key() in (Qt.Key_PageUp, Qt.Key_Home, Qt.Key_Up) or event.matches(QKeySequence.SelectAll):
            self.on_read()
        super().keyPressEvent(event)


class LogColors(QSyntaxHighlighter):
    def __init__(self, document):
        super().__init__(document)
        self.levels = []

    def highlightBlock(self, text):
        i = self.currentBlock().blockNumber()
        severity = self.levels[i] if i < len(self.levels) else 'other'
        color = {'error': '#eea79a', 'warn': '#e1c788', 'info': '#a9ceaf', 'debug': '#8da9c0', 'trace': '#879c91'}.get(severity, '#b2c0b7')
        fmt = QTextCharFormat()
        fmt.setForeground(QColor(color))
        self.setFormat(0, len(text), fmt)


class ServiceDialog(QDialog):
    def __init__(self, parent, fields=None):
        super().__init__(parent)
        self.fields = fields or {}
        self.editing = fields is not None
        self.setWindowTitle('编辑服务' if self.editing else '添加服务')
        self.setMinimumWidth(610)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(28, 24, 28, 24)
        layout.addWidget(label(self.windowTitle(), 'title'))
        layout.addWidget(label('将工作目录和启动命令交给 devtray 管理。', 'muted'))
        form = QFormLayout()
        form.setSpacing(12)
        self.inputs = {}
        for key, title, hint in [
            ('project', '项目名称', 'my-project'), ('service', '服务名称', 'backend'),
            ('cwd', '工作目录', '~/Workspace/my-project'), ('cmd', '启动命令', 'cargo run / bun run dev'),
            ('port', '就绪端口', '可选，如 3000'), ('ready', '就绪日志正则', '可选，如 Listening on'),
            ('url', '预览地址', '可选，如 http://localhost:3000'),
        ]:
            edit = QLineEdit(str(self.fields.get(key, '')))
            edit.setPlaceholderText(hint)
            if self.editing and key in ('project', 'service'):
                edit.setReadOnly(True)
            self.inputs[key] = edit
            if key == 'cwd':
                form.addRow(title, row(edit, button('选择…', self.browse)))
            else:
                form.addRow(title, edit)
        self.env = QPlainTextEdit()
        self.env.setPlaceholderText('RUST_LOG=info\nNODE_ENV=development')
        self.env.setPlainText('\n'.join(f'{k}={v}' for k, v in self.fields.get('env', [])))
        self.env.setMaximumHeight(95)
        form.addRow('环境变量', self.env)
        self.autostart = QCheckBox('托盘启动时自动启动此服务')
        self.autostart.setChecked(self.fields.get('autostart', False))
        form.addRow('', self.autostart)
        layout.addLayout(form)
        self.error = label('', 'errorBanner', True)
        self.error.hide()
        layout.addWidget(self.error)
        self.buttons = QDialogButtonBox(QDialogButtonBox.Save | QDialogButtonBox.Cancel)
        self.buttons.button(QDialogButtonBox.Save).setText('保存服务')
        self.buttons.button(QDialogButtonBox.Cancel).setText('取消')
        self.buttons.accepted.connect(self.save)
        self.buttons.rejected.connect(self.reject)
        layout.addWidget(self.buttons)

    def browse(self):
        path = QFileDialog.getExistingDirectory(self, '选择工作目录', os.path.expanduser(self.inputs['cwd'].text()) or str(Path.home()))
        if path:
            self.inputs['cwd'].setText(path)

    def save(self):
        fields = {k: w.text().strip() for k, w in self.inputs.items()}
        env = []
        for line in self.env.toPlainText().splitlines():
            if not line.strip():
                continue
            if '=' not in line or not line.split('=', 1)[0].strip():
                self.fail('环境变量请使用 KEY=VALUE，每行一个。')
                return
            k, v = line.split('=', 1)
            env.append([k.strip(), v])
        fields.update(autostart=self.autostart.isChecked(), env=env)
        self.buttons.setEnabled(False)
        def done(data, error):
            self.buttons.setEnabled(True)
            if error:
                self.fail(error)
            else:
                self.parent().notify(data['message'])
                self.accept()
                self.parent().refresh()
        self.parent().request(['_save-service'], done, {'mode': 'edit' if self.editing else 'add', 'fields': fields})

    def fail(self, text):
        self.error.setText(text)
        self.error.show()


class Panel(QMainWindow):
    def __init__(self, binary, config, poll=True):
        super().__init__()
        self.binary, self.config = binary, config
        digest = hashlib.sha256(str(config).encode()).hexdigest()[:16]
        self.settings = QSettings('devtray', 'panel-' + digest)
        self.data = {'services': []}
        self.processes = set()
        self.refreshing = self.log_loading = False
        self.project = None
        self.selected = None
        self.page = 0
        self.poll_enabled = poll
        self.tree_signature = None
        self.card_signature = None
        self.card_columns = 2
        self.read_revision = 0
        self.log_generation = 0
        self.log_text = None
        self.log_entries = []
        self.log_filter_key = None
        self.action_pending = set()
        self.bulk_buttons = []
        self.setWindowTitle('devtray · 开发工作台')
        self.resize(1340, 880)
        self.setMinimumSize(1040, 720)
        self.setStyleSheet(STYLE)
        central = QWidget()
        self.setCentralWidget(central)
        shell = QHBoxLayout(central)
        shell.setContentsMargins(0, 0, 0, 0)
        shell.setSpacing(0)
        self.sidebar = QFrame()
        self.sidebar.setObjectName('sidebar')
        self.sidebar.setFixedWidth(234)
        side = QVBoxLayout(self.sidebar)
        side.setContentsMargins(20, 27, 18, 21)
        side.setSpacing(8)
        side.addWidget(label('▦  devtray', 'brand'))
        side.addWidget(label('LOCAL DEVELOPMENT DESK', 'eyebrow'))
        side.addSpacing(30)
        self.nav = []
        for index, title in enumerate(['▦   总览', '▤   服务管理', '≡   日志中心', '⚙   工作台设置']):
            b = button(title, lambda checked=False, i=index: self.navigate(i), 'nav')
            b.setCheckable(True)
            side.addWidget(b)
            self.nav.append(b)
        side.addSpacing(26)
        side.addLayout(row(label('工作目录', 'sideCaption'), None, button('+', self.add_service, 'nav')))
        self.project_search = QLineEdit()
        self.project_search.setPlaceholderText('筛选项目…')
        self.project_search.setStyleSheet('background:#263d33; color:#dce7e0; border:1px solid #3a5042;')
        self.project_search.textChanged.connect(self.filter_tree)
        side.addWidget(self.project_search)
        self.tree = QTreeWidget()
        self.tree.setHeaderHidden(True)
        self.tree.setIndentation(12)
        self.tree.itemClicked.connect(self.select_tree)
        side.addWidget(self.tree, 1)
        side.addWidget(label('●  本机工作空间', 'sideCaption'))
        side.addWidget(label('窗口收回后，服务继续运行', 'sideCaption'))
        shell.addWidget(self.sidebar)
        main = QWidget()
        body = QVBoxLayout(main)
        body.setContentsMargins(30, 24, 30, 18)
        body.setSpacing(18)
        self.breadcrumb = label('工作台 / 总览', 'muted')
        self.compact_button = button('切换小面板', lambda: self.present('small', remember=True))
        body.addLayout(row(self.breadcrumb, None, self.compact_button, button('↻  刷新', self.refresh), button('＋ 添加服务', self.add_service, 'primary'), button('收回 ↘', self.hide_panel)))
        self.banner = label('', 'errorBanner', True)
        self.banner.hide()
        body.addWidget(self.banner)
        self.pages = QStackedWidget()
        body.addWidget(self.pages, 1)
        shell.addWidget(main, 1)
        self.build_overview()
        self.build_services()
        self.build_logs()
        self.build_settings()
        self.footer = label('正在连接本机服务…', 'muted')
        body.addLayout(row(self.footer, None, label('DEVTRAY  /  01', 'mono')))
        self.navigate(0)
        self.timer = QTimer(self)
        self.timer.timeout.connect(self.refresh)
        if poll:
            self.timer.start(2000)
            QTimer.singleShot(0, self.refresh)
        QShortcut(QKeySequence('Escape'), self, self.hide_panel)
        QShortcut(QKeySequence('Ctrl+L'), self, lambda: self.navigate(2))
        QShortcut(QKeySequence('Ctrl+R'), self, self.refresh)
        geometry = self.settings.value('geometry')
        if geometry:
            self.restoreGeometry(geometry)

    def build_overview(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(18)
        hero, h = card('hero')
        h.addWidget(label('让每个项目，都有条不紊。', 'title'))
        self.hero_caption = label('本机服务总览 · 启动、检查、回到工作。', 'muted')
        h.addWidget(self.hero_caption)
        h.addLayout(row(label('WORKSPACE OVERVIEW', 'mono'), None, self.bulk_button('start'), self.bulk_button('stop')))
        layout.addWidget(hero)
        metrics = QHBoxLayout()
        metrics.setSpacing(12)
        self.metrics = []
        for title, color, subtitle in [('工作项目', '#314c3c', '统一管理工作目录'), ('运行服务', '#428561', '就绪 / 运行中'), ('需要关注', '#b78332', '启动中 / 异常退出'), ('已停止', '#8a978d', '随时启动，继续工作')]:
            box, stack = card()
            stack.addWidget(label(title, 'muted'))
            number = label('—', 'number')
            number.setStyleSheet('color:' + color)
            stack.addWidget(number)
            stack.addWidget(label(subtitle, 'muted'))
            metrics.addWidget(box)
            self.metrics.append(number)
        layout.addLayout(metrics)
        split = QSplitter(Qt.Horizontal)
        left = QWidget()
        l = QVBoxLayout(left)
        l.setContentsMargins(0, 0, 8, 0)
        l.setSpacing(12)
        self.cards_title = label('服务一览', 'sectionTitle')
        l.addLayout(row(self.cards_title, None, button('查看全部 →', lambda: self.navigate(1))))
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        content = QWidget()
        self.cards_viewport = scroll.viewport()
        self.cards_viewport.installEventFilter(self)
        self.cards = QGridLayout(content)
        self.cards.setContentsMargins(0, 0, 0, 0)
        self.cards.setSpacing(12)
        self.cards.setAlignment(Qt.AlignTop)
        scroll.setWidget(content)
        l.addWidget(scroll)
        split.addWidget(left)
        self.inspector, self.inspector_layout = card()
        self.inspector.setMinimumWidth(245)
        inspector_scroll = QScrollArea()
        inspector_scroll.setWidgetResizable(True)
        inspector_scroll.setMinimumWidth(280)
        inspector_scroll.setWidget(self.inspector)
        split.addWidget(inspector_scroll)
        split.setSizes([640, 280])
        layout.addWidget(split, 1)
        self.pages.addWidget(page)

    def bulk_button(self, op, style=None):
        b = button({'start': '启动全部', 'stop': '停止全部', 'restart': '重启全部'}[op], lambda: self.bulk(op), style)
        self.bulk_buttons.append((b, op))
        return b

    def eventFilter(self, watched, event):
        if watched is getattr(self, 'cards_viewport', None) and event.type() == QEvent.Resize:
            columns = 2 if watched.width() >= 570 else 1
            if columns != self.card_columns:
                self.card_columns = columns
                QTimer.singleShot(0, self.render_services)
        return super().eventFilter(watched, event)

    def build_services(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(label('服务管理', 'title'))
        layout.addWidget(label('按项目查看命令、就绪状态和工作目录。', 'muted'))
        controls = row(self.bulk_button('start', 'primary'), self.bulk_button('restart'), self.bulk_button('stop'), None)
        layout.addLayout(controls)
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        content = QWidget()
        self.services_layout = QVBoxLayout(content)
        self.services_layout.setAlignment(Qt.AlignTop)
        self.services_layout.setContentsMargins(0, 12, 0, 0)
        self.services_layout.setSpacing(12)
        scroll.setWidget(content)
        layout.addWidget(scroll)
        self.pages.addWidget(page)

    def build_logs(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(13)
        layout.addWidget(label('日志中心', 'title'))
        layout.addWidget(label('这里显示可筛选的文件日志；原始颜色、进度与交互输出请打开实时终端。', 'muted'))
        controls, c = card()
        self.log_service = QComboBox()
        self.log_service.setMinimumWidth(230)
        self.log_service.currentIndexChanged.connect(self.log_changed)
        self.log_source = QComboBox()
        self.log_source.addItem('当前日志', '0')
        self.log_source.addItem('所有保留日志', 'all')
        self.log_source.currentIndexChanged.connect(self.log_changed)
        self.pause = button('Ⅱ  暂停刷新', self.toggle_pause)
        self.paused = False
        self.log_terminal = button('实时终端', lambda: self.open_terminal(self.log_service.currentData()))
        self.log_terminal.setToolTip('打开选中服务的 tmux 原始终端，不受日志筛选影响')
        c.addLayout(row(self.log_service, self.log_source, None, self.log_terminal, self.pause))
        self.level_buttons = {}
        filters = QHBoxLayout()
        for name, text in [('error', 'ERROR'), ('warn', 'WARN'), ('info', 'INFO'), ('debug', 'DEBUG'), ('trace', 'TRACE'), ('other', '其他')]:
            b = QPushButton(text)
            b.setCheckable(True)
            b.setChecked(name in str(self.settings.value('levels', 'error,warn,info,debug,trace,other')).split(','))
            b.clicked.connect(self.log_changed)
            self.level_buttons[name] = b
            filters.addWidget(b)
        self.log_search = QLineEdit(str(self.settings.value('search', '')))
        self.log_search.setPlaceholderText('搜索日志内容…')
        self.search_timer = QTimer(self)
        self.search_timer.setSingleShot(True)
        self.search_timer.setInterval(300)
        self.search_timer.timeout.connect(self.log_changed)
        self.log_search.textChanged.connect(lambda: self.search_timer.start())
        filters.addWidget(self.log_search, 1)
        c.addLayout(filters)
        for b in self.level_buttons.values():
            b.setStyleSheet('QPushButton:checked { background:#e4eddb; border-color:#99b583; color:#36582b; }')
        layout.addWidget(controls)
        self.log_view = LogEditor(self.freeze_logs)
        self.log_view.setPlaceholderText('请选择一个服务。尚未启动的服务没有日志。')
        self.highlighter = LogColors(self.log_view.document())
        self.log_view.verticalScrollBar().sliderPressed.connect(self.freeze_logs)
        layout.addWidget(self.log_view, 1)
        self.log_summary = label('等待日志', 'muted')
        layout.addLayout(row(self.log_summary, None, button('复制所选', self.copy_selected), button('导出筛选结果', self.export_logs)))
        self.pages.addWidget(page)

    def build_settings(self):
        page = QWidget()
        self.settings_layout = QVBoxLayout(page)
        self.settings_layout.setContentsMargins(0, 0, 0, 0)
        self.settings_layout.setAlignment(Qt.AlignTop)
        self.pages.addWidget(page)

    def request(self, args, callback, payload=None, raw=False):
        process = QProcess(self)
        from PySide6.QtCore import QProcessEnvironment
        env = QProcessEnvironment.systemEnvironment()
        env.insert('DEVTRAY_CONFIG', str(self.config))
        process.setProcessEnvironment(env)
        process.setProgram(self.binary)
        process.setArguments(args)
        self.processes.add(process)
        finished = False
        timeout = QTimer(process)
        timeout.setSingleShot(True)
        def complete(error=None):
            nonlocal finished
            if finished:
                return
            finished = True
            timeout.stop()
            stdout = bytes(process.readAllStandardOutput()).decode('utf-8', 'replace')
            stderr = bytes(process.readAllStandardError()).decode('utf-8', 'replace')
            if not error and process.exitCode() != 0:
                error = stderr.strip() or f'命令失败（{process.exitCode()}）'
            data = stdout
            if not error and not raw:
                try:
                    data = json.loads(stdout)
                except ValueError as e:
                    error = '无法读取服务响应：' + str(e)
            self.processes.discard(process)
            process.deleteLater()
            callback(data, error)
        def timed_out():
            process.kill()
            complete('操作超过 30 秒，请检查服务后重试。')
        process.finished.connect(lambda *_: complete())
        process.errorOccurred.connect(lambda e: complete(process.errorString()) if e == QProcess.FailedToStart else None)
        timeout.timeout.connect(timed_out)
        process.start()
        timeout.start(30000)
        if payload is not None:
            process.write(json.dumps(payload).encode())
            process.closeWriteChannel()

    def refresh(self):
        if self.refreshing:
            return
        self.refreshing = True
        def done(data, error):
            self.refreshing = False
            if error:
                self.notify('状态刷新失败，保留上次结果：' + error, error=True)
                self.footer.setText('连接异常 · 正在重试')
                return
            self.apply_data(data)
        self.request(['_panel-data'], done)
        if self.page == 2:
            self.refresh_logs()

    def apply_data(self, data):
        self.data = data
        if hasattr(self, 'quick_menu'):
            self.quick_menu.sync(data)
        services = data['services']
        projects = sorted({s['project'] for s in services})
        if self.project not in projects:
            self.project = None
        names = {s['name'] for s in services}
        if self.selected not in names:
            self.selected = services[0]['name'] if services else None
        vals = [len(projects), sum(s['state'] in ('running', 'ready') for s in services), sum(s['state'] in ('starting', 'exited_failed') for s in services), sum(s['state'] not in LIVE and s['state'] != 'exited_failed' for s in services)]
        for w, v in zip(self.metrics, vals):
            w.setText(str(v).zfill(2))
        self.hero_caption.setText(f'{len(projects)} 个项目，{len(services)} 项服务 · ' + ('有服务需要关注，查看状态了解详情。' if vals[2] else '工作空间已就位。'))
        signature = [(s['name'], s['cwd']) for s in services]
        if signature != self.tree_signature:
            self.tree_signature = signature
            self.tree.clear()
            all_item = QTreeWidgetItem(['全部项目'])
            all_item.setData(0, Qt.UserRole, ('all', ''))
            self.tree.addTopLevelItem(all_item)
            for project in projects:
                paths = [os.path.expanduser(s['cwd']) for s in services if s['project'] == project]
                root = os.path.commonpath(paths) if paths and all(os.path.isabs(p) for p in paths) else ''
                short_root = root.replace(str(Path.home()), '~', 1)
                item = QTreeWidgetItem([project + ('\n' + short_root if short_root else '')])
                item.setToolTip(0, root)
                item.setData(0, Qt.UserRole, ('project', project))
                self.tree.addTopLevelItem(item)
                for s in services:
                    if s['project'] == project:
                        child = QTreeWidgetItem([s['service']])
                        child.setToolTip(0, s['cwd'])
                        child.setData(0, Qt.UserRole, ('service', s['name']))
                        item.addChild(child)
                item.setExpanded(True)
            self.filter_tree()
            previous = self.log_service.currentData() or self.settings.value('log_service')
            self.log_service.blockSignals(True)
            self.log_service.clear()
            for s in services:
                self.log_service.addItem(s['name'], s['name'])
            index = self.log_service.findData(previous)
            if index >= 0:
                self.log_service.setCurrentIndex(index)
            self.log_service.blockSignals(False)
            self.log_changed()
        self.render_services()
        self.render_settings()
        self.update_log_terminal()
        self.footer.setText(f'● 状态已同步  {datetime.now():%H:%M:%S}    ·    {len(services)} 项服务')

    def filtered_services(self):
        return [s for s in self.data['services'] if self.project is None or s['project'] == self.project]

    def render_services(self):
        if hasattr(self, 'quick_menu'):
            self.quick_menu.sync(self.data)
        services = self.filtered_services()
        for b, op in self.bulk_buttons:
            b.setText({'start': '启动', 'stop': '停止', 'restart': '重启'}[op] + ('当前项目' if self.project else '全部'))
        signature = json.dumps([services, self.selected, sorted(self.action_pending), self.card_columns], sort_keys=True)
        if signature == self.card_signature:
            return
        self.card_signature = signature
        self.cards_title.setText((self.project or '全部项目') + ' / 服务一览')
        clear_layout(self.cards)
        clear_layout(self.services_layout)
        if not services:
            empty, l = card()
            l.addWidget(label('工作台，等待第一个项目。', 'sectionTitle'))
            l.addWidget(label('添加工作目录与启动命令，即可在这里管理服务。', 'muted', True))
            l.addWidget(button('＋ 添加第一个服务', self.add_service, 'primary'))
            self.cards.addWidget(empty, 0, 0, 1, 2)
            self.services_layout.addWidget(label('还没有服务。点击右上角「添加服务」开始。', 'muted'))
        for i, s in enumerate(services):
            box, l = self.service_card(s)
            self.cards.addWidget(box, i // self.card_columns, i % self.card_columns)
            wide, w = self.service_card(s, wide=True)
            self.services_layout.addWidget(wide)
        self.cards.setColumnStretch(0, 1)
        self.cards.setColumnStretch(1, 1 if self.card_columns == 2 else 0)
        self.render_inspector()

    def service_card(self, s, wide=False):
        box, l = card()
        state, color, bg = STATES.get(s['state'], STATES['not_started'])
        badge = label('●  ' + state, 'badge')
        badge.setStyleSheet(f'color:{color};background:{bg};')
        name = button(s['service'], lambda: self.inspect(s['name']))
        name.setStyleSheet('border:none; padding:0; text-align:left; font-size:16px; font-weight:700; background:transparent;')
        l.addLayout(row(name, None, badge))
        l.addWidget(label(s['project'], 'muted'))
        cwd = label(s['cwd'], 'mono', True)
        cwd.setTextInteractionFlags(Qt.TextSelectableByMouse)
        l.addWidget(cwd)
        command = label('›  ' + s['cmd'], 'mono', True)
        command.setTextInteractionFlags(Qt.TextSelectableByMouse)
        l.addWidget(command)
        actions = QHBoxLayout()
        op = 'stop' if s['state'] in LIVE else 'start'
        action = button('■ 停止' if op == 'stop' else '▶ 启动', lambda: self.action(op, s['name']))
        action.setEnabled(s['name'] not in self.action_pending)
        restart = button('↻', lambda: self.action('restart', s['name']))
        restart.setToolTip('重启服务')
        restart.setEnabled(s['name'] not in self.action_pending)
        actions.addWidget(action)
        actions.addWidget(restart)
        actions.addStretch()
        if wide:
            actions.addWidget(button('编辑', lambda: self.edit_service(s['name'])))
        actions.addWidget(button('分类日志 →', lambda: self.open_logs(s['name'])))
        l.addLayout(actions)
        terminal = button('实时终端', lambda: self.open_terminal(s['name']))
        terminal.setEnabled(s['state'] != 'not_started')
        terminal.setToolTip('打开 tmux 原始屏幕、实时输出与交互；已退出的会话也可查看')
        preview = button('浏览器 ↗', lambda: self.open_preview(s))
        preview.setEnabled(bool(s.get('url')))
        preview.setToolTip(s.get('url') or '尚无预览地址，请在服务配置中填写 URL 或端口')
        l.addLayout(row(terminal, preview, None))
        return box, l

    def render_inspector(self):
        clear_layout(self.inspector_layout)
        l = self.inspector_layout
        s = next((s for s in self.data['services'] if s['name'] == self.selected), None)
        l.addWidget(label('工作目录 / 结构', 'sectionTitle'))
        if not s:
            l.addWidget(label('选择服务，查看它的运行结构。', 'muted', True))
            l.addStretch()
            return
        l.addWidget(label(s['project'], 'sectionTitle', True))
        for title, value in [('└─ ' + s['service'], s['cwd']), ('启动命令', s['cmd']), ('就绪探测', (s.get('ready') or '') + (f" · TCP {s['port']}" if s.get('port') else '') or '进程存活'), ('预览地址', s.get('url') or '未配置'), ('日志归档', s['log'])]:
            l.addWidget(label(title, 'muted'))
            text = label(value, 'mono', True)
            text.setTextInteractionFlags(Qt.TextSelectableByMouse)
            l.addWidget(text)
        l.addWidget(label('● 自动启动' if s['autostart'] else '○ 手动启动', 'muted'))
        l.addStretch()
        l.addWidget(button('打开工作目录 ↗', lambda: QDesktopServices.openUrl(QUrl.fromLocalFile(os.path.expanduser(s['cwd'])))))
        l.addWidget(button('编辑服务配置', lambda: self.edit_service(s['name'])))

    def render_settings(self):
        clear_layout(self.settings_layout)
        l = self.settings_layout
        l.addWidget(label('工作台设置', 'title'))
        for title, text in [
            ('日志留存', '每次启动归档上一份日志；持续输出达到 10 MiB 时轮转。每项服务保留当前文件和最近 10 份归档（约 110 MiB 上限）。'),
            ('窗口与快捷键', '左键展开 / 收回上次选择的面板；右键打开系统菜单。小面板底部与完整面板顶栏可切换模式，切换后自动记忆。实时终端显示 tmux 原始输出，分类日志支持筛选和历史。Esc 收回；Ctrl+L 打开日志，Ctrl+R 刷新状态。'),
            ('配置文件', self.data.get('config', str(self.config))),
            ('日志目录', self.data.get('logs_dir', '正在读取…')),
            ('工作台模块', '总览、服务管理与日志中心共享同一套服务配置。后续可继续增加数据库、任务和其他资源的管理页面。'),
        ]:
            box, b = card()
            b.addWidget(label(title, 'sectionTitle'))
            text_label = label(text, 'muted', True)
            text_label.setTextInteractionFlags(Qt.TextSelectableByMouse)
            b.addWidget(text_label)
            if title == '配置文件':
                b.addWidget(button('打开配置所在目录 ↗', lambda: QDesktopServices.openUrl(QUrl.fromLocalFile(str(Path(self.config).parent)))))
            l.addWidget(box)
        if not self.data.get('tmux_available', True):
            l.addWidget(label('未检测到 tmux，请安装后再启动服务。', 'errorBanner'))

    def filter_tree(self):
        query = self.project_search.text().lower()
        for i in range(self.tree.topLevelItemCount()):
            item = self.tree.topLevelItem(i)
            parent_matches = query in item.text(0).lower()
            matches = parent_matches
            for j in range(item.childCount()):
                child = item.child(j)
                show = parent_matches or query in child.text(0).lower() or query in child.toolTip(0).lower()
                child.setHidden(not show)
                matches = matches or show
            item.setHidden(not matches and bool(query))

    def select_tree(self, item):
        kind, value = item.data(0, Qt.UserRole)
        if kind == 'all':
            self.project = None
        elif kind == 'project':
            self.project = value
        else:
            self.selected = value
            s = next(s for s in self.data['services'] if s['name'] == value)
            self.project = s['project']
            if self.page == 2:
                self.open_logs(value)
        self.render_services()

    def inspect(self, name):
        self.selected = name
        self.render_services()
        self.navigate(0)

    def navigate(self, page):
        self.page = page
        self.pages.setCurrentIndex(page)
        for i, b in enumerate(self.nav):
            b.setChecked(i == page)
        self.breadcrumb.setText('工作台 / ' + ['总览', '服务管理', '日志中心', '设置'][page])
        if page == 2:
            self.refresh_logs()

    def open_logs(self, name):
        self.log_service.setCurrentIndex(self.log_service.findData(name))
        self.navigate(2)

    def log_changed(self):
        self.update_log_terminal()
        self.log_generation += 1
        self.log_filter_key = None
        self.log_text = None
        if self.log_source.currentData() not in (None, '0'):
            self.freeze_logs()
        self.settings.setValue('log_service', self.log_service.currentData())
        self.settings.setValue('levels', ','.join(k for k, b in self.level_buttons.items() if b.isChecked()))
        self.settings.setValue('search', self.log_search.text())
        # A deliberate filter change starts a new view, even when paused.
        self.refresh_logs(force=True)

    def refresh_logs(self, force=False):
        if self.log_loading or (self.paused and not force):
            return
        target = self.log_service.currentData()
        levels = [k for k, b in self.level_buttons.items() if b.isChecked()]
        if not target or not levels:
            self.log_entries = []
            self.log_view.clear()
            self.log_summary.setText('请选择服务和至少一个日志级别')
            return
        source = self.log_source.currentData()
        args = ['logs', target, '--json', '-n', '2000', '--level', ','.join(levels), '--search', self.log_search.text()]
        args.extend(['--history'] if source == 'all' else ['--run', str(source or '0')])
        read_revision = self.read_revision
        generation = self.log_generation
        self.log_loading = True
        def done(data, error):
            self.log_loading = False
            if generation != self.log_generation:
                self.refresh_logs(force=True)
                return
            if error:
                self.log_summary.setText('读取失败：' + error)
                return
            if read_revision != self.read_revision or ((self.paused or self.log_view.textCursor().hasSelection()) and not force):
                return
            self.apply_logs(data)
        self.request(args, done)

    def apply_logs(self, data):
        self.log_entries = data['entries']
        lines, levels = [], []
        previous_source = None
        for entry in self.log_entries:
            if self.log_source.currentData() == 'all' and entry['source'] != previous_source:
                source = entry['source']
                lines.append(('当前日志' if source == 0 else f'归档 {source}') + '  ────────────────────────────────')
                levels.append('other')
                previous_source = source
            lines.append(entry['text'])
            levels.append(entry['level'])
        text = '\n'.join(lines)
        if text != self.log_text:
            self.log_text = text
            self.highlighter.levels = levels
            self.log_view.setPlainText(text)
            self.highlighter.rehighlight()
            self.log_view.verticalScrollBar().setValue(self.log_view.verticalScrollBar().maximum())
        self.log_view.setPlaceholderText('没有符合条件的日志。可以切换级别、搜索条件或历史归档。')
        current = self.log_source.currentData()
        self.log_source.blockSignals(True)
        self.log_source.clear()
        self.log_source.addItem('当前日志', '0')
        self.log_source.addItem('所有保留日志', 'all')
        for f in data['files']:
            if f['index'] > 0:
                stamp = datetime.fromtimestamp(f['modified']).strftime('%m-%d %H:%M')
                self.log_source.addItem(f"归档 {f['index']} · {stamp}", str(f['index']))
        index = self.log_source.findData(current)
        if index < 0 and current not in (None, '0', 'all'):
            self.log_source.addItem(f'归档 {current}（已轮转）', current)
            index = self.log_source.count() - 1
        self.log_source.setCurrentIndex(max(index, 0))
        self.log_source.blockSignals(False)
        size = sum(f['bytes'] for f in data['files']) / 1024 / 1024
        self.log_summary.setText(f"{len(self.log_entries)} 行 · {len(data['files'])} 份日志 · {size:.1f} MiB" + (' · 已限量，仅扫描各文件末尾 2 MiB / 展示最新 2000 行' if data['truncated'] else ''))

    def freeze_logs(self):
        self.read_revision += 1
        self.paused = True
        self.pause.setText('▶  继续实时刷新')

    def toggle_pause(self):
        if self.paused:
            self.paused = False
            cursor = self.log_view.textCursor()
            cursor.clearSelection()
            self.log_view.setTextCursor(cursor)
            self.pause.setText('Ⅱ  暂停刷新')
            self.refresh_logs()
        else:
            self.freeze_logs()

    def copy_selected(self):
        text = self.log_view.textCursor().selectedText().replace('\u2029', '\n')
        if text:
            QApplication.clipboard().setText(text)
            self.notify('已复制所选日志')
        else:
            self.notify('先在日志区拖选需要复制的内容。')

    def export_logs(self):
        name, _ = QFileDialog.getSaveFileName(self, '导出当前筛选结果', 'devtray-logs.txt', '文本文件 (*.txt)')
        if name:
            try:
                Path(name).write_text(self.log_view.toPlainText() + '\n', encoding='utf-8')
                self.notify('已导出当前显示的筛选结果：' + name)
            except OSError as error:
                self.notify(str(error), error=True)

    def action(self, op, name, after=None):
        if name in self.action_pending:
            if after:
                after()
            return
        self.action_pending.add(name)
        self.render_services()
        def done(message, error):
            self.action_pending.discard(name)
            self.notify(error or message.strip(), error=bool(error))
            self.refresh()
            self.render_services()
            if after:
                after()
        self.request([op, name], done, raw=True)

    def bulk(self, op):
        services = self.filtered_services()
        if op == 'start':
            services = [s for s in services if s['state'] not in LIVE]
        elif op == 'stop':
            services = [s for s in services if s['state'] in LIVE]
        names = [s['name'] for s in services if s['name'] not in self.action_pending]
        if not names:
            self.notify('当前没有需要执行此操作的服务。')
            return
        if op in ('stop', 'restart'):
            title = '停止' if op == 'stop' else '重启'
            if QMessageBox.question(self, title + '服务', f'将{title}以下 {len(names)} 项服务：\n\n' + '\n'.join(names)) != QMessageBox.Yes:
                return
        def next_one():
            if names:
                self.action(op, names.pop(0), next_one)
        next_one()

    def open_preview(self, service):
        self.request(['open', service['name']], lambda msg, err: self.notify(err or msg.strip(), bool(err)), raw=True)

    def open_terminal(self, name):
        if name:
            self.request(['terminal', name], lambda msg, err: self.notify(err or msg.strip(), bool(err)), raw=True)

    def update_log_terminal(self):
        service = next((s for s in self.data['services'] if s['name'] == self.log_service.currentData()), None)
        self.log_terminal.setEnabled(bool(service and service['state'] != 'not_started'))

    def preferred_mode(self):
        mode = self.settings.value('panel_mode', 'small')
        return mode if mode in ('small', 'full') else 'small'

    def present(self, mode=None, toggle=False, remember=False, page=None, anchor=None):
        if not hasattr(self, 'quick_menu'):
            self.quick_menu = QuickMenu(self)
        if toggle and (self.isVisible() or self.quick_menu.isVisible()):
            self.hide_panel()
            self.quick_menu.hide()
            return
        mode = mode or self.preferred_mode()
        if mode not in ('small', 'full'):
            return
        if remember:
            self.settings.setValue('panel_mode', mode)
            self.settings.sync()
        if mode == 'small':
            self.hide_panel()
            self.quick_menu.present(anchor)
        else:
            self.quick_menu.hide()
            if page is not None:
                self.navigate(page)
            self.show()
            self.raise_()
            self.activateWindow()

    def add_service(self):
        ServiceDialog(self).exec()

    def edit_service(self, name):
        def done(data, error):
            if error:
                self.notify(error, True)
            else:
                ServiceDialog(self, data).exec()
        self.request(['_service-config', name], done)

    def notify(self, text, error=False):
        if hasattr(self, 'quick_menu'):
            self.quick_menu.notice(text, error)
        self.banner.setText(('⚠  ' if error else '') + text)
        self.banner.show()

    def hide_panel(self):
        self.settings.setValue('geometry', self.saveGeometry())
        self.timer.stop()
        self.hide()

    def closeEvent(self, event):
        self.hide_panel()
        event.ignore()

    def showEvent(self, event):
        super().showEvent(event)
        if self.poll_enabled:
            self.timer.start(2000)
            QTimer.singleShot(0, self.refresh)


def main():
    app = QApplication(sys.argv[:1])
    app.setApplicationName('devtray')
    app.setDesktopFileName('devtray')
    app.setWindowIcon(QIcon.fromTheme('devtray', QIcon.fromTheme('utilities-terminal')))
    app.setQuitOnLastWindowClosed(False)
    binary, config, *options = sys.argv[1:]
    key = hashlib.sha256((str(os.getuid()) + ':' + str(Path(config).resolve())).encode()).hexdigest()[:24]
    name = 'devtray-panel-' + key
    target = options[options.index('--target') + 1] if '--target' in options else None
    mode = options[options.index('--mode') + 1] if '--mode' in options else ('small' if '--menu' in options else None)
    anchor = None
    if '--anchor' in options:
        i = options.index('--anchor')
        anchor = [int(options[i + 1]), int(options[i + 2])]
    request = {'toggle': '--toggle' in options, 'target': target, 'quit': '--quit' in options,
               'mode': mode, 'anchor': anchor}
    message = json.dumps(request).encode() + b'\n'
    socket = QLocalSocket()
    socket.connectToServer(name)
    if socket.waitForConnected(400):
        socket.write(message)
        socket.waitForBytesWritten(1000)
        socket.disconnectFromServer()
        return 0
    if '--quit' in options:
        return 0
    instance_lock = QLockFile(QDir.tempPath() + '/' + name + '.lock')
    if not instance_lock.tryLock(1000):
        socket.connectToServer(name)
        if socket.waitForConnected(1500):
            socket.write(message)
            socket.waitForBytesWritten(1000)
            return 0
        print('面板正在启动，请稍后重试。', file=sys.stderr)
        return 1
    server = QLocalServer()
    # A failed connection can leave a stale Unix socket after a crash.
    QLocalServer.removeServer(name)
    if not server.listen(name):
        print('无法建立面板单实例通信：' + server.errorString(), file=sys.stderr)
        return 1
    panel = Panel(binary, config)
    panel.quick_menu = QuickMenu(panel)
    pending_target = target
    def deliver(request):
        nonlocal pending_target
        if request.get('inspect'):
            return
        if request.get('quit'):
            panel.settings.setValue('geometry', panel.saveGeometry())
            app.quit()
        elif request.get('target'):
            pending_target = request['target']
            panel.present('full', page=2)
        else:
            mode = request.get('mode') or ('small' if request.get('menu') else None)
            panel.present(mode, toggle=request.get('toggle', False), remember=bool(mode), anchor=request.get('anchor'))
    connections = set()
    def connected():
        client = server.nextPendingConnection()
        connections.add(client)
        buffer = bytearray()
        def read():
            buffer.extend(bytes(client.readAll()))
            if b'\n' in buffer:
                try:
                    deliver(json.loads(buffer.split(b'\n')[0]))
                    client.write(json.dumps({'visible': panel.isVisible(), 'page': panel.page, 'menu_visible': panel.quick_menu.isVisible(), 'mode': panel.preferred_mode()}).encode() + b'\n')
                    client.flush()
                except (ValueError, TypeError):
                    pass
                client.disconnectFromServer()
        client.readyRead.connect(read)
        def cleanup():
            connections.discard(client)
            client.deleteLater()
        client.disconnected.connect(cleanup)
        read()
    server.newConnection.connect(connected)
    def focus_target():
        nonlocal pending_target
        if pending_target and panel.log_service.findData(pending_target) >= 0:
            panel.open_logs(pending_target)
            pending_target = None
    focus_timer = QTimer()
    focus_timer.timeout.connect(focus_target)
    focus_timer.start(200)
    deliver(request)
    return app.exec()


if __name__ == '__main__':
    raise SystemExit(main())
