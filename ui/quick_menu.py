"""Compact, custom-drawn tray menu; shares the panel's asynchronous command layer."""
import json

from PySide6.QtCore import Qt, QTimer, QEvent, QPoint
from PySide6.QtGui import QColor, QCursor, QIcon, QKeySequence, QShortcut
from PySide6.QtWidgets import (
    QApplication, QWidget, QFrame, QLabel, QPushButton, QLineEdit, QScrollArea,
    QVBoxLayout, QHBoxLayout, QGraphicsDropShadowEffect, QSizePolicy,
)

QUICK_STYLE = """
* { font-family: "Noto Sans CJK SC", "Noto Sans", sans-serif; font-size: 12px; color: #dfe9df; }
QFrame#quickSurface { background: #1c3027; border: 1px solid #46604f; border-radius: 16px; }
QLabel { background: transparent; border: none; }
QLabel#quickBrand { font-size: 22px; font-weight: 700; letter-spacing: -1px; color: #f1f5e9; }
QLabel#quickKicker { font-size: 9px; letter-spacing: 2px; color: #9bb49e; }
QLabel#quickMuted { color: #9db09f; font-size: 11px; }
QLabel#quickStat { font-size: 23px; font-weight: 650; }
QFrame#quickMetrics { background: #263d30; border: 1px solid #3b5240; border-radius: 10px; }
QFrame#quickProject { background: #22382d; border: 1px solid #354d3d; border-radius: 10px; }
QFrame#quickService { background: transparent; border: none; border-radius: 6px; }
QFrame#quickService:hover { background: #2c4635; }
QPushButton { background: transparent; border: 1px solid transparent; border-radius: 6px; padding: 5px 9px; color: #c8dac7; }
QPushButton:hover { background: #39523d; border-color: #536f51; color: #f4f7e9; }
QPushButton:focus { border-color: #b7ce8c; }
QPushButton:pressed { background: #456346; }
QPushButton:disabled { color: #6d8371; }
QPushButton#quickPin:checked { background: #3b563d; border-color: #739263; color: #d9e7bc; }
QPushButton#quickAction { background: #2c4433; border: 1px solid #3e5941; font-size: 14px; padding: 0; }
QPushButton#quickAction:hover { background: #466047; border-color: #a2bf86; }
QPushButton#quickOpen { background: #bed297; border: 1px solid #bed297; color: #213525; border-radius: 8px; font-weight: 650; padding: 11px; }
QPushButton#quickOpen:hover { background: #d1e0b1; }
QPushButton#quickProjectTitle { text-align: left; color: #e3ecdb; font-weight: 650; font-size: 12px; padding: 6px 3px; }
QLineEdit { background: #182a21; color: #e4ecdf; border: 1px solid #3c5543; border-radius: 7px; padding: 8px 10px; selection-background-color: #557747; }
QLineEdit:focus { border-color: #a5bc7e; }
QScrollArea { background: transparent; border: none; }
QScrollArea > QWidget > QWidget { background: transparent; }
QScrollBar:vertical { background: transparent; width: 6px; margin: 0; }
QScrollBar::handle:vertical { background: #536e54; border-radius: 3px; min-height: 25px; }
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical { height: 0; }
QToolTip { background: #e9eddd; color: #263a2a; border: 1px solid #9fb08c; padding: 5px; }
QLabel#quickNotice { background: #30432e; border: 1px solid #4f6848; border-radius: 7px; padding: 7px; font-size: 11px; }
"""

STATUS = {
    'ready': ('就绪', '#b8dca2'), 'running': ('运行中', '#b8dca2'),
    'starting': ('启动中', '#edca7c'), 'exited_failed': ('异常', '#f0a38d'),
    'not_started': ('未启动', '#a2b1a5'), 'exited_ok': ('已结束', '#a2b1a5'),
    'exited_signal': ('已停止', '#a2b1a5'),
}
LIVE = {'ready', 'running', 'starting'}


def text(value, name=None):
    w = QLabel(value)
    w.setTextFormat(Qt.PlainText)
    if name:
        w.setObjectName(name)
    return w


def button(value, callback, name=None):
    w = QPushButton(value)
    if name:
        w.setObjectName(name)
    w.setCursor(Qt.PointingHandCursor)
    w.clicked.connect(callback)
    return w


def horizontal(*widgets):
    layout = QHBoxLayout()
    layout.setSpacing(7)
    for w in widgets:
        if w is None:
            layout.addStretch()
        else:
            layout.addWidget(w)
    return layout


class ServiceRow(QFrame):
    def __init__(self, menu, service):
        super().__init__()
        self.menu = menu
        self.service = service
        self.setObjectName('quickService')
        container = QVBoxLayout(self)
        container.setContentsMargins(7, 8, 7, 8)
        container.setSpacing(6)
        layout = QHBoxLayout()
        container.addLayout(layout)
        layout.setSpacing(6)
        self.dot = text('●')
        self.dot.setFixedWidth(13)
        layout.addWidget(self.dot)
        description = QVBoxLayout()
        description.setSpacing(3)
        self.name = text(service['service'])
        self.name.setStyleSheet('font-weight:550; color:#e4eddf;')
        self.name.setMinimumWidth(0)
        self.name.setSizePolicy(QSizePolicy.Ignored, QSizePolicy.Preferred)
        self.state = text('', 'quickMuted')
        description.addWidget(self.name)
        description.addWidget(self.state)
        layout.addLayout(description, 1)
        self.start_stop = self.action_button('▷', self.toggle)
        self.restart = self.action_button('↻', lambda: menu.perform('restart', self.service['name']))
        self.restart.setToolTip('重启服务')
        self.logs = self.action_button('≡', lambda: menu.open_logs(self.service['name']))
        self.logs.setToolTip('查看日志')
        for w in (self.start_stop, self.restart, self.logs):
            layout.addWidget(w)
        self.terminal = button('实时终端', lambda: menu.owner.open_terminal(self.service['name']))
        self.terminal.setToolTip('tmux 原始屏幕与实时输出，不受分类筛选影响')
        self.preview = button('浏览器 ↗', lambda: menu.owner.open_preview(self.service))
        container.addLayout(horizontal(self.terminal, self.preview, None))
        self.update_service(service)

    def action_button(self, value, callback):
        b = button(value, callback, 'quickAction')
        b.setFixedSize(29, 29)
        return b

    def update_service(self, service):
        self.service = service
        status, color = STATUS.get(service['state'], STATUS['not_started'])
        self.dot.setStyleSheet(f'color:{color};')
        self.name.setText(service['service'])
        self.name.setToolTip(service['name'] + '\n' + service['cwd'])
        detail = status
        if service.get('port'):
            detail += f"  ·  :{service['port']}"
        if service['state'] == 'exited_failed' and service.get('exit_code') is not None:
            detail += f"  ·  exit {service['exit_code']}"
        self.state.setText(detail)
        self.state.setStyleSheet(f'color:{color};font-size:10px;')
        running = service['state'] in LIVE
        self.start_stop.setText('□' if running else '▷')
        self.start_stop.setToolTip('停止服务' if running else '启动服务')
        self.start_stop.setAccessibleName(('停止 ' if running else '启动 ') + service['name'])
        self.restart.setAccessibleName('重启 ' + service['name'])
        self.logs.setAccessibleName('查看日志 ' + service['name'])
        self.logs.setToolTip('分类日志 / 历史归档')
        self.terminal.setEnabled(service['state'] != 'not_started')
        self.terminal.setAccessibleName('实时终端 ' + service['name'])
        self.preview.setEnabled(bool(service.get('url')))
        self.preview.setToolTip(service.get('url') or '尚无预览地址，请配置 URL 或端口')
        self.preview.setAccessibleName('浏览器打开 ' + service['name'])
        busy = service['name'] in self.menu.owner.action_pending
        self.start_stop.setEnabled(not busy)
        self.restart.setEnabled(not busy)
        if busy:
            self.state.setText('正在执行…')

    def toggle(self):
        self.menu.perform('stop' if self.service['state'] in LIVE else 'start', self.service['name'])


class ProjectCard(QFrame):
    def __init__(self, menu, name, services):
        super().__init__()
        self.menu, self.name = menu, name
        self.setObjectName('quickProject')
        layout = QVBoxLayout(self)
        layout.setContentsMargins(9, 5, 9, 7)
        layout.setSpacing(1)
        self.title = button(name, self.toggle, 'quickProjectTitle')
        self.title.setMinimumWidth(0)
        self.title.setSizePolicy(QSizePolicy.Ignored, QSizePolicy.Fixed)
        self.title.setToolTip(name)
        self.count = text('', 'quickMuted')
        header = QHBoxLayout()
        header.addWidget(self.title, 1)
        header.addWidget(self.count)
        layout.addLayout(header)
        self.contents = QWidget()
        rows = QVBoxLayout(self.contents)
        rows.setContentsMargins(0, 0, 0, 0)
        rows.setSpacing(1)
        self.rows = {}
        for service in services:
            item = ServiceRow(menu, service)
            rows.addWidget(item)
            self.rows[service['name']] = item
        layout.addWidget(self.contents)
        self.update_services(services)

    def toggle(self):
        if self.name in self.menu.collapsed:
            self.menu.collapsed.remove(self.name)
        else:
            self.menu.collapsed.add(self.name)
        self.menu.owner.settings.setValue('quick_collapsed', json.dumps(sorted(self.menu.collapsed)))
        self.filter(self.menu.search.text())

    def update_services(self, services):
        live = sum(s['state'] in LIVE for s in services)
        self.count.setText(f'{live} / {len(services)}')
        self.count.setToolTip('运行中的服务 / 全部服务')
        for service in services:
            self.rows[service['name']].update_service(service)
        self.filter(self.menu.search.text())

    def filter(self, query):
        query = query.strip().lower()
        found = False
        for item in self.rows.values():
            service = item.service
            match = not query or any(query in str(service.get(k, '')).lower() for k in ('name', 'cwd', 'cmd'))
            item.setVisible(match)
            found = found or match
        opened = self.name not in self.menu.collapsed or bool(query)
        self.title.setText(('⌄  ' if opened else '›  ') + self.name)
        self.title.setAccessibleName(('收起 ' if opened else '展开 ') + self.name)
        self.contents.setVisible(opened)
        self.setVisible(found)
        return found


class DragHeader(QWidget):
    """Ask the compositor to move the window (also works on Wayland)."""
    def __init__(self, menu):
        super().__init__()
        self.menu = menu
        self.drag_offset = None
        self.setCursor(Qt.SizeAllCursor)
        self.setToolTip('拖动标题移动浮窗')

    def mousePressEvent(self, event):
        if event.button() == Qt.LeftButton:
            handle = self.menu.windowHandle()
            if not handle or not handle.startSystemMove():
                self.drag_offset = event.globalPosition().toPoint() - self.menu.pos()
            event.accept()
        else:
            super().mousePressEvent(event)

    def mouseMoveEvent(self, event):
        if self.drag_offset is not None and event.buttons() & Qt.LeftButton:
            self.menu.move(event.globalPosition().toPoint() - self.drag_offset)
            event.accept()
        else:
            super().mouseMoveEvent(event)

    def mouseReleaseEvent(self, event):
        self.drag_offset = None
        super().mouseReleaseEvent(event)


class QuickMenu(QWidget):
    def __init__(self, owner):
        super().__init__(None, Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint)
        self.owner = owner
        self.setWindowTitle('devtray · 小面板')
        self.positioned = False
        self.pinned = owner.settings.value('quick_pinned', True, type=bool)
        self.setAttribute(Qt.WA_TranslucentBackground)
        self.setStyleSheet(QUICK_STYLE)
        self.setFixedWidth(430)
        self.resize(430, 660)
        self.signature = None
        self.default_collapse = owner.settings.value('quick_collapsed') is None
        self.projects = {}
        try:
            self.collapsed = set(json.loads(str(owner.settings.value('quick_collapsed', '[]'))))
        except (ValueError, TypeError):
            self.collapsed = set()
        layout = QVBoxLayout(self)
        layout.setContentsMargins(12, 12, 12, 16)
        self.surface = QFrame()
        self.surface.setObjectName('quickSurface')
        shadow = QGraphicsDropShadowEffect(self.surface)
        shadow.setBlurRadius(22)
        shadow.setOffset(0, 4)
        shadow.setColor(QColor(0, 0, 0, 105))
        self.surface.setGraphicsEffect(shadow)
        layout.addWidget(self.surface)
        body = QVBoxLayout(self.surface)
        body.setContentsMargins(20, 19, 20, 16)
        body.setSpacing(12)
        brand = text('devtray', 'quickBrand')
        icon = text('')
        icon.setPixmap(QIcon.fromTheme('devtray', QIcon.fromTheme('utilities-terminal')).pixmap(28, 28))
        icon.setFixedSize(30, 30)
        close = button('×', self.hide)
        close.setFixedSize(26, 26)
        close.setStyleSheet('padding:0;font-size:18px;')
        close.setToolTip('收回浮窗 · Esc')
        close.setAccessibleName('收回服务浮窗')
        self.pin = button('固定', self.set_pinned, 'quickPin')
        self.pin.setCheckable(True)
        self.pin.setChecked(self.pinned)
        self.pin.setAccessibleName('保持浮窗显示')
        self.header = DragHeader(self)
        header_layout = horizontal(icon, brand, None, self.pin, close)
        header_layout.setContentsMargins(0, 0, 0, 0)
        self.header.setLayout(header_layout)
        body.addWidget(self.header)
        self.hint = text('', 'quickMuted')
        body.addWidget(self.hint)
        self.update_pin_hint()
        metrics = QFrame()
        metrics.setObjectName('quickMetrics')
        metric_layout = QHBoxLayout(metrics)
        metric_layout.setContentsMargins(15, 11, 15, 11)
        self.numbers = []
        for title, color in [('运行中', '#c0dba6'), ('启动中', '#edca7c'), ('需关注', '#eea18a')]:
            box = QVBoxLayout()
            box.setSpacing(2)
            number = text('—', 'quickStat')
            number.setStyleSheet(f'color:{color}')
            self.numbers.append(number)
            box.addWidget(number)
            box.addWidget(text(title, 'quickMuted'))
            metric_layout.addLayout(box, 1)
        body.addWidget(metrics)
        self.search = QLineEdit()
        self.search.setPlaceholderText('搜索项目、服务或工作目录…')
        self.search.setClearButtonEnabled(True)
        self.search.textChanged.connect(self.filter_projects)
        body.addWidget(self.search)
        self.project_count = text('项目', 'quickKicker')
        self.connection = text('连接中', 'quickMuted')
        refresh = button('↻', self.refresh)
        refresh.setFixedSize(24, 24)
        refresh.setStyleSheet('padding:0;font-size:16px;')
        refresh.setToolTip('刷新状态')
        body.addLayout(horizontal(self.project_count, None, self.connection, refresh))
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        content = QWidget()
        self.project_layout = QVBoxLayout(content)
        self.project_layout.setContentsMargins(0, 0, 3, 0)
        self.project_layout.setSpacing(9)
        self.project_layout.setAlignment(Qt.AlignTop)
        self.empty = text('添加第一个服务，开始工作。', 'quickMuted')
        self.empty.setWordWrap(True)
        self.project_layout.addWidget(self.empty)
        scroll.setWidget(content)
        body.addWidget(scroll, 1)
        self.notice_label = text('', 'quickNotice')
        self.notice_label.setWordWrap(True)
        self.notice_label.hide()
        body.addWidget(self.notice_label)
        self.expand_button = button('切换完整面板    ↗', lambda: owner.present('full', remember=True), 'quickOpen')
        body.addWidget(self.expand_button)
        body.addLayout(horizontal(button('＋ 添加服务', self.add_service), None, button('设置', lambda: self.open_panel(3))))
        self.timer = QTimer(self)
        self.timer.setInterval(2000)
        self.timer.timeout.connect(self.refresh)
        self.notice_timer = QTimer(self)
        self.notice_timer.setSingleShot(True)
        self.notice_timer.timeout.connect(self.notice_label.hide)
        QShortcut(QKeySequence('Escape'), self, self.hide)
        QShortcut(QKeySequence('Ctrl+F'), self, self.search.setFocus)
        owner.destroyed.connect(self.deleteLater)
        self.sync(owner.data)

    def sync(self, data):
        services = data.get('services', [])
        counts = [sum(s['state'] in ('ready', 'running') for s in services),
                  sum(s['state'] == 'starting' for s in services),
                  sum(s['state'] == 'exited_failed' for s in services)]
        for number, value in zip(self.numbers, counts):
            number.setText(str(value).zfill(2))
        groups = {}
        for service in services:
            groups.setdefault(service['project'], []).append(service)
        if self.default_collapse and services:
            self.collapsed.update(name for name, items in groups.items() if all(s['state'] not in LIVE and s['state'] != 'exited_failed' for s in items))
            self.default_collapse = False
        signature = [(project, [s['name'] for s in items]) for project, items in groups.items()]
        if signature != self.signature:
            # Only configuration changes rebuild rows; live status updates keep focus/hover intact.
            for card in self.projects.values():
                self.project_layout.removeWidget(card)
                card.deleteLater()
            self.projects.clear()
            self.signature = signature
            for name, items in groups.items():
                card = ProjectCard(self, name, items)
                self.projects[name] = card
                self.project_layout.insertWidget(self.project_layout.count() - 1, card)
        else:
            for name, items in groups.items():
                self.projects[name].update_services(items)
        self.project_count.setText(f'项目  /  {len(groups):02d}')
        if data.get('config'):
            self.connection.setText('● 已同步')
        self.filter_projects()

    def filter_projects(self):
        matches = sum(card.filter(self.search.text()) for card in self.projects.values())
        self.empty.setText('没有匹配的项目或服务。' if self.projects else '添加第一个服务，开始工作。')
        self.empty.setVisible(matches == 0)

    def notice(self, message, error=False):
        if not self.isVisible():
            return
        self.notice_label.setText(message)
        self.notice_label.setToolTip(message)
        if error:
            self.connection.setText('同步异常')
        self.notice_label.show()
        self.notice_timer.start(8000 if error else 4000)

    def refresh(self):
        self.owner.refresh()

    def set_pinned(self, checked):
        self.pinned = checked
        self.owner.settings.setValue('quick_pinned', checked)
        self.update_pin_hint()

    def update_pin_hint(self):
        self.hint.setText('拖动标题移动 · 已固定' if self.pinned else '拖动标题移动 · 点击外部收回')
        self.pin.setToolTip('取消固定：点击其他窗口时收回' if self.pinned else '固定浮窗：切换窗口时保持显示')

    def perform(self, op, name):
        self.owner.action(op, name)

    def open_panel(self, page=0):
        # Opening a detail is temporary; only the explicit mode switch changes
        # the user's preferred launch mode.
        self.owner.present('full', page=page)

    def open_logs(self, name):
        self.open_panel(2)
        self.owner.open_logs(name)

    def add_service(self):
        self.open_panel(1)
        QTimer.singleShot(0, self.owner.add_service)

    def present(self, anchor=None, toggle=False):
        if toggle and self.isVisible():
            self.hide()
            return
        point = QPoint(*anchor) if anchor and len(anchor) == 2 and anchor != [0, 0] else QCursor.pos()
        screen = QApplication.screenAt(point) or QApplication.primaryScreen()
        area = screen.availableGeometry()
        # Fixed size also tells tiling compositors (e.g. niri) to open this as
        # a floating utility window without changing the user's window rules.
        self.setFixedSize(self.width(), min(720, max(360, area.height() - 24)))
        # Preserve a user-dragged position when reopening within this UI session.
        if self.positioned:
            area = self.screen().availableGeometry()
            x, y = self.x(), self.y()
        else:
            x, y = point.x() - self.width() + 24, point.y() + 12
        x = min(max(area.left(), x), area.right() - self.width() + 1)
        if y + self.height() > area.bottom():
            y = area.bottom() - self.height() if self.positioned else point.y() - self.height() - 12
        y = max(area.top(), min(y, area.bottom() - self.height() + 1))
        self.move(x, y)
        self.positioned = True
        self.sync(self.owner.data)
        self.show()
        self.raise_()
        self.activateWindow()
        self.search.setFocus(Qt.PopupFocusReason)
        if self.owner.poll_enabled:
            self.timer.start()
            self.refresh()

    def hideEvent(self, event):
        self.timer.stop()
        super().hideEvent(event)

    def event(self, event):
        if event.type() == QEvent.WindowDeactivate and not self.pinned:
            self.hide()
        return super().event(event)
