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
from resource_panel import ResourcePanelMixin
from resource_selection import human_bytes
from panel_theme import STYLE
from service_alert_panel import ServiceAlertsMixin, unread

from PySide6.QtNetwork import QLocalServer, QLocalSocket
from PySide6.QtWidgets import (
    QApplication, QMainWindow, QWidget, QFrame, QLabel, QPushButton, QLineEdit,
    QVBoxLayout, QHBoxLayout, QGridLayout, QStackedWidget, QTreeWidget, QTreeWidgetItem,
    QScrollArea, QPlainTextEdit, QComboBox, QCheckBox, QFileDialog, QMessageBox,
    QDialog, QDialogButtonBox, QFormLayout, QSplitter, QSizePolicy,
    QTableWidget, QTableWidgetItem, QHeaderView, QAbstractItemView, QInputDialog, QMenu,
)


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
    layout.setContentsMargins(14, 14, 14, 14)
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
            item.widget().hide()
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


class Panel(ResourcePanelMixin, ServiceAlertsMixin, QMainWindow):
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
        self.init_resource_state()
        self.active_job = None
        self.active_job_project = None
        self.active_job_kind = None
        self.job_polling = False
        self.pending_write = False
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
        self.sidebar.setFixedWidth(224)
        side = QVBoxLayout(self.sidebar)
        side.setContentsMargins(12, 16, 12, 16)
        side.setSpacing(8)
        side.addWidget(label('devtray', 'brand'))
        side.addWidget(label('本地工作台', 'sideCaption'))
        side.addSpacing(12)
        self.nav = []
        for index, title in enumerate(['总览', '服务管理', '资源', 'Agent 与 Skills', '日志中心', '工作台设置']):
            b = button(title, lambda checked=False, i=index: self.navigate(i), 'nav')
            b.setCheckable(True)
            side.addWidget(b)
            self.nav.append(b)
        side.addSpacing(12)
        side.addLayout(row(label('工作目录', 'sideCaption'), None, button('+', self.add_service, 'nav')))
        self.project_search = QLineEdit()
        self.project_search.setPlaceholderText('筛选项目或服务…')
        self.project_search.setStyleSheet('background:#263d33; color:#dce7e0; border:1px solid #3a5042;')
        self.project_search.textChanged.connect(self.filter_tree)
        side.addWidget(self.project_search)
        self.tree = QTreeWidget()
        self.tree.setHeaderHidden(True)
        self.tree.setIndentation(12)
        self.tree.itemClicked.connect(self.select_tree)
        self.tree.itemExpanded.connect(self.remember_tree_expansion)
        self.tree.itemCollapsed.connect(self.remember_tree_expansion)
        self._updating_tree = False
        self._tree_expanded_before_search = None
        side.addWidget(self.tree, 1)
        side.addWidget(label('●  本机工作空间', 'sideCaption'))
        side.addWidget(label('窗口收回后，服务继续运行', 'sideCaption'))
        shell.addWidget(self.sidebar)
        main = QWidget()
        body = QVBoxLayout(main)
        body.setContentsMargins(18, 18, 18, 14)
        body.setSpacing(12)
        self.breadcrumb = label('工作台 / 总览', 'muted')
        self.compact_button = button('切换小面板', lambda: self.present('small', remember=True))
        body.addLayout(row(self.breadcrumb, None, self.compact_button, button('刷新', self.refresh), button('＋ 添加服务', self.add_service, 'primary'), button('收回', self.hide_panel)))
        self.banner = label('', 'errorBanner', True)
        self.banner.hide()
        self.dismiss_button = button('清除提示', self.dismiss_notice)
        self.dismiss_button.setToolTip('清除提示并确认当前托盘告警；服务与日志保持原样')
        self.dismiss_button.hide()
        notice_row = row(self.banner, self.dismiss_button)
        notice_row.setStretch(0, 1)
        self.dismiss_button.setSizePolicy(QSizePolicy.Maximum, QSizePolicy.Fixed)
        body.addLayout(notice_row)
        self.pages = QStackedWidget()
        body.addWidget(self.pages, 1)
        shell.addWidget(main, 1)
        self.build_overview()
        self.build_services()
        self.build_resources()
        self.build_agents()
        self.build_logs()
        self.build_settings()
        self.footer = label('正在连接本机服务…', 'muted')
        self.ack_all_button = button('全部异常标记已读', self.dismiss_notice)
        self.ack_all_button.setEnabled(False)
        body.addLayout(row(self.footer, None, self.ack_all_button, label('本机开发工作台', 'mono')))
        self.navigate(0)
        self.job_timer = QTimer(self)
        self.job_timer.timeout.connect(self.poll_job)
        self.timer = QTimer(self)
        self.timer.timeout.connect(self.refresh)
        if poll:
            self.timer.start(2000)
            QTimer.singleShot(0, self.refresh)
        QShortcut(QKeySequence('Escape'), self, self.hide_panel)
        QShortcut(QKeySequence('Ctrl+L'), self, lambda: self.navigate(4))
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
        h.addWidget(label('本机服务总览', 'title'))
        self.hero_caption = label('本机服务总览 · 启动、检查、回到工作。', 'muted')
        h.addWidget(self.hero_caption)
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
        asset, asset_layout = card()
        self.asset_summary = label('尚未扫描 · 打开资源页开始扫描', 'muted')
        asset_layout.addLayout(row(label('资产摘要', 'sectionTitle'), self.asset_summary, None, button('查看资源', lambda: self.navigate(2))))
        layout.addWidget(asset)
        split = QSplitter(Qt.Horizontal)
        left = QWidget()
        l = QVBoxLayout(left)
        l.setContentsMargins(0, 0, 8, 0)
        l.setSpacing(12)
        self.cards_title = label('服务一览', 'sectionTitle')
        self.project_ack_button = button('本项目异常标记已读', lambda: self.acknowledge_project(self.project))
        self.project_ack_button.hide()
        l.addLayout(row(self.cards_title, None, button('查看全部', lambda: self.navigate(1))))
        l.addWidget(self.project_ack_button, 0, Qt.AlignLeft)
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

    def poll_job(self):
        if not self.active_job:
            self.job_timer.stop()
            return
        if self.job_polling:
            return
        job_id = self.active_job
        self.job_polling = True
        def done(data, error):
            self.job_polling = False
            if job_id != self.active_job:
                return
            if error:
                # The worker may still be writing. Keep tracking it rather than
                # enabling another write merely because a status request failed.
                if getattr(self, 'job_last_error', None) != error:
                    self.notify('查询任务失败，保留当前任务并重试：' + error, error=True)
                self.job_last_error = error
                self.job_timer.start(2000)
                return
            self.job_last_error = None
            self.job_timer.setInterval(500)
            state = data.get('state')
            if state in ('done', 'error', 'cancelled'):
                self.job_timer.stop()
                self.active_job = None
                if self.active_job_kind == 'clean':
                    self.finish_resource_job(data)
                else:
                    self.active_job_kind = None
                    if state == 'done':
                        self.notify('技能任务完成，正在刷新清单')
                        self.load_agents_page()
                    else:
                        self.notify(data.get('error') or f'任务结束：{state}', error=True)
                    self.update_resource_controls()
            else:
                progress = int(float(data.get('progress') or 0) * 100)
                self.footer.setText(f'后台任务 {progress}% · {data.get("message") or self.active_job_project or "处理中"}')
        self.request(['_job', job_id], done, timeout_ms=15000)

    def load_asset_summary(self):
        def done(data, error):
            if error or not isinstance(data, list):
                return
            if not data:
                self.asset_summary.setText('尚未扫描 · 打开资源页开始扫描')
                return
            total = sum(u['total_allocated'] for u in data)
            clean = sum(u['deletable_allocated'] for u in data)
            self.asset_summary.setText(
                f'磁盘 {human_bytes(total)} · 可清理候选 {human_bytes(clean)} · 已扫描 {len(data)} 个项目'
            )

        self.request(['_disk-cache'], done, timeout_ms=15000)

    def build_agents(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(14)
        hero, h = card('pageIntro')
        h.addWidget(label('Agent 与 Skills', 'title'))
        h.addWidget(label('Agent 重新检测本地版本；Skills 检查 GitHub 上游更新。右键列表项查看操作和详情。', 'muted'))
        self.skills_check_button = button('检查 Skills 更新', self.check_skills_updates)
        h.addLayout(row(
            button('重新检测 Agent', self.load_agents_page, 'primary'),
            self.skills_check_button,
            button('安装技能…', self.install_skill),
            None,
            button('打开 Skills 目录', self.open_skills_dir),
        ))
        layout.addWidget(hero)
        self.agents_summary = label('尚未检测', 'muted')
        layout.addWidget(self.agents_summary)
        split = QSplitter(Qt.Vertical)
        agents_card, al = card()
        al.addLayout(row(label('本机 Agent', 'sectionTitle'), None, button('选中项操作', self.show_agent_actions)))
        self.agents_table = QTableWidget(0, 4)
        self.agents_table.setHorizontalHeaderLabels(['Agent', '版本', '来源', '建议升级命令'])
        self.agents_table.verticalHeader().setVisible(False)
        self.agents_table.setAlternatingRowColors(True)
        self.agents_table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.agents_table.setSelectionMode(QAbstractItemView.SingleSelection)
        self.agents_table.setEditTriggers(QAbstractItemView.NoEditTriggers)
        self.agents_table.setContextMenuPolicy(Qt.CustomContextMenu)
        self.agents_table.customContextMenuRequested.connect(self.agent_context_menu)
        self.agents_table.horizontalHeader().setSectionResizeMode(3, QHeaderView.Stretch)
        self.agents_table.setColumnWidth(0, 190)
        self.agents_table.setColumnWidth(1, 110)
        self.agents_table.setColumnWidth(2, 190)
        al.addWidget(self.agents_table, 1)
        split.addWidget(agents_card)
        skills_card, sl = card()
        self.skills_search = QLineEdit()
        self.skills_search.setPlaceholderText('筛选技能名 / 来源…')
        sl.addLayout(row(label('Skills', 'sectionTitle'), None, self.skills_search, button('刷新', self.load_agents_page)))
        self.skills_search.textChanged.connect(lambda _text: self.render_skills())
        sl.addLayout(row(
            button('升级选中', lambda: self.skill_action('upgrade')),
            button('删除选中', lambda: self.skill_action('remove'), 'danger'),
            button('恢复快照', self.restore_selected),
            None,
            label('写入前自动快照；副本漂移默认拒绝', 'muted'),
        ))
        self.skills_table = QTableWidget(0, 6)
        self.skills_table.setHorizontalHeaderLabels(['名称', '来源', '版本', '安装', '副本', '状态'])
        self.skills_table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.skills_table.setSelectionMode(QAbstractItemView.SingleSelection)
        self.skills_table.verticalHeader().setVisible(False)
        self.skills_table.setAlternatingRowColors(True)
        self.skills_table.setEditTriggers(QAbstractItemView.NoEditTriggers)
        self.skills_table.setContextMenuPolicy(Qt.CustomContextMenu)
        self.skills_table.customContextMenuRequested.connect(self.skill_context_menu)
        self.skills_table.setColumnHidden(4, True)
        sl.addLayout(row(None, button('选中项操作', self.show_skill_actions)))
        self.skills_table.horizontalHeader().setSectionResizeMode(1, QHeaderView.Stretch)
        self.skills_table.setColumnWidth(0, 220)
        self.skills_table.setColumnWidth(2, 80)
        self.skills_table.setColumnWidth(3, 90)
        self.skills_table.setColumnWidth(4, 110)
        self.skills_table.setColumnWidth(5, 110)
        sl.addWidget(self.skills_table, 1)
        split.addWidget(skills_card)
        split.setSizes([260, 480])
        layout.addWidget(split, 1)
        self.pages.addWidget(page)

    def load_agents_page(self):
        def agents_done(data, error):
            if error:
                self.notify('Agent 检测失败：' + error, error=True)
                return
            self.render_agents(data if isinstance(data, list) else [])

        def skills_done(data, error):
            if error:
                self.notify('Skills 清单读取失败：' + error, error=True)
                return
            self.skills_inventory = data
            self.render_skills()

        self.request(['_agents'], agents_done, timeout_ms=30000)
        self.request(['_skills'], skills_done, timeout_ms=60000)

    def render_agents(self, agents):
        self.agents_data = agents
        self.agents_table.setRowCount(0)
        for agent in agents:
            row = self.agents_table.rowCount()
            self.agents_table.insertRow(row)
            self.agents_table.setItem(row, 0, QTableWidgetItem(agent['name'] + '  ' + agent['id']))
            self.agents_table.setItem(row, 1, QTableWidgetItem(agent.get('version') or '未知'))
            source = agent.get('provenance') or 'unknown'
            if agent.get('package'):
                source += ' · ' + agent['package']
            source_item = QTableWidgetItem(source)
            source_item.setToolTip(agent.get('path') or '')
            self.agents_table.setItem(row, 2, source_item)
            command = agent.get('update_command') or '—'
            command_item = QTableWidgetItem(command)
            command_item.setToolTip('需 sudo，请自行在终端执行' if agent.get('needs_sudo') else '建议命令，尚未执行')
            self.agents_table.setItem(row, 3, command_item)
        self.update_agents_summary()

    def render_skills(self):
        inventory = getattr(self, 'skills_inventory', None)
        if not inventory:
            return
        summary = inventory['summary']
        query = self.skills_search.text().strip().lower()
        skills = [
            s for s in inventory['skills']
            if not query
            or query in s['name'].lower()
            or query in (s.get('source') or '').lower()
            or query in (s.get('display_name') or '').lower()
        ]
        self.skills_table.setRowCount(0)
        for skill in skills:
            row = self.skills_table.rowCount()
            self.skills_table.insertRow(row)
            self.skills_table.setItem(row, 0, QTableWidgetItem(skill['name']))
            self.skills_table.setItem(row, 1, QTableWidgetItem(skill.get('source') or '来源未知'))
            sha = skill.get('tree_sha') or ''
            self.skills_table.setItem(row, 2, QTableWidgetItem(sha[:7] if sha else '—'))
            installed = (skill.get('installed_at') or '')[:10]
            self.skills_table.setItem(row, 3, QTableWidgetItem(installed or '—'))
            drift = sum(1 for m in skill.get('mirrors', []) if m['status'] == 'drift')
            copies = len(skill.get('mirrors', []))
            copy_text = f'{copies} 份' + (f' · {drift} 漂移' if drift else '') if copies else '—'
            copy_item = QTableWidgetItem(copy_text)
            copy_item.setToolTip('\n'.join(m['path'] for m in skill.get('mirrors', [])))
            self.skills_table.setItem(row, 4, copy_item)
            flags = []
            if not skill.get('dir_exists'):
                flags.append('目录缺失')
            if not skill.get('managed'):
                flags.append('来源未知')
            else:
                if skill.get('dir_exists'):
                    flags.append({
                        'available': '有更新',
                        'current': '已是最新',
                        'unknown': '无法检查',
                    }.get(skill.get('update'), '未检查'))
            status = ' · '.join(flags) or '未检查'
            status_item = QTableWidgetItem(status)
            tip = list(skill.get('notes', []))
            if skill.get('update_detail'):
                tip.append('上游：' + skill['update_detail'])
            if skill.get('update_checked_at'):
                tip.append('检查于 ' + datetime.fromtimestamp(skill['update_checked_at']).strftime('%m-%d %H:%M'))
            status_item.setToolTip('\n'.join(tip))
            self.skills_table.setItem(row, 5, status_item)
        self.update_agents_summary()

    def selected_skill_name(self):
        row = self.skills_table.currentRow()
        if row < 0:
            return None
        item = self.skills_table.item(row, 0)
        return item.text() if item else None

    def make_agent_menu(self, agent):
        menu = QMenu(self)
        menu.addAction('重新检测 Agent', self.load_agents_page)
        menu.addAction('复制可执行文件路径', lambda: QApplication.clipboard().setText(agent.get('path') or ''))
        command = menu.addAction('复制建议升级命令', lambda: QApplication.clipboard().setText(agent.get('update_command') or ''))
        command.setEnabled(bool(agent.get('update_command')))
        def details():
            text = '\n'.join([
                agent.get('path') or '路径未知',
                '本地版本：' + (agent.get('version') or '无法读取'),
                '安装来源：' + (agent.get('provenance') or '未知'),
                'Agent 暂无通用的远程更新检查；建议命令按安装来源生成。',
                *agent.get('notes', []),
            ])
            QMessageBox.information(self, agent['name'] + ' 检测详情', text)
        menu.addAction('检测详情', details)
        return menu

    def agent_context_menu(self, position):
        item = self.agents_table.itemAt(position)
        if item is None:
            return
        self.agents_table.selectRow(item.row())
        agents = getattr(self, 'agents_data', [])
        if item.row() < len(agents):
            self.make_agent_menu(agents[item.row()]).exec(self.agents_table.viewport().mapToGlobal(position))

    def show_agent_actions(self):
        index = self.agents_table.currentRow()
        agents = getattr(self, 'agents_data', [])
        if 0 <= index < len(agents):
            self.make_agent_menu(agents[index]).exec(self.agents_table.mapToGlobal(self.agents_table.rect().center()))
        else:
            self.notify('先选中一个 Agent')

    def make_skill_menu(self, skill):
        menu = QMenu(self)
        open_action = menu.addAction('打开技能目录', lambda: QDesktopServices.openUrl(QUrl.fromLocalFile(skill['dir'])))
        open_action.setEnabled(bool(skill.get('dir_exists')))
        menu.addAction('复制来源', lambda: QApplication.clipboard().setText(skill.get('source_url') or skill.get('source') or skill['dir']))
        menu.addAction('检查全部 Skills 更新', self.check_skills_updates)
        menu.addSeparator()
        for title, callback in [('升级此技能', lambda: self.skill_action('upgrade', skill['name'])), ('删除此技能', lambda: self.skill_action('remove', skill['name']))]:
            action = menu.addAction(title, callback)
            action.setEnabled(bool(skill.get('managed') and skill.get('dir_exists')) and not self.write_busy())
        menu.addAction('恢复快照', lambda: self.restore_selected(skill['name'])).setEnabled(not self.write_busy())
        menu.addSeparator()
        def details():
            mirrors = [f"{m['agent']}：{m['path']}（{m['kind']} / {m['status']}）" for m in skill.get('mirrors', [])]
            QMessageBox.information(self, skill['name'] + ' 详情', '\n'.join([
                skill['dir'], '来源：' + (skill.get('source') or '未知'),
                '副本检查用于写入保护；不会单独自动清理副本。', *mirrors, *skill.get('notes', []),
            ]))
        menu.addAction('详情与副本状态', details)
        return menu

    def skill_context_menu(self, position):
        item = self.skills_table.itemAt(position)
        if item is None:
            return
        self.skills_table.selectRow(item.row())
        self.show_skill_actions(self.skills_table.viewport().mapToGlobal(position))

    def show_skill_actions(self, position=None):
        name = self.selected_skill_name()
        skill = next((s for s in getattr(self, 'skills_inventory', {}).get('skills', []) if s['name'] == name), None)
        if skill:
            point = position if position is not None and not isinstance(position, bool) else self.skills_table.mapToGlobal(self.skills_table.rect().center())
            self.make_skill_menu(skill).exec(point)
        else:
            self.notify('先选中一个技能')

    def write_busy(self):
        return bool(self.active_job or self.cleanup_busy or self.pending_write)

    def begin_write(self):
        if self.write_busy():
            self.notify('已有写入计划或后台任务，请先完成当前操作', error=True)
            return False
        self.pending_write = True
        self.update_resource_controls()
        return True

    def release_write(self):
        self.pending_write = False
        self.update_resource_controls()

    def skill_action(self, action, name=None):
        if self.write_busy():
            self.notify('已有后台任务进行中，先等它结束', error=True)
            return
        name = name if isinstance(name, str) else self.selected_skill_name()
        if not name:
            self.notify('先在 Skills 表里选中一行', error=True)
            return
        if not self.begin_write():
            return
        self.request(['_skills-plan', action, name], self.skill_planned, timeout_ms=120000)

    def skill_planned(self, plan, error):
        if error:
            self.release_write()
            self.notify('无法生成技能计划：' + error, error=True)
            return
        allow_drift = False
        if plan.get('drift'):
            answer = QMessageBox.question(
                self,
                '存在副本漂移',
                '这个技能在某个 agent 目录下有独立副本，内容与主存储不一致。\n'
                '继续会由 skills CLI 覆盖这些副本，但已先快照。仍要继续吗？',
            )
            if answer != QMessageBox.Yes:
                self.release_write()
                return
            allow_drift = True
        verb = '升级' if plan['action'] == 'upgrade' else '删除'
        paths = '\n'.join(plan['paths'])
        text = (
            f'将{verb} {plan["name"]}（{plan.get("source") or "来源未知"}）。\n\n'
            f'影响路径：\n{paths}\n\n命令：{plan["command"]}\n'
            '执行前会写入技能快照；删除不可撤销，但可从快照手动恢复。'
        )
        if QMessageBox.question(self, f'确认{verb}', text) != QMessageBox.Yes:
            self.release_write()
            return
        args = ['_skills-start', plan['action'], plan['name']]
        if allow_drift:
            args.append('--allow-drift')

        def started(info, start_error):
            self.release_write()
            if start_error:
                self.notify('启动技能任务失败：' + start_error, error=True)
                return
            self.active_job = info['job_id']
            self.active_job_project = None
            self.active_job_kind = 'skills'
            self.job_timer.start(500)
            self.footer.setText(f'技能任务已启动 · {plan["name"]}')

        self.request(args, started, timeout_ms=120000)

    def install_skill(self):
        if self.write_busy():
            self.notify('已有后台任务进行中，先等它结束', error=True)
            return
        source, ok = QInputDialog.getText(
            self, '安装技能', 'GitHub 地址、owner/repo，或 skills.sh 上的技能名：'
        )
        if not ok or not source.strip():
            return
        skill, ok = QInputDialog.getText(
            self, '限制技能名（可留空）',
            '仓库里有多个 SKILL.md 时填写目录名；留空表示唯一候选或由预检报出候选：'
        )
        if not ok:
            return
        args = ['_skills-install-plan', source.strip()]
        if skill.strip():
            args += ['--skill', skill.strip()]

        if not self.begin_write():
            return
        def planned(plan, error):
            if error:
                self.release_write()
                self.notify('安装预检失败：' + error, error=True)
                return
            text = (
                f'将安装 {plan["skill"]}（{plan["repo"]}）。\n\n'
                f'候选：{", ".join(plan["candidates"])}\n'
                f'目标 agent：{", ".join(plan["agents"])}\n'
                f'命令：{plan["command"]}\n\n'
                '安装后会校验 lock、主存储与每个 agent 副本。'
            )
            if QMessageBox.question(self, '确认安装', text) != QMessageBox.Yes:
                self.release_write()
                return
            start_args = ['_skills-install-start', source.strip()]
            if skill.strip():
                start_args += ['--skill', skill.strip()]

            def started(info, start_error):
                self.release_write()
                if start_error:
                    self.notify('启动安装失败：' + start_error, error=True)
                    return
                self.active_job = info['job_id']
                self.active_job_project = None
                self.active_job_kind = 'skills'
                self.job_timer.start(500)
                self.footer.setText(f'安装任务已启动 · {plan["skill"]}')

            self.request(start_args, started, timeout_ms=120000)

        self.request(args, planned, timeout_ms=120000)

    def restore_selected(self, name=None):
        if self.write_busy():
            self.notify('已有后台任务进行中，先等它结束', error=True)
            return
        name = name if isinstance(name, str) else self.selected_skill_name()
        if not name:
            self.notify('先在 Skills 表里选中一行', error=True)
            return

        if not self.begin_write():
            return
        def listed(backups, error):
            if error:
                self.release_write()
                self.notify('读取快照失败：' + error, error=True)
                return
            if not backups:
                self.release_write()
                self.notify(f'{name} 没有可用快照', error=True)
                return
            labels = [
                f'{b["id"]} · {b["action"]} · ' + ('含 lock' if b['has_lock'] else '无 lock')
                for b in backups
            ]
            choice, ok = QInputDialog.getItem(
                self, '恢复快照', f'{name} 的快照（新 → 旧）', labels, 0, False
            )
            if not ok:
                self.release_write()
                return
            backup = backups[labels.index(choice)]
            restore_lock = QMessageBox.question(
                self, 'lock 记录',
                '是否同时恢复该技能的 lock 版本记录？\n不恢复则只还原文件，状态会显示与 lock 不一致。',
            ) == QMessageBox.Yes
            text = (
                f'将把 {name} 恢复到快照 {backup["id"]}。\n'
                '当前内容会被覆盖；恢复前不会再快照当前状态。\n'
                + ('会同时写回该技能的 lock 条目。' if restore_lock else '不会修改 lock。')
            )
            if QMessageBox.question(self, '确认恢复', text) != QMessageBox.Yes:
                self.release_write()
                return
            args = ['_skills-restore', name, '--snapshot', backup['id']]
            if restore_lock:
                args.append('--restore-lock')

            def done(data, restore_error):
                self.release_write()
                if restore_error:
                    self.notify('恢复失败：' + restore_error, error=True)
                    return
                self.notify(data.get('message') or '快照已恢复')
                self.load_agents_page()

            self.request(args, done, timeout_ms=120000)

        self.request(['_skills-backups', name], listed, timeout_ms=30000)

    def update_agents_summary(self):
        agents = len(getattr(self, 'agents_data', []))
        inventory = getattr(self, 'skills_inventory', None)
        if not inventory:
            self.agents_summary.setText(
                f'Agent {agents} 个 · Skills 尚未读取'
                if agents else '尚未检测'
            )
            return
        summary = inventory['summary']
        self.agents_summary.setText(
            f"Agent {agents} 个 · Skills {summary['total']} 个"
            f"（纳管 {summary['managed']} / 来源未知 {summary['unmanaged']}）"
            f" · 可更新 {summary.get('update_available', 0)}"
        )

    def check_skills_updates(self):
        self.skills_check_button.setEnabled(False)
        self.skills_check_button.setText('检查中…')

        def done(data, error):
            self.skills_check_button.setEnabled(True)
            self.skills_check_button.setText('检查 Skills 更新')
            if error:
                self.notify('更新检查失败：' + error, error=True)
                return
            self.skills_inventory = data
            self.render_skills()
            self.notify('更新检查完成（缓存 6 小时）')

        self.request(['_skills-check'], done, timeout_ms=180000)

    def open_skills_dir(self):
        self.request(
            ['skills', 'open'],
            lambda _data, error: self.notify(error, error=True) if error else None,
            raw=True,
            timeout_ms=15000,
        )

    def build_logs(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(13)
        layout.addWidget(label('日志中心', 'title'))
        layout.addWidget(label('这里显示可筛选的文件日志；原始颜色、进度与交互输出请打开实时终端。', 'muted'))
        split = QSplitter(Qt.Vertical)
        split.setChildrenCollapsible(False)
        split.addWidget(self.build_alert_list())
        reader = QWidget()
        reader.setMinimumHeight(240)
        log_layout = QVBoxLayout(reader)
        log_layout.setContentsMargins(0, 0, 0, 0)
        log_layout.setSpacing(10)
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
        log_layout.addWidget(controls)
        self.log_view = LogEditor(self.freeze_logs)
        self.log_view.setPlaceholderText('请选择一个服务。尚未启动的服务没有日志。')
        self.highlighter = LogColors(self.log_view.document())
        self.log_view.verticalScrollBar().sliderPressed.connect(self.freeze_logs)
        log_layout.addWidget(self.log_view, 1)
        self.log_summary = label('等待日志', 'muted')
        log_layout.addLayout(row(self.log_summary, None, button('复制所选', self.copy_selected), button('导出筛选结果', self.export_logs)))
        split.addWidget(reader)
        split.setSizes([290, 370])
        layout.addWidget(split, 1)
        self.pages.addWidget(page)

    def build_settings(self):
        page = QWidget()
        self.settings_layout = QVBoxLayout(page)
        self.settings_layout.setContentsMargins(0, 0, 0, 0)
        self.settings_layout.setAlignment(Qt.AlignTop)
        self.pages.addWidget(page)

    def request(self, args, callback, payload=None, raw=False, timeout_ms=30000):
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
            complete(f'操作超过 {max(1, timeout_ms // 1000)} 秒，请检查服务后重试。')
        process.finished.connect(lambda *_: complete())
        process.errorOccurred.connect(lambda e: complete(process.errorString()) if e == QProcess.FailedToStart else None)
        timeout.timeout.connect(timed_out)
        process.start()
        timeout.start(timeout_ms)
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
            if getattr(self, "_alert_refresh_needed", False):
                self._alert_refresh_needed = False
                QTimer.singleShot(0, self.refresh)
        self.request(['_panel-data'], done)
        if self.page == 4:
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
        unread = sum(s['state'] == 'exited_failed' and not s.get('alert_acknowledged') for s in services)
        self.ack_all_button.setEnabled(bool(unread))
        vals = [len(projects), sum(s['state'] in ('running', 'ready') for s in services),
                sum(s['state'] == 'starting' for s in services) + unread,
                sum(s['state'] not in LIVE for s in services) - unread]
        for w, v in zip(self.metrics, vals):
            w.setText(str(v).zfill(2))
        self.hero_caption.setText(f'{len(projects)} 个项目，{len(services)} 项服务 · ' + ('有服务需要关注，查看状态了解详情。' if vals[2] else '当前没有未读异常。'))
        signature = [(s['name'], s['cwd']) for s in services]
        if signature != self.tree_signature:
            self.tree_signature = signature
            self._updating_tree = True
            try:
                collapsed = set(json.loads(str(self.settings.value('collapsed_projects', '[]'))))
            except (ValueError, TypeError):
                collapsed = set()
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
                item.setExpanded(project not in collapsed)
            self._updating_tree = False
            self.sync_project_tree()
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
        self.sync_alert_projects()
        self.render_settings()
        self.update_log_terminal()
        projects_sig = tuple(projects)
        if getattr(self, '_resource_projects_sig', None) != projects_sig:
            self._resource_projects_sig = projects_sig
            self.render_resource_projects()
        if services and not getattr(self, '_asset_loaded', False):
            self._asset_loaded = True
            QTimer.singleShot(200, self.load_asset_summary)
        self.footer.setText(f'● 状态已同步  {datetime.now():%H:%M:%S}    ·    {len(services)} 项服务')

    def filtered_services(self):
        return [s for s in self.data['services'] if self.project is None or s['project'] == self.project]

    def render_services(self):
        if hasattr(self, 'quick_menu'):
            self.quick_menu.sync(self.data)
        services = self.filtered_services()
        if self.selected not in {s['name'] for s in services}:
            self.selected = services[0]['name'] if services else None
        self.update_project_alert_button(self.project_ack_button, self.project)
        self.render_alert_list()
        for b, op in self.bulk_buttons:
            b.setText({'start': '启动', 'stop': '停止', 'restart': '重启'}[op] + ('当前项目' if self.project else '全部'))
        signature = json.dumps([services, self.project, self.selected, sorted(self.action_pending), self.card_columns], sort_keys=True)
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
        last_project = None
        for i, s in enumerate(sorted(services, key=lambda item: (item['project'], item['service']))):
            if s['project'] != last_project:
                last_project = s['project']
                project_ack = button('本项目异常标记已读', lambda checked=False, project=last_project: self.acknowledge_project(project))
                self.update_project_alert_button(project_ack, last_project)
                self.services_layout.addLayout(row(label(last_project, 'sectionTitle'), None, project_ack))
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
        if s['state'] == 'exited_failed' and s.get('alert_acknowledged'):
            state, color, bg = '异常退出 · 已读', '#69766d', '#eef1e9'
        badge = label('●  ' + state, 'badge')
        badge.setStyleSheet(f'color:{color};background:{bg};')
        name = button(s['service'], lambda: self.inspect(s['name']))
        name.setStyleSheet('border:none; padding:0; text-align:left; font-size:16px; font-weight:700; background:transparent;')
        l.addLayout(row(name, None, label(s['project'], 'muted')))
        status_widgets = [badge]
        if unread(s):
            status_widgets.append(self.make_acknowledge_button(s))
        l.addLayout(row(*status_widgets, None))
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
        actions.addWidget(button('分类日志', lambda: self.open_logs(s['name'])))
        l.addLayout(actions)
        terminal = button('实时终端', lambda: self.open_terminal(s['name']))
        terminal.setEnabled(s['state'] != 'not_started')
        terminal.setToolTip('打开 tmux 原始屏幕、实时输出与交互；已退出的会话也可查看')
        preview = button('浏览器 ↗', lambda: self.open_preview(s))
        preview.setEnabled(bool(s.get('url')))
        preview.setToolTip(s.get('url') or '尚无预览地址，请在服务配置中填写 URL 或端口')
        links = [terminal, preview]
        l.addLayout(row(*links, None))
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
        if s['state'] == 'exited_failed':
            status = '异常已读' if s.get('alert_acknowledged') else '异常未读'
            l.addWidget(label(f'{s["service"]} · {status} · exit {s.get("exit_code")}', 'muted', True))
            if unread(s):
                l.addWidget(self.make_acknowledge_button(s))
        for title, value in [('└─ ' + s['service'], s['cwd']), ('启动命令', s['cmd']), ('就绪探测', (s.get('ready') or '') + (f" · TCP {s['port']}" if s.get('port') else '') or '进程存活'), ('预览地址', s.get('url') or '未配置'), ('日志归档', s['log'])]:
            l.addWidget(label(title, 'muted'))
            text = label(value, 'mono', True)
            text.setTextInteractionFlags(Qt.TextSelectableByMouse)
            l.addWidget(text)
        l.addWidget(label('● 自动启动' if s['autostart'] else '○ 手动启动', 'muted'))
        l.addStretch()
        l.addWidget(button('打开工作目录', lambda: QDesktopServices.openUrl(QUrl.fromLocalFile(os.path.expanduser(s['cwd'])))))
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

    def remember_tree_expansion(self, *_args):
        if self._updating_tree or self.project_search.text().strip():
            return
        collapsed = []
        for index in range(self.tree.topLevelItemCount()):
            item = self.tree.topLevelItem(index)
            kind, name = item.data(0, Qt.UserRole)
            if kind == 'project' and not item.isExpanded():
                collapsed.append(name)
        self.settings.setValue('collapsed_projects', json.dumps(collapsed))

    def sync_project_tree(self):
        target = ('project', self.project) if self.project else ('all', '')
        for index in range(self.tree.topLevelItemCount()):
            item = self.tree.topLevelItem(index)
            if item.data(0, Qt.UserRole) == target:
                self.tree.setCurrentItem(item)
                break

    def filter_tree(self):
        query = self.project_search.text().strip().lower()
        if query and self._tree_expanded_before_search is None:
            self._tree_expanded_before_search = {
                self.tree.topLevelItem(i).data(0, Qt.UserRole)[1]: self.tree.topLevelItem(i).isExpanded()
                for i in range(self.tree.topLevelItemCount())
            }
        self._updating_tree = True
        for i in range(self.tree.topLevelItemCount()):
            item = self.tree.topLevelItem(i)
            parent_matches = query in (item.text(0) + item.toolTip(0)).lower()
            matches = parent_matches
            for j in range(item.childCount()):
                child = item.child(j)
                show = parent_matches or query in (child.text(0) + child.toolTip(0)).lower()
                child.setHidden(not show)
                matches = matches or show
            item.setHidden(not matches and bool(query))
            if query and matches:
                item.setExpanded(True)
            elif not query and self._tree_expanded_before_search is not None:
                item.setExpanded(self._tree_expanded_before_search.get(item.data(0, Qt.UserRole)[1], True))
        if not query:
            self._tree_expanded_before_search = None
        self._updating_tree = False

    def select_tree(self, item):
        kind, value = item.data(0, Qt.UserRole)
        if kind == 'all':
            self.project = None
        elif kind == 'project':
            self.project = value
        else:
            self.selected = value
            service = next((s for s in self.data['services'] if s['name'] == value), None)
            if not service:
                return
            self.project = service['project']
            if self.page == 4:
                self.open_logs(value)
        if self.project:
            self.resource_selected = self.project
            self.render_resource_projects()
            self.render_resource_detail()
            if self.page == 2 and self.project not in self.resource_usages:
                self.scan_project(self.project)
        self.render_services()
        self.sync_alert_projects(follow_sidebar=True)

    def inspect(self, name):
        self.selected = name
        self.render_services()
        self.navigate(0)

    def navigate(self, page):
        self.page = page
        self.pages.setCurrentIndex(page)
        for i, b in enumerate(self.nav):
            b.setChecked(i == page)
        self.breadcrumb.setText('工作台 / ' + ['总览', '服务管理', '资源', 'Agent 与 Skills', '日志中心', '设置'][page])
        if page == 2:
            self.load_resources()
        if page == 3:
            self.load_agents_page()
        if page == 4:
            self.refresh_logs()

    def open_logs(self, name):
        self.log_service.setCurrentIndex(self.log_service.findData(name))
        self.navigate(4)

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
        self.dismiss_button.show()

    def acknowledge_service(self, service):
        self.acknowledge_services([service])

    def dismiss_notice(self):
        self.banner.clear()
        self.banner.hide()
        self.dismiss_button.hide()
        if hasattr(self, 'quick_menu'):
            self.quick_menu.notice_label.hide()
            self.quick_menu.notice_timer.stop()
            self.quick_menu.dismiss_notice_button.hide()
        def done(_data, error):
            if error:
                self.notify('清除托盘提示失败：' + error, error=True)
            else:
                self.refresh()
        self.request(['_dismiss-alerts'], done)

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
            panel.present('full', page=4)
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
