"""Resources workspace: project browsing, durable choices, and sequential cleanup jobs."""
from datetime import datetime
from pathlib import Path

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QLabel, QPushButton, QWidget, QFrame, QVBoxLayout, QHBoxLayout, QSplitter,
    QTreeWidget, QTreeWidgetItem, QTableWidget, QTableWidgetItem, QHeaderView,
    QAbstractItemView, QDialog, QDialogButtonBox, QMessageBox, QMenu,
)
from resource_selection import ResourceSelection, cleanable, human_bytes

KIND_LABELS = {
    'node_modules': 'Node 依赖', 'cargo_target': 'Rust 构建', 'python_venv': 'Python 环境',
    'py_cache': 'Python 缓存', 'pytest_cache': 'pytest 缓存', 'mypy_cache': 'mypy 缓存',
    'ruff_cache': 'Ruff 缓存', 'next': 'Next 缓存', 'nuxt': 'Nuxt 缓存',
    'turbo': 'Turbo 缓存', 'parcel_cache': 'Parcel 缓存', 'svelte_kit': 'Svelte 缓存',
    'extra': '自定义',
}


def text(value, name=None, wrap=False):
    widget = QLabel(value)
    widget.setTextFormat(Qt.PlainText)
    widget.setWordWrap(wrap)
    if name:
        widget.setObjectName(name)
    return widget


def button(value, callback, name=None):
    widget = QPushButton(value)
    widget.clicked.connect(callback)
    if name:
        widget.setObjectName(name)
    return widget


def row(*widgets):
    layout = QHBoxLayout()
    layout.setSpacing(8)
    for widget in widgets:
        if widget is None:
            layout.addStretch()
        else:
            layout.addWidget(widget)
    return layout


def pane():
    widget = QFrame()
    widget.setObjectName('resourcePane')
    layout = QVBoxLayout(widget)
    layout.setContentsMargins(12, 12, 12, 12)
    layout.setSpacing(10)
    return widget, layout


class ResourcePanelMixin:
    def init_resource_state(self):
        self.resource_selected = None
        self.resource_usages = {}
        self.resource_scanning = set()
        self.resource_queue = []
        self.res_artifacts = {}
        self.cleanup_selection = ResourceSelection()
        self.cleanup_busy = False
        self.cleanup_plans = []
        self.cleanup_completed = 0
        self.cleanup_stopped = False

    def build_resources(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(12)
        layout.addWidget(text('资源', 'title'))
        layout.addWidget(text('浏览项目目录，将需要清理的内容加入右侧清单。', 'muted'))
        self.res_scan_all = button('扫描全部', self.scan_all)
        self.res_scan_current = button('重新扫描', lambda: self.scan_project(self.resource_selected, refresh=True))
        layout.addLayout(row(self.res_scan_all, self.res_scan_current, button('打开目录', self.open_resource_dir), None))
        self.resource_split = QSplitter(Qt.Horizontal)
        self.resource_split.setChildrenCollapsible(False)
        left, left_layout = pane()
        left.setMinimumWidth(142)
        left_layout.addWidget(text('项目', 'sectionTitle'))
        self.res_projects = QTreeWidget()
        self.res_projects.setObjectName('resourceProjects')
        self.res_projects.setHeaderHidden(True)
        self.res_projects.setRootIsDecorated(False)
        self.res_projects.setIndentation(0)
        self.res_projects.itemClicked.connect(self.select_resource_project)
        left_layout.addWidget(self.res_projects, 1)
        self.resource_split.addWidget(left)

        middle, middle_layout = pane()
        middle.setMinimumWidth(285)
        self.res_title = text('选择项目', 'sectionTitle')
        self.res_caption = text('尚未扫描', 'muted', True)
        middle_layout.addWidget(self.res_title)
        middle_layout.addWidget(self.res_caption)
        self.res_table = QTableWidget(0, 7)
        self.res_table.setHorizontalHeaderLabels(['选择', '类型', '目录', '占用', '逻辑', '文件', '状态'])
        self.res_table.verticalHeader().setVisible(False)
        self.res_table.verticalHeader().setDefaultSectionSize(44)
        self.res_table.setAlternatingRowColors(True)
        self.res_table.setEditTriggers(QAbstractItemView.NoEditTriggers)
        self.res_table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.res_table.setWordWrap(False)
        self.res_table.horizontalHeader().setSectionResizeMode(2, QHeaderView.Stretch)
        for column, width in [(0, 40), (1, 88), (3, 82), (6, 70)]:
            self.res_table.setColumnWidth(column, width)
        self.res_table.setColumnHidden(4, True)
        self.res_table.setColumnHidden(5, True)
        self.res_table.setContextMenuPolicy(Qt.CustomContextMenu)
        self.res_table.customContextMenuRequested.connect(self.resource_context_menu)
        self.res_table.itemChanged.connect(self.update_clean_selection)
        middle_layout.addWidget(self.res_table, 1)
        middle_layout.addWidget(text('右键查看完整路径、逻辑大小与清理依据', 'muted'))
        self.resource_split.addWidget(middle)

        right, right_layout = pane()
        right.setObjectName('cleanupPane')
        right.setMinimumWidth(230)
        right_layout.addWidget(text('待清理清单', 'sectionTitle'))
        right_layout.addWidget(text('跨项目保留选择', 'muted'))
        self.res_total = text('尚未选择目录', 'muted', True)
        self.res_selected_size = text('0 B', 'selectionSize')
        right_layout.addWidget(self.res_selected_size)
        right_layout.addWidget(self.res_total)
        self.res_selection_tree = QTreeWidget()
        self.res_selection_tree.setHeaderLabels(['目录', '占用'])
        self.res_selection_tree.setColumnCount(2)
        self.res_selection_tree.setIndentation(12)
        self.res_selection_tree.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        self.res_selection_tree.header().setSectionResizeMode(0, QHeaderView.Stretch)
        self.res_selection_tree.header().setSectionResizeMode(1, QHeaderView.ResizeToContents)
        self.res_selection_tree.currentItemChanged.connect(self.update_resource_controls)
        right_layout.addWidget(self.res_selection_tree, 1)
        self.res_remove_button = button('移出选中', self.remove_cleanup_selection)
        self.res_clear_button = button('清空', self.clear_cleanup_selection)
        right_layout.addLayout(row(self.res_remove_button, self.res_clear_button))
        self.res_clean_button = button('预览清理计划', self.clean_selected, 'primary')
        right_layout.addWidget(self.res_clean_button)
        self.res_stop_button = button('停止后续项目', self.stop_cleanup_queue)
        self.res_stop_button.setToolTip('当前项目继续执行，尚未启动的项目保留在清单中')
        self.res_stop_button.hide()
        right_layout.addWidget(self.res_stop_button)
        right_layout.addWidget(text('占用不等于可释放空间。执行前会逐项检查并确认。', 'muted', True))
        self.resource_split.addWidget(right)
        self.resource_split.setSizes([165, 570, 275])
        layout.addWidget(self.resource_split, 1)
        self.pages.addWidget(page)
        self.update_resource_controls()

    def load_resources(self):
        names = sorted({s['project'] for s in self.data.get('services', [])})
        if self.project in names:
            self.resource_selected = self.project
        if self.resource_selected not in names:
            self.resource_selected = names[0] if names else None
        def done(data, error):
            if error:
                self.notify('读取扫描缓存失败：' + error, error=True)
                return
            if isinstance(data, list):
                for usage in data:
                    if usage['project'] in names:
                        self.resource_usages[usage['project']] = usage
                        self.cleanup_selection.reconcile(usage)
            self.render_resource_projects()
            self.render_resource_detail()
            if self.resource_selected not in self.resource_usages:
                self.scan_project(self.resource_selected)
        self.request(['_disk-cache'], done, timeout_ms=15000)

    def render_resource_projects(self):
        names = sorted({s['project'] for s in self.data.get('services', [])})
        if self.resource_selected not in names:
            self.resource_selected = names[0] if names else None
        self.res_projects.clear()
        for name in names:
            usage = self.resource_usages.get(name)
            size = human_bytes(usage['total_allocated']) if usage else '未扫描'
            item = QTreeWidgetItem([f'{name}\n{size}'])
            item.setToolTip(0, name + ('\n' + usage['root'] if usage else ''))
            item.setData(0, Qt.UserRole, name)
            self.res_projects.addTopLevelItem(item)
            if name == self.resource_selected:
                self.res_projects.setCurrentItem(item)

    def select_resource_project(self, item, _column=0):
        name = item.data(0, Qt.UserRole)
        if not name:
            return
        self.project = self.resource_selected = name
        self.sync_project_tree()
        self.render_services()
        self.render_resource_detail()
        if name not in self.resource_usages:
            self.scan_project(name)

    def render_resource_detail(self):
        name = self.resource_selected
        self.res_table.blockSignals(True)
        self.res_table.setRowCount(0)
        self.res_artifacts = {}
        usage = self.resource_usages.get(name)
        self.res_title.setText(name or '尚未选择项目')
        if usage:
            dropped = self.cleanup_selection.reconcile(usage)
            if dropped:
                self.notify(f'{name} 的 {dropped} 项选择已失效，请根据新扫描结果重新勾选')
            scanned = datetime.fromtimestamp(usage['scanned_at']).strftime('%m-%d %H:%M')
            source = '推断根' if usage['inferred'] else '显式根'
            caption = f"{usage['root']}\n{source} · {scanned} · 分配 {human_bytes(usage['total_allocated'])}"
            if not usage.get('complete'):
                caption += '\n扫描不完整，暂不允许清理'
            if usage.get('cleaning_reason'):
                caption += '\n' + usage['cleaning_reason']
            self.res_caption.setText(caption)
            for artifact in usage.get('artifacts', []):
                index = self.res_table.rowCount()
                self.res_table.insertRow(index)
                item = QTableWidgetItem('')
                allowed = cleanable(usage, artifact)
                item.setData(Qt.UserRole, artifact['path'])
                if allowed:
                    item.setFlags(Qt.ItemIsEnabled | Qt.ItemIsUserCheckable | Qt.ItemIsSelectable)
                    item.setCheckState(Qt.Checked if self.cleanup_selection.contains(name, artifact['path']) else Qt.Unchecked)
                else:
                    item.setFlags(Qt.ItemIsEnabled)
                    item.setText('—')
                self.res_table.setItem(index, 0, item)
                notes = artifact.get('evidence', {}).get('notes') or []
                cells = [KIND_LABELS.get(artifact['kind'], artifact['kind']), artifact['path'],
                         human_bytes(artifact['allocated']), human_bytes(artifact['logical']),
                         str(artifact['files']), '可清理' if allowed else '只读']
                tooltip = f"{Path(usage['root']) / artifact['path']}\n分配：{cells[2]} / 逻辑：{cells[3]} / 文件：{cells[4]}\n" + '；'.join(notes)
                for column, value in enumerate(cells, 1):
                    cell = QTableWidgetItem(value)
                    cell.setToolTip(tooltip)
                    self.res_table.setItem(index, column, cell)
                self.res_artifacts[artifact['path']] = artifact
        else:
            self.res_caption.setText('正在扫描…' if name in self.resource_scanning else '选择项目后开始扫描。')
        self.res_table.blockSignals(False)
        self.render_cleanup_selection()

    def update_clean_selection(self, *_args):
        if self.cleanup_busy:
            return
        usage = self.resource_usages.get(self.resource_selected)
        if usage:
            for index in range(self.res_table.rowCount()):
                item = self.res_table.item(index, 0)
                artifact = self.res_artifacts.get(item.data(Qt.UserRole))
                if artifact:
                    self.cleanup_selection.set_checked(usage, artifact, item.checkState() == Qt.Checked)
        self.render_cleanup_selection()

    def render_cleanup_selection(self):
        self.res_selection_tree.clear()
        groups = self.cleanup_selection.groups()
        for name, entries in groups.items():
            parent = QTreeWidgetItem([name, ''])
            self.res_selection_tree.addTopLevelItem(parent)
            for entry in entries:
                child = QTreeWidgetItem([entry['path'], human_bytes(entry['allocated'])])
                child.setToolTip(0, entry['absolute_path'])
                child.setData(0, Qt.UserRole, (name, entry['path']))
                parent.addChild(child)
            parent.setExpanded(True)
        count = len(self.cleanup_selection.entries)
        self.res_total.setText(f'已选 {count} 项 · {len(groups)} 个项目' if count else '从中间目录列表勾选，切换项目后仍会保留')
        self.res_selected_size.setText(human_bytes(self.cleanup_selection.allocated))
        self.update_resource_controls()

    def update_resource_controls(self, *_args):
        busy = self.cleanup_busy
        item = self.res_selection_tree.currentItem()
        self.res_remove_button.setEnabled(not busy and bool(item and item.data(0, Qt.UserRole)))
        self.res_clear_button.setEnabled(not busy and bool(self.cleanup_selection.entries))
        self.res_clean_button.setEnabled(not busy and bool(self.cleanup_selection.entries) and not self.write_busy())
        self.res_clean_button.setText('正在处理清理计划…' if busy else '预览清理计划')
        self.res_table.setEnabled(not busy)
        self.res_scan_all.setEnabled(not busy)
        self.res_scan_current.setEnabled(not busy and bool(self.resource_selected))
        self.res_stop_button.setVisible(busy and bool(self.cleanup_plans))

    def remove_cleanup_selection(self):
        item = self.res_selection_tree.currentItem()
        if not self.cleanup_busy and item and item.data(0, Qt.UserRole):
            self.cleanup_selection.remove(item.data(0, Qt.UserRole))
            self.render_resource_detail()

    def clear_cleanup_selection(self):
        if self.cleanup_busy or not self.cleanup_selection.entries:
            return
        if QMessageBox.question(self, '清空待清理清单', '只移除清单中的选择，不删除目录。') == QMessageBox.Yes:
            self.cleanup_selection.clear()
            self.render_resource_detail()

    def resource_context_menu(self, position):
        item = self.res_table.itemAt(position)
        if item is None:
            return
        path = self.res_table.item(item.row(), 2).text()
        menu = QMenu(self)
        menu.addAction('查看目录与清理依据', lambda: QMessageBox.information(self, path, self.res_table.item(item.row(), 2).toolTip()))
        menu.exec(self.res_table.viewport().mapToGlobal(position))

    def scan_project(self, name, refresh=False):
        if not name or name in self.resource_scanning or self.cleanup_busy:
            return
        self.resource_scanning.add(name)
        if name == self.resource_selected:
            self.res_caption.setText(f'正在扫描 {name}…')
        def done(data, error):
            self.resource_scanning.discard(name)
            if error:
                self.notify(f'{name} 扫描失败：{error}', error=True)
            else:
                self.resource_usages[name] = data
                dropped = self.cleanup_selection.reconcile(data)
                if dropped:
                    self.notify(f'{name} 更新后有 {dropped} 项选择失效，已从待清理清单移除')
                self.render_resource_projects()
                self.render_resource_detail()
                self.load_asset_summary()
            self.pump_scan_queue()
        self.request(['_disk-scan', name] + (['--refresh'] if refresh else []), done, timeout_ms=600000)

    def scan_all(self):
        self.resource_queue = sorted({s['project'] for s in self.data.get('services', []) if s['project'] not in self.resource_scanning})
        self.pump_scan_queue()

    def pump_scan_queue(self):
        while self.resource_queue and len(self.resource_scanning) < 2 and not self.cleanup_busy:
            self.scan_project(self.resource_queue.pop(0), refresh=True)

    def open_resource_dir(self):
        if self.resource_selected:
            self.request(['disk', 'open', self.resource_selected], lambda _data, error: self.notify(error, error=True) if error else None, raw=True, timeout_ms=15000)

    def clean_selected(self):
        if self.write_busy():
            self.notify('已有后台任务进行中，先等它结束', error=True)
            return
        groups = self.cleanup_selection.groups()
        if not groups:
            self.notify('先勾选要清理的目录', error=True)
            return
        self.cleanup_busy = True
        self.update_resource_controls()
        pending = list(groups.items())
        plans = []
        def next_plan():
            if not pending:
                current = self.cleanup_selection.groups()
                signature = lambda values: {name: sorted(e['absolute_path'] for e in entries) for name, entries in values.items()}
                if signature(current) != signature(groups):
                    self.finish_cleanup_batch('扫描结果已变化，请重新预览清理计划', error=True)
                elif self.confirm_cleanup(plans):
                    self.cleanup_plans = plans
                    self.cleanup_completed = 0
                    self.cleanup_stopped = False
                    self.start_next_cleanup()
                else:
                    self.finish_cleanup_batch()
                return
            name, entries = pending.pop(0)
            paths = [entry['absolute_path'] for entry in entries]
            args = ['_clean-plan', name]
            for path in paths:
                args.extend(['--path', path])
            def planned(data, error):
                if error:
                    self.finish_cleanup_batch('无法生成清理计划：' + error, error=True)
                    return
                if not isinstance(data, dict) or data.get('project') != name or {c.get('path') for c in data.get('candidates', [])} != set(paths):
                    self.finish_cleanup_batch('清理计划与所选目录不一致，请重新扫描', error=True)
                    return
                plans.append(data)
                next_plan()
            self.request(args, planned, timeout_ms=600000)
        next_plan()

    def confirm_cleanup(self, plans):
        dialog = QDialog(self)
        dialog.setWindowTitle('确认清理计划')
        dialog.resize(760, 480)
        layout = QVBoxLayout(dialog)
        count = sum(len(plan['candidates']) for plan in plans)
        total = sum(plan['total'] for plan in plans)
        layout.addWidget(text(f'{len(plans)} 个项目 · {count} 个目录 · 分配 {human_bytes(total)}', 'sectionTitle'))
        table = QTableWidget(count, 3)
        table.setHorizontalHeaderLabels(['项目', '目录', '分配空间'])
        table.setEditTriggers(QAbstractItemView.NoEditTriggers)
        table.horizontalHeader().setSectionResizeMode(1, QHeaderView.Stretch)
        index = 0
        for plan in plans:
            for candidate in plan['candidates']:
                for column, value in enumerate([plan['project'], candidate['relative'], human_bytes(candidate['allocated'])]):
                    cell = QTableWidgetItem(value)
                    cell.setToolTip(candidate['path'])
                    table.setItem(index, column, cell)
                index += 1
        layout.addWidget(table, 1)
        layout.addWidget(text('删除不可撤销。下次构建或安装依赖时会重建；分配空间不等于实际释放空间。逐项目执行，失败后停止后续项目。', 'muted', True))
        buttons = QDialogButtonBox(QDialogButtonBox.Yes | QDialogButtonBox.Cancel)
        buttons.button(QDialogButtonBox.Yes).setText('确认清理')
        buttons.button(QDialogButtonBox.Cancel).setText('取消')
        buttons.button(QDialogButtonBox.Cancel).setDefault(True)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        # YesRole does not emit accepted on every Qt version.
        buttons.button(QDialogButtonBox.Yes).clicked.connect(dialog.accept)
        layout.addWidget(buttons)
        return dialog.exec() == QDialog.Accepted

    def start_next_cleanup(self):
        if not self.cleanup_plans:
            message = f'清理结束 · {self.cleanup_completed} 个项目完成'
            if self.cleanup_stopped:
                message += '；后续项目已停止，选择仍保留'
            self.finish_cleanup_batch(message)
            return
        plan = self.cleanup_plans.pop(0)
        name = plan['project']
        args = ['_clean-start', name]
        for candidate in plan['candidates']:
            args.extend(['--path', candidate['path']])
        self.update_resource_controls()
        def started(info, error):
            if error or not isinstance(info, dict) or not info.get('job_id'):
                self.finish_cleanup_batch('启动清理失败：' + (error or '缺少任务 ID'), error=True)
                return
            self.active_job = info['job_id']
            self.active_job_project = name
            self.active_job_kind = 'clean'
            self.job_timer.start(500)
            self.footer.setText(f'正在清理 {name} · 后续 {len(self.cleanup_plans)} 个项目')
        self.request(args, started, timeout_ms=120000)

    def stop_cleanup_queue(self):
        self.cleanup_plans.clear()
        self.cleanup_stopped = True
        self.update_resource_controls()
        self.footer.setText('当前项目继续执行；已停止后续项目，剩余选择保留')

    def finish_resource_job(self, data):
        project = self.active_job_project
        result = data.get('result') or {}
        removed = set(result.get('removed', []))
        for key, entry in list(self.cleanup_selection.entries.items()):
            if key[0] == project and entry['absolute_path'] in removed:
                self.cleanup_selection.remove(key)
        self.render_resource_detail()
        self.load_asset_summary()
        if data.get('state') != 'done' or result.get('failed'):
            error = data.get('error') or '部分目录清理失败，后续项目已停止；未完成项保留在清单中'
            self.finish_cleanup_batch(error, error=True)
            self.scan_project(project, refresh=True)
            return
        self.cleanup_completed += 1
        self.start_next_cleanup()
        if not self.cleanup_busy:
            self.scan_project(project, refresh=True)

    def finish_cleanup_batch(self, message=None, error=False):
        self.cleanup_busy = False
        self.cleanup_plans = []
        self.active_job = None
        self.active_job_kind = None
        self.job_timer.stop()
        self.update_resource_controls()
        if message:
            self.notify(message, error=error)
        self.pump_scan_queue()
