"""Project-scoped acknowledgement and the log centre's current exception queue."""
from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QAbstractItemView, QComboBox, QFrame, QHBoxLayout, QHeaderView, QLabel,
    QPushButton, QTableWidget, QTableWidgetItem, QVBoxLayout, QWidget,
)


def unread(service):
    return service.get('state') == 'exited_failed' and not service.get('alert_acknowledged')


def text_label(text, name=None):
    result = QLabel(text)
    result.setTextFormat(Qt.PlainText)
    if name:
        result.setObjectName(name)
    return result


def action_button(text, callback):
    result = QPushButton(text)
    result.setCursor(Qt.PointingHandCursor)
    result.clicked.connect(callback)
    return result


class ServiceAlertsMixin:
    def make_acknowledge_button(self, service):
        result = action_button('标记已读', lambda: self.acknowledge_service(service))
        result.setAccessibleName('标记已读 ' + service['name'])
        result.setToolTip('确认本次异常并解除托盘提醒，保留退出码与日志')
        result.setEnabled(unread(service) and service['name'] not in self.action_pending)
        return result

    def update_project_alert_button(self, control, project):
        failed = [s for s in self.data['services'] if s['project'] == project and s.get('state') == 'exited_failed']
        pending = [s for s in failed if unread(s)]
        control.setText(f'本项目异常标记已读 ({len(pending)})' if pending else '本项目异常已读')
        control.setAccessibleName('项目异常标记已读 ' + str(project))
        control.setToolTip('只确认此项目当前的异常，不影响其他项目或新的运行异常')
        control.setVisible(bool(failed))
        control.setEnabled(bool(pending) and not any(s['name'] in self.action_pending for s in failed))

    def build_alert_list(self):
        frame = QFrame()
        frame.setObjectName('card')
        frame.setMinimumHeight(240)
        layout = QVBoxLayout(frame)
        layout.setContentsMargins(14, 12, 14, 12)
        layout.setSpacing(8)
        heading = QHBoxLayout()
        heading.addWidget(text_label('异常处理', 'sectionTitle'))
        self.alert_count = text_label('暂无异常', 'muted')
        heading.addWidget(self.alert_count)
        heading.addStretch()
        self.alert_project = QComboBox()
        self.alert_project.setAccessibleName('异常项目筛选')
        self.alert_project.addItem('全部项目', None)
        self.alert_project.setMinimumWidth(140)
        self.alert_project.currentIndexChanged.connect(self.render_alert_list)
        heading.addWidget(self.alert_project)
        self.alert_status = QComboBox()
        self.alert_status.setAccessibleName('异常处理状态筛选')
        for title, value in [('全部异常', 'all'), ('未读', 'unread'), ('已读', 'read')]:
            self.alert_status.addItem(title, value)
        self.alert_status.currentIndexChanged.connect(self.render_alert_list)
        heading.addWidget(self.alert_status)
        layout.addLayout(heading)
        self.alert_note = text_label('列出当前保留的异常退出；标记已读会保留退出码与日志。', 'muted')
        layout.addWidget(self.alert_note)
        self.alert_table = QTableWidget(0, 5)
        self.alert_table.setAccessibleName('服务异常处理列表')
        self.alert_table.setHorizontalHeaderLabels(['项目', '服务', '异常原因', '处理状态', '操作'])
        self.alert_table.verticalHeader().setVisible(False)
        self.alert_table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.alert_table.setSelectionMode(QAbstractItemView.SingleSelection)
        self.alert_table.setEditTriggers(QAbstractItemView.NoEditTriggers)
        self.alert_table.setAlternatingRowColors(True)
        for column, width in [(0, 120), (1, 100), (3, 90), (4, 218)]:
            self.alert_table.setColumnWidth(column, width)
        self.alert_table.horizontalHeader().setSectionResizeMode(2, QHeaderView.Stretch)
        self.alert_table.verticalHeader().setDefaultSectionSize(48)
        layout.addWidget(self.alert_table, 1)
        self.alert_empty = text_label('当前没有异常退出。', 'muted')
        self.alert_empty.setWordWrap(True)
        layout.addWidget(self.alert_empty)
        self._alert_signature = None
        return frame

    def sync_alert_projects(self, follow_sidebar=False):
        projects = sorted({s['project'] for s in self.data['services']})
        selected = self.project if follow_sidebar else self.alert_project.currentData()
        if [self.alert_project.itemData(i) for i in range(1, self.alert_project.count())] != projects:
            self.alert_project.blockSignals(True)
            self.alert_project.clear()
            self.alert_project.addItem('全部项目', None)
            for project in projects:
                self.alert_project.addItem(project, project)
            self.alert_project.blockSignals(False)
        self.alert_project.blockSignals(True)
        self.alert_project.setCurrentIndex(max(0, self.alert_project.findData(selected)))
        self.alert_project.blockSignals(False)
        self.render_alert_list()

    def render_alert_list(self, *_args):
        project = self.alert_project.currentData()
        status = self.alert_status.currentData()
        failed = [s for s in self.data['services'] if s.get('state') == 'exited_failed' and (project is None or s['project'] == project)]
        pending = sum(unread(s) for s in failed)
        self.alert_count.setText(f'{pending} 未读 / {len(failed) - pending} 已读')
        visible = [s for s in failed if status == 'all' or (status == 'unread') == unread(s)]
        visible.sort(key=lambda s: (not unread(s), s['project'], s['service']))
        signature = repr([(s['name'], s.get('exit_code'), s.get('failure_token'), s.get('alert_acknowledged'), s['name'] in self.action_pending) for s in visible])
        signature = (project, status, signature)
        if signature == self._alert_signature:
            return
        self._alert_signature = signature
        selected = self.alert_table.item(self.alert_table.currentRow(), 0)
        selected_name = selected.data(Qt.UserRole) if selected else None
        self.alert_table.setRowCount(0)
        for index, service in enumerate(visible):
            self.alert_table.insertRow(index)
            code = service.get('exit_code')
            values = [service['project'], service['service'], f'异常退出 · exit {code}' if code is not None else '异常退出', '未读' if unread(service) else '已读']
            for column, value in enumerate(values):
                item = QTableWidgetItem(value)
                item.setData(Qt.UserRole, service['name'])
                item.setToolTip(service['name'] + '\n' + values[2])
                self.alert_table.setItem(index, column, item)
            controls = QWidget()
            actions = QHBoxLayout(controls)
            actions.setContentsMargins(4, 4, 4, 4)
            actions.setSpacing(6)
            view = action_button('查看日志', lambda checked=False, s=service: self.view_alert_logs(s))
            view.setAccessibleName('查看异常日志 ' + service['name'])
            actions.addWidget(view)
            acknowledge = self.make_acknowledge_button(service)
            if not unread(service):
                acknowledge.setText('已读')
            actions.addWidget(acknowledge)
            for control in (view, acknowledge):
                control.setStyleSheet('padding:4px 7px; min-height:20px;')
            self.alert_table.setCellWidget(index, 4, controls)
            if selected_name == service['name']:
                self.alert_table.selectRow(index)
        self.alert_table.setVisible(bool(visible))
        self.alert_empty.setVisible(not visible)
        self.alert_empty.setText('当前筛选没有异常；可切换项目或处理状态。' if failed or project is not None or status != 'all' else '当前没有异常退出。')

    def view_alert_logs(self, service):
        # Preserve search, levels, history, pause and text selection when merely
        # acknowledging. This explicit navigation alone changes the log target.
        self.open_logs(service['name'])
        self.log_view.setFocus()

    def acknowledge_project(self, project):
        if project is not None:
            self.acknowledge_services([s for s in self.data['services'] if s['project'] == project and unread(s)])

    def acknowledge_services(self, services):
        services = [s for s in services if unread(s)]
        if not services or any(s['name'] in self.action_pending for s in services):
            return
        if any(not s.get('failure_token') for s in services):
            self.notify('当前记录缺少运行标识，请刷新后再确认', error=True)
            self.refresh()
            return
        queue = list(services)
        self.action_pending.update(s['name'] for s in queue)
        self.render_services()
        self.render_alert_list()
        errors = []
        completed = []
        def next_one():
            if not queue:
                self.action_pending.difference_update(s['name'] for s in services)
                message = (f'{services[0]["name"]} 的本次异常已读，退出状态和日志保留' if len(services) == 1 else f'{len(completed)} 条异常已读，退出状态和日志保留')
                self.notify('；'.join(errors) if errors else message, error=bool(errors))
                # An older in-flight status response must not win over the ack.
                # Queue another refresh when the existing request finishes.
                self._alert_refresh_needed = True
                if not self.refreshing:
                    self._alert_refresh_needed = False
                    self.refresh()
                self.render_services()
                self.render_alert_list()
                return
            service = queue.pop(0)
            def done(_data, error):
                if error:
                    errors.append(service['name'] + '：' + error)
                else:
                    completed.append(service['name'])
                next_one()
            self.request(['ack', service['name'], '--failure-token', service['failure_token']], done)
        next_one()
