"""Git workspace: per-project repositories, changes, history and guarded writes."""
import json
import os

from PySide6.QtCore import Qt, QUrl
from PySide6.QtGui import QColor, QDesktopServices, QKeySequence, QShortcut, QSyntaxHighlighter, QTextCharFormat
from PySide6.QtWidgets import (
    QAbstractItemView, QHeaderView, QLineEdit, QMessageBox, QPlainTextEdit,
    QTabWidget, QTableWidget, QTableWidgetItem, QVBoxLayout, QWidget, QSplitter,
)

from resource_panel import button, row, text

STATUS_LABELS = {
    'M': '修改', 'A': '新增', 'D': '删除', 'R': '重命名', 'C': '复制',
    'T': '类型变更', 'U': '冲突', 'X': '未知',
}


def change_label(change):
    if change.get('untracked'):
        return '未跟踪'
    if change.get('conflict'):
        return '冲突'
    code = change.get('code') or '..'
    parts = []
    if change.get('staged'):
        parts.append(STATUS_LABELS.get(code[0], code[0]))
    if change.get('unstaged') and code[1] not in '.':
        parts.append(STATUS_LABELS.get(code[1], code[1]))
    label = ' / '.join(dict.fromkeys(parts)) or '变更'
    if change.get('staged') and change.get('unstaged'):
        return label + '（部分已暂存）'
    return label + ('（已暂存）' if change.get('staged') else '')


class DiffColors(QSyntaxHighlighter):
    """Tint +/- lines so a diff stays readable in the warm paper theme."""

    def highlightBlock(self, value):
        color = None
        if value.startswith('+++') or value.startswith('---'):
            color = '#5a6b61'
        elif value.startswith('+'):
            color = '#2f6f4e'
        elif value.startswith('-'):
            color = '#a64b3f'
        elif value.startswith('@@'):
            color = '#2d6087'
        elif value.startswith('diff --git') or value.startswith('index '):
            color = '#7a6a45'
        if color:
            fmt = QTextCharFormat()
            fmt.setForeground(QColor(color))
            self.setFormat(0, len(value), fmt)


class GitPanelMixin:
    def init_git_state(self):
        self.git_repos = []
        self.git_current = None
        self.git_log = []
        self.git_log_pending = False
        self.git_selected_path = None
        self.git_changes_signature = None
        self.git_repos_signature = None
        self.git_busy = False
        self.git_loaded = False
        self.git_messages = {}
        self.git_unchecked = set()
        self.git_batch_summary = None
        self.git_batch_items = []

    # ── 构建界面 ────────────────────────────────────────────────────────────

    def build_git(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(12)
        layout.addWidget(text('Git', 'title'))
        layout.addWidget(text('按项目仓库查看变更与历史。拉取只做 fast-forward；批量拉取会跳过有未提交改动的仓库。', 'muted', True))
        self.git_pull_all_button = button('拉取全部项目', self.git_pull_all, 'primary')
        self.git_refresh_button = button('刷新', self.git_reload)
        self.git_search = QLineEdit()
        self.git_search.setPlaceholderText('筛选项目…')
        self.git_search.setClearButtonEnabled(True)
        self.git_search.textChanged.connect(lambda _value: self.render_git_repos())
        self.git_summary = text('尚未读取', 'muted')
        layout.addLayout(row(self.git_pull_all_button, self.git_refresh_button, self.git_search, None, self.git_summary))

        split = QSplitter(Qt.Horizontal)
        split.setChildrenCollapsible(False)

        left = QWidget()
        left_layout = QVBoxLayout(left)
        left_layout.setContentsMargins(0, 0, 8, 0)
        left_layout.setSpacing(8)
        left_layout.addWidget(text('仓库', 'sectionTitle'))
        self.git_repo_table = QTableWidget(0, 2)
        self.git_repo_table.setHorizontalHeaderLabels(['项目', '状态'])
        self.git_repo_table.verticalHeader().setVisible(False)
        self.git_repo_table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.git_repo_table.setSelectionMode(QAbstractItemView.SingleSelection)
        self.git_repo_table.setEditTriggers(QAbstractItemView.NoEditTriggers)
        self.git_repo_table.setAlternatingRowColors(True)
        self.git_repo_table.setWordWrap(False)
        self.git_repo_table.horizontalHeader().setSectionResizeMode(0, QHeaderView.Stretch)
        self.git_repo_table.horizontalHeader().setSectionResizeMode(1, QHeaderView.Fixed)
        self.git_repo_table.setColumnWidth(1, 112)
        self.git_repo_table.itemClicked.connect(self.select_git_repo)
        left_layout.addWidget(self.git_repo_table, 1)
        self.git_batch_label = text('', 'muted', True)
        left_layout.addWidget(self.git_batch_label)
        self.git_batch_detail_button = button('查看批量拉取详情', self.show_git_batch_details)
        self.git_batch_detail_button.hide()
        left_layout.addWidget(self.git_batch_detail_button)
        left.setMinimumWidth(200)
        split.addWidget(left)

        middle = QWidget()
        middle_layout = QVBoxLayout(middle)
        middle_layout.setContentsMargins(12, 0, 12, 0)
        middle_layout.setSpacing(8)
        self.git_tabs = QTabWidget()
        changes = QWidget()
        changes_layout = QVBoxLayout(changes)
        changes_layout.setContentsMargins(10, 10, 10, 10)
        changes_layout.setSpacing(8)
        self.git_changes_hint = text('选择左侧仓库查看变更', 'muted')
        changes_layout.addWidget(self.git_changes_hint)
        self.git_change_table = QTableWidget(0, 3)
        self.git_change_table.setHorizontalHeaderLabels(['选择', '状态', '文件'])
        self.git_change_table.verticalHeader().setVisible(False)
        self.git_change_table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.git_change_table.setSelectionMode(QAbstractItemView.SingleSelection)
        self.git_change_table.setEditTriggers(QAbstractItemView.NoEditTriggers)
        self.git_change_table.setAlternatingRowColors(True)
        self.git_change_table.horizontalHeader().setSectionResizeMode(2, QHeaderView.Stretch)
        self.git_change_table.setColumnWidth(0, 46)
        self.git_change_table.setColumnWidth(1, 116)
        self.git_change_table.itemChanged.connect(self.update_git_commit_state)
        self.git_change_table.itemClicked.connect(self.select_git_change)
        changes_layout.addWidget(self.git_change_table, 1)
        actions = row(button('全选', lambda: self.set_all_git_checks(True)), button('清空选择', lambda: self.set_all_git_checks(False)), None)
        changes_layout.addLayout(actions)
        self.git_message = QPlainTextEdit()
        self.git_message.setPlaceholderText('提交信息（Ctrl+Enter 提交）')
        self.git_message.setFixedHeight(68)
        self.git_message.textChanged.connect(self.update_git_commit_state)
        changes_layout.addWidget(self.git_message)
        self.git_commit_button = button('提交选中文件', self.git_commit, 'primary')
        self.git_commit_hint = text('', 'muted')
        changes_layout.addLayout(row(self.git_commit_button, None))
        changes_layout.addWidget(self.git_commit_hint)
        self.git_tabs.addTab(changes, '变更')

        history = QWidget()
        history_layout = QVBoxLayout(history)
        history_layout.setContentsMargins(10, 10, 10, 10)
        history_layout.setSpacing(8)
        self.git_log_table = QTableWidget(0, 4)
        self.git_log_table.setHorizontalHeaderLabels(['提交', '日期', '作者', '说明'])
        self.git_log_table.verticalHeader().setVisible(False)
        self.git_log_table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.git_log_table.setSelectionMode(QAbstractItemView.SingleSelection)
        self.git_log_table.setEditTriggers(QAbstractItemView.NoEditTriggers)
        self.git_log_table.setAlternatingRowColors(True)
        self.git_log_table.horizontalHeader().setSectionResizeMode(3, QHeaderView.Stretch)
        self.git_log_table.setColumnWidth(0, 84)
        self.git_log_table.setColumnWidth(1, 88)
        self.git_log_table.setColumnWidth(2, 100)
        self.git_log_table.itemClicked.connect(self.select_git_commit)
        history_layout.addWidget(self.git_log_table, 1)
        self.git_log_hint = text('', 'muted')
        history_layout.addWidget(self.git_log_hint)
        self.git_tabs.addTab(history, '历史')
        middle_layout.addWidget(self.git_tabs, 1)
        middle.setMinimumWidth(290)
        split.addWidget(middle)

        right = QWidget()
        right_layout = QVBoxLayout(right)
        right_layout.setContentsMargins(8, 0, 0, 0)
        right_layout.setSpacing(8)
        self.git_repo_title = text('未选择仓库', 'sectionTitle')
        self.git_repo_caption = text('', 'mono', True)
        self.git_repo_state = text('', 'muted', True)
        right_layout.addWidget(self.git_repo_title)
        right_layout.addWidget(self.git_repo_caption)
        right_layout.addWidget(self.git_repo_state)
        self.git_pull_button = button('拉取', self.git_pull)
        self.git_pull_button.setToolTip('拉取远端更新，只允许 fast-forward')
        self.git_push_button = button('推送', self.git_push)
        self.git_push_button.setToolTip('推送到已有上游，绝不 force')
        self.git_open_button = button('目录', self.git_open_dir)
        self.git_open_button.setToolTip('用文件管理器打开仓库目录')
        right_layout.addLayout(row(self.git_pull_button, self.git_push_button, self.git_open_button, None))
        self.git_diff_title = text('改动预览', 'sectionTitle')
        self.git_diff_title.setTextInteractionFlags(Qt.TextSelectableByMouse)
        right_layout.addWidget(self.git_diff_title)
        self.git_diff_view = QPlainTextEdit()
        self.git_diff_view.setReadOnly(True)
        self.git_diff_view.setLineWrapMode(QPlainTextEdit.NoWrap)
        self.git_diff_view.setObjectName('diffView')
        self.git_diff_view.setPlaceholderText('选择仓库或变更查看差异。')
        self.git_diff_highlight = DiffColors(self.git_diff_view.document())
        right_layout.addWidget(self.git_diff_view, 1)
        right.setMinimumWidth(230)
        split.addWidget(right)

        split.setSizes([260, 410, 520])
        layout.addWidget(split, 1)
        self.pages.addWidget(page)

        shortcut = QShortcut(QKeySequence('Ctrl+Return'), self.git_message)
        shortcut.setContext(Qt.WidgetShortcut)
        shortcut.activated.connect(self.git_commit)

    # ── 数据加载 ────────────────────────────────────────────────────────────

    def git_reload(self):
        self.git_loaded = False
        self.load_git_page()

    def load_git_page(self):
        if self.git_busy:
            return
        self.git_refresh_button.setEnabled(False)
        self.git_refresh_button.setText('刷新中…')
        self.git_summary.setText('正在读取各项目仓库…')

        def done(data, error):
            self.git_refresh_button.setEnabled(True)
            self.git_refresh_button.setText('刷新')
            if error:
                self.git_summary.setText('状态读取失败，保留上次结果')
                self.notify('Git 状态读取失败：' + error, error=True)
                return
            repos = data.get('repos', []) if isinstance(data, dict) else []
            self.git_repos = repos
            self.git_loaded = True
            if not any(repo['root'] == self.git_current for repo in repos):
                self.git_current = next((repo['root'] for repo in repos if repo['is_repo']), repos[0]['root'] if repos else None)
                self.git_selected_path = None
            self.render_git_repos()
            self.render_git_current()
            if self.git_current:
                self.load_git_log()

        self.request(['git', 'status', '--json'], done, timeout_ms=60000)

    def current_git_repo(self):
        return next((repo for repo in self.git_repos if repo['root'] == self.git_current), None)

    def load_git_log(self):
        repo = self.current_git_repo()
        if not repo or not repo['is_repo'] or repo.get('error'):
            self.git_log = []
            self.render_git_log()
            return
        self.git_log_pending = True
        self.git_log_hint.setText('正在读取提交历史…')

        def done(data, error):
            self.git_log_pending = False
            if error:
                self.git_log = []
                self.git_log_hint.setText('历史读取失败：' + error)
                self.render_git_log()
                return
            self.git_log = data.get('commits', []) if isinstance(data, dict) else []
            self.render_git_log()

        self.request(['git', 'log', repo['project'], '-n', '30', '--json'], done, timeout_ms=30000)

    # ── 渲染 ────────────────────────────────────────────────────────────────

    def visible_git_repos(self):
        query = self.git_search.text().strip().lower() if hasattr(self, 'git_search') else ''
        if not query:
            return self.git_repos
        return [repo for repo in self.git_repos if query in ' '.join(repo['projects']).lower() or query in repo['root'].lower()]

    def repo_state_text(self, repo):
        if not repo['is_repo']:
            return '非 Git 仓库'
        if repo.get('error'):
            return '读取失败'
        parts = []
        if repo['detached']:
            parts.append('分离 HEAD')
        else:
            parts.append(repo['branch'] or '无分支')
        if repo['ahead']:
            parts.append(f'↑{repo["ahead"]}')
        if repo['behind']:
            parts.append(f'↓{repo["behind"]}')
        if repo.get('operation'):
            parts.append(repo['operation'])
        parts.append(f'{len(repo["changes"])} 处变更' if repo['changes'] else '干净')
        return ' · '.join(parts)

    def repo_badge_text(self, repo):
        if not repo['is_repo']:
            return '非 Git'
        if repo.get('error'):
            return '读取失败'
        sync = ''
        if repo['ahead']:
            sync += f'↑{repo["ahead"]}'
        if repo['behind']:
            sync += f'↓{repo["behind"]}'
        changes = f'{len(repo["changes"])} 项' if repo['changes'] else ''
        if sync and changes:
            return f'{sync} · {changes}'
        return sync or changes or '干净'

    def render_git_repos(self):
        repos = self.visible_git_repos()
        signature = json.dumps([[repo['root'], repo['is_repo'], repo.get('error'), repo['branch'], repo.get('upstream'), repo['ahead'], repo['behind'], len(repo['changes']), repo.get('operation'), repo.get('warning')] for repo in repos], sort_keys=True)
        if signature == self.git_repos_signature:
            self.update_git_controls()
            return
        self.git_repos_signature = signature
        table = self.git_repo_table
        table.blockSignals(True)
        table.setRowCount(0)
        selected_row = -1
        for index, repo in enumerate(repos):
            table.insertRow(index)
            name = '、'.join(repo['projects'])
            item = QTableWidgetItem(name)
            item.setData(Qt.UserRole, repo['root'])
            item.setToolTip(repo['root'])
            if not repo['is_repo']:
                item.setForeground(QColor('#8a9790'))
            table.setItem(index, 0, item)
            state = QTableWidgetItem(self.repo_badge_text(repo))
            state.setToolTip(self.repo_state_text(repo) + '\n' + (repo.get('error') or repo['root']))
            if not repo['is_repo']:
                state.setForeground(QColor('#8a9790'))
            elif repo.get('error'):
                state.setForeground(QColor('#b3584e'))
            elif repo['dirty']:
                state.setForeground(QColor('#b48027'))
            else:
                state.setForeground(QColor('#3c8060'))
            table.setItem(index, 1, state)
            if repo['root'] == self.git_current:
                selected_row = index
        table.blockSignals(False)
        if repos:
            self.git_repo_table.selectRow(selected_row if selected_row >= 0 else 0)
        writable = [repo for repo in self.git_repos if repo['is_repo'] and not repo.get('error')]
        self.git_summary.setText(
            f'{len(self.git_repos)} 个仓库 · {sum(1 for repo in writable if repo["dirty"])} 个有变更'
            f' · {sum(1 for repo in writable if repo["behind"])} 个可拉取'
            if self.git_repos else '配置里还没有项目'
        )
        self.git_pull_all_button.setEnabled(bool(writable) and not self.git_busy and not self.write_busy())
        if self.git_batch_summary:
            self.git_batch_label.setText(self.git_batch_summary)
        self.update_git_controls()

    def render_git_current(self):
        repo = self.current_git_repo()
        if not repo:
            self.git_repo_title.setText('未选择仓库')
            self.git_repo_caption.setText('在配置中添加项目后，这里会显示对应 Git 仓库。')
            self.git_repo_state.setText('')
            self.git_change_table.setRowCount(0)
            self.git_diff_view.clear()
            self.update_git_controls()
            return
        self.git_repo_title.setText('、'.join(repo['projects']))
        self.git_repo_caption.setText(repo['root'])
        if not repo['is_repo']:
            self.git_repo_state.setText(repo.get('error') or '不是 Git 仓库；无法在这里执行 Git 操作。')
        else:
            detail = []
            detail.append('分离 HEAD' if repo['detached'] else f'分支 {repo["branch"] or "（无）"}')
            if repo['upstream']:
                detail.append(f'上游 {repo["upstream"]}')
            else:
                detail.append('没有上游分支')
            if repo['ahead'] or repo['behind']:
                detail.append(f'领先 {repo["ahead"]} / 落后 {repo["behind"]}')
            if repo.get('operation'):
                detail.append(repo['operation'])
            if repo.get('warning'):
                detail.append(repo['warning'])
            if repo.get('error'):
                detail.append('读取失败：' + repo['error'])
            self.git_repo_state.setText(' · '.join(detail))
        self.render_git_changes()
        self.render_git_log()
        self.update_git_controls()

    def render_git_changes(self):
        repo = self.current_git_repo()
        changes = repo['changes'] if repo and repo['is_repo'] and not repo.get('error') else []
        self.git_unchecked &= {change['path'] for change in changes}
        signature = json.dumps([[change['path'], change['code'], change['untracked'], change['conflict'], change['staged'], change['unstaged']] for change in changes], sort_keys=True)
        if signature != self.git_changes_signature:
            self.git_changes_signature = signature
            table = self.git_change_table
            table.blockSignals(True)
            table.setRowCount(0)
            for index, change in enumerate(changes):
                table.insertRow(index)
                check = QTableWidgetItem('')
                check.setFlags(Qt.ItemIsUserCheckable | Qt.ItemIsEnabled | Qt.ItemIsSelectable)
                check.setCheckState(Qt.Unchecked if change['path'] in self.git_unchecked else Qt.Checked)
                check.setData(Qt.UserRole, change['path'])
                table.setItem(index, 0, check)
                status = QTableWidgetItem(change_label(change))
                if change['conflict']:
                    status.setForeground(QColor('#b3584e'))
                elif change['untracked']:
                    status.setForeground(QColor('#8a6a2f'))
                table.setItem(index, 1, status)
                path = QTableWidgetItem(change['path'] + (f'  ←  {change["orig"]}' if change.get('orig') else ''))
                path.setToolTip(change['path'])
                table.setItem(index, 2, path)
            table.blockSignals(False)
            if changes and not any(change['path'] == self.git_selected_path for change in changes):
                self.git_selected_path = None
            if changes and not self.git_selected_path:
                self.git_selected_path = changes[0]['path']
            if self.git_selected_path:
                for index, change in enumerate(changes):
                    if change['path'] == self.git_selected_path:
                        self.git_change_table.selectRow(index)
                        break
            else:
                self.git_change_table.clearSelection()
            if self.git_selected_path:
                self.git_diff_for(self.git_selected_path)
            else:
                self.git_diff_title.setText('改动预览')
                self.git_diff_view.clear()
                self.git_diff_view.setPlaceholderText('在上方变更列表中选择文件查看差异。')
        if not repo or not repo['is_repo']:
            self.git_changes_hint.setText('选择左侧仓库查看变更')
        elif repo.get('error'):
            self.git_changes_hint.setText('仓库读取失败，暂不能查看变更')
        elif not changes:
            self.git_changes_hint.setText('工作区干净，没有可提交的改动')
        else:
            self.git_changes_hint.setText(f'{len(changes)} 个文件有改动；勾选后填写提交信息')

    def render_git_log(self):
        if self.git_log_pending:
            return
        table = self.git_log_table
        table.blockSignals(True)
        table.setRowCount(0)
        repo = self.current_git_repo()
        for index, commit in enumerate(self.git_log):
            table.insertRow(index)
            short = QTableWidgetItem(commit['short'])
            short.setToolTip(commit['hash'])
            table.setItem(index, 0, short)
            table.setItem(index, 1, QTableWidgetItem(commit['date']))
            table.setItem(index, 2, QTableWidgetItem(commit['author']))
            subject = commit['subject'] + (f'  ({commit["decorations"]})' if commit.get('decorations') else '')
            item = QTableWidgetItem(subject)
            item.setToolTip(commit['hash'])
            table.setItem(index, 3, item)
        table.blockSignals(False)
        if not repo or not repo['is_repo']:
            self.git_log_hint.setText('')
        elif not self.git_log:
            self.git_log_hint.setText('这个仓库还没有提交')
        else:
            self.git_log_hint.setText(f'{len(self.git_log)} 条最近提交；点击查看详情')

    # ── 交互 ────────────────────────────────────────────────────────────────

    def select_git_repo(self, item):
        root = item.data(Qt.UserRole) or self.git_repo_table.item(item.row(), 0).data(Qt.UserRole)
        if not root or root == self.git_current:
            return
        self.remember_git_message()
        self.git_current = root
        self.git_selected_path = None
        self.git_unchecked.clear()
        self.git_diff_view.clear()
        self.git_changes_signature = None
        self.git_log = []
        self.git_log_table.setRowCount(0)
        repo = self.current_git_repo()
        if repo:
            self.git_message.blockSignals(True)
            self.git_message.setPlainText(self.git_messages.get(root, ''))
            self.git_message.blockSignals(False)
        self.render_git_current()
        self.load_git_log()
        self.update_git_commit_state()

    def remember_git_message(self):
        if self.git_current:
            self.git_messages[self.git_current] = self.git_message.toPlainText()

    def select_git_change(self, item):
        path_item = self.git_change_table.item(item.row(), 2)
        if not path_item:
            return
        change = self.git_change_table.item(item.row(), 0)
        path = change.data(Qt.UserRole) if change else path_item.toolTip()
        if path:
            self.git_selected_path = path
            self.git_diff_for(path)

    def git_diff_for(self, path):
        repo = self.current_git_repo()
        if not repo or not repo['is_repo'] or repo.get('error'):
            return
        self.git_diff_title.setText('改动预览 · ' + os.path.basename(path))
        self.git_diff_title.setToolTip(path)
        self.git_diff_view.setPlainText('读取中…')

        def done(data, error):
            if error:
                self.git_diff_view.setPlainText('无法读取差异：' + error)
                return
            self.git_diff_view.setPlainText(data.get('diff') or '（没有差异）')

        self.request(['git', 'diff', repo['project'], '--path', path, '--json'], done, timeout_ms=60000)

    def select_git_commit(self, item):
        row = item.row()
        commit = self.git_log[row] if 0 <= row < len(self.git_log) else None
        if not commit:
            return
        self.git_diff_title.setText('提交详情 · ' + commit['short'])
        lines = [
            f'提交  {commit["hash"]}',
            f'作者  {commit["author"]}',
            f'日期  {commit["date"]}',
            '',
            commit['subject'],
        ]
        if commit.get('decorations'):
            lines.insert(4, f'引用  {commit["decorations"]}')
        self.git_diff_view.setPlainText('\n'.join(lines))

    def set_all_git_checks(self, checked):
        state = Qt.Checked if checked else Qt.Unchecked
        self.git_change_table.blockSignals(True)
        for row in range(self.git_change_table.rowCount()):
            item = self.git_change_table.item(row, 0)
            if item:
                item.setCheckState(state)
        self.git_change_table.blockSignals(False)
        self.update_git_commit_state()

    def checked_git_paths(self):
        paths = []
        for row in range(self.git_change_table.rowCount()):
            item = self.git_change_table.item(row, 0)
            if item and item.checkState() == Qt.Checked:
                paths.append(item.data(Qt.UserRole))
        return paths

    def update_git_commit_state(self, *_args):
        for row in range(self.git_change_table.rowCount()):
            item = self.git_change_table.item(row, 0)
            if not item:
                continue
            path = item.data(Qt.UserRole)
            if item.checkState() == Qt.Checked:
                self.git_unchecked.discard(path)
            else:
                self.git_unchecked.add(path)
        repo = self.current_git_repo()
        writable = bool(repo and repo['is_repo'] and not repo.get('error'))
        busy = self.git_busy or bool(self.active_job)
        paths = self.checked_git_paths()
        message = self.git_message.toPlainText().strip()
        self.git_commit_button.setEnabled(writable and bool(paths) and bool(message) and not busy)
        self.git_commit_button.setText('提交选中文件' if len(paths) != 1 else '提交 1 个文件')
        if not writable:
            self.git_commit_hint.setText('当前仓库不可写')
        elif not paths:
            self.git_commit_hint.setText('勾选要提交的文件')
        elif not message:
            self.git_commit_hint.setText('填写提交信息')
        else:
            self.git_commit_hint.setText(f'{len(paths)} 个文件 · Ctrl+Enter')
        self.git_pull_button.setEnabled(writable and not busy)
        self.git_push_button.setEnabled(writable and not busy and bool(repo and repo['upstream']) and not (repo and repo['detached']))
        self.git_open_button.setEnabled(bool(repo))
        if hasattr(self, 'git_pull_all_button'):
            self.git_pull_all_button.setEnabled(any(r['is_repo'] and not r.get('error') for r in self.git_repos) and not busy)
        self.git_change_table.setEnabled(writable and not busy)
        self.git_message.setReadOnly(not writable)

    def update_git_controls(self):
        self.update_git_commit_state()

    def git_open_dir(self):
        repo = self.current_git_repo()
        if repo:
            QDesktopServices.openUrl(QUrl.fromLocalFile(repo['root']))

    # ── 写操作 ──────────────────────────────────────────────────────────────

    def git_commit(self):
        repo = self.current_git_repo()
        if not repo or self.git_busy or self.write_busy():
            self.notify('已有操作进行中，先等它结束', error=True)
            return
        paths = self.checked_git_paths()
        message = self.git_message.toPlainText().strip()
        if not paths:
            self.notify('先勾选要提交的文件', error=True)
            return
        if not message:
            self.notify('提交信息不能为空', error=True)
            return
        self.remember_git_message()
        self.git_busy = True
        self.update_git_controls()
        self.git_commit_button.setText('提交中…')

        def done(data, error):
            self.git_busy = False
            self.git_commit_button.setText('提交选中文件')
            if error:
                self.notify('提交失败：' + error, error=True)
                self.update_git_controls()
                return
            self.git_messages.pop(repo['root'], None)
            self.git_message.blockSignals(True)
            self.git_message.clear()
            self.git_message.blockSignals(False)
            self.git_selected_path = None
            self.git_unchecked.clear()
            self.git_changes_signature = None
            self.notify(f'已提交 {data["short"]}：{data["subject"]}（{data["files"]} 个文件）')
            self.load_git_page()

        self.request(['_git-commit'], done, {
            'project': repo['project'],
            'message': message,
            'paths': paths,
            'all': False,
        }, timeout_ms=120000)

    def git_pull(self):
        repo = self.current_git_repo()
        if not repo or self.git_busy or self.write_busy():
            self.notify('已有操作进行中，先等它结束', error=True)
            return
        self.git_busy = True
        self.update_git_controls()
        self.git_pull_button.setText('拉取中…')

        def done(data, error):
            self.git_busy = False
            self.git_pull_button.setText('拉取')
            if error:
                self.notify('拉取失败：' + error, error=True)
            else:
                self.notify((data.get('message') or '拉取完成') + (f'（{data["warning"]}）' if data.get('warning') else ''))
            self.load_git_page()

        self.request(['git', 'pull', repo['project'], '--json'], done, timeout_ms=120000)

    def git_push(self):
        repo = self.current_git_repo()
        if not repo or self.git_busy or self.write_busy():
            self.notify('已有操作进行中，先等它结束', error=True)
            return
        self.git_busy = True
        self.update_git_controls()
        self.git_push_button.setText('推送中…')

        def done(data, error):
            self.git_busy = False
            self.git_push_button.setText('推送')
            if error:
                self.notify('推送失败：' + error, error=True)
            else:
                self.notify(data.get('message') or '推送完成')
            self.load_git_page()

        self.request(['git', 'push', repo['project'], '--json'], done, timeout_ms=120000)

    def git_pull_all(self):
        if self.write_busy():
            self.notify('已有后台任务或写操作进行中，先等它结束', error=True)
            return
        self.git_pull_all_button.setEnabled(False)
        self.git_pull_all_button.setText('准备拉取…')
        self.git_batch_detail_button.hide()

        def done(data, error):
            self.git_pull_all_button.setText('拉取全部项目')
            if error:
                self.notify('批量拉取启动失败：' + error, error=True)
                self.update_git_controls()
                return
            self.active_job = data['job_id']
            self.active_job_project = None
            self.active_job_kind = 'git'
            self.git_batch_summary = f'批量拉取进行中：{data["count"]} 个仓库'
            self.git_batch_label.setText(self.git_batch_summary)
            self.notify('已开始批量拉取，完成后会汇总结果')
            self.job_timer.start(500)
            self.update_git_controls()

        self.request(['_git-pull-start'], done, timeout_ms=30000)

    def finish_git_job(self, job):
        self.git_busy = False
        if job.get('state') != 'done':
            self.git_batch_summary = None
            self.git_batch_items = []
            self.git_batch_detail_button.hide()
            self.notify(job.get('error') or f'批量拉取未完成：{job.get("state")}', error=True)
            self.update_git_controls()
            return
        result = job.get('result') or {}
        items = result.get('items') or []
        summary = result.get('summary') or {}
        message = result.get('message') or '批量拉取结束'
        self.git_batch_items = items
        self.git_batch_summary = '上次批量拉取：' + message
        if self.git_batch_label:
            self.git_batch_label.setText(self.git_batch_summary)
        self.git_batch_detail_button.setVisible(bool(items))
        self.notify(message, error=bool(summary.get('failed')))
        self.git_loaded = False
        self.load_git_page()

    def show_git_batch_details(self):
        if not self.git_batch_items:
            return
        labels = {'updated': '已更新', 'uptodate': '已最新', 'skipped': '跳过', 'failed': '失败'}
        lines = []
        for item in self.git_batch_items:
            detail = item.get('reason') or ''
            if item.get('outcome') == 'updated' and item.get('commits'):
                detail = f'{item["commits"]} 个提交'
            lines.append(f'{item["project"]}：{labels.get(item["outcome"], item["outcome"])}{(" · " + detail) if detail else ""}')
        QMessageBox.information(self, 'devtray · 批量拉取结果', '\n'.join(lines))
