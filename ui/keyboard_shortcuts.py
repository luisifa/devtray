"""Window-local shortcuts with validated, per-configuration overrides."""
import json
import weakref

from PySide6.QtCore import QEvent, QSettings, Qt
from PySide6.QtGui import QKeySequence, QShortcut
from PySide6.QtWidgets import QApplication, QKeySequenceEdit


SPECS = [
    ('overview', '总览', 'Ctrl+1'),
    ('services', '服务管理', 'Ctrl+2'),
    ('resources', '资源', 'Ctrl+3'),
    ('git', 'Git', 'Ctrl+4'),
    ('agents', 'Agent 与 Skills', 'Ctrl+5'),
    ('logs', '日志中心', 'Ctrl+6'),
    ('settings', '工作台设置', 'Ctrl+,'),
    ('search', '搜索当前页面', 'Ctrl+F'),
    ('projects', '搜索项目或服务', 'Ctrl+Shift+F'),
    ('refresh', '刷新服务状态', 'Ctrl+R'),
    ('new_service', '添加服务', 'Ctrl+N'),
    ('service_logs', '查看当前服务日志', 'Ctrl+L'),
    ('pause_logs', '暂停／继续日志刷新', 'Ctrl+Shift+P'),
    ('toggle_mode', '切换大小面板', 'Ctrl+M'),
    ('help', '快捷键说明', 'F1'),
    ('hide', '收回当前面板', 'Esc'),
]
DEFAULTS = {key: sequence for key, _title, sequence in SPECS}
TITLES = {key: title for key, title, _sequence in SPECS}
SETTINGS_KEY = 'keyboard/overrides_v1'


def canonical(value):
    if not isinstance(value, str):
        raise ValueError('快捷键格式无效')
    if not value.strip():
        return ''
    sequence = QKeySequence.fromString(value, QKeySequence.PortableText)
    if sequence.count() != 1 or sequence[0].key() in (Qt.Key_unknown, Qt.Key(0)):
        raise ValueError('请使用单组快捷键，如 Ctrl+R 或 F5')
    return sequence.toString(QKeySequence.PortableText)


def validate(values):
    """Return normalized full bindings and errors; empty values disable an action."""
    normalized, errors, owners = {}, {}, {}
    reserved = {'Ctrl+C', 'Ctrl+V', 'Ctrl+X', 'Ctrl+A', 'Ctrl+Z', 'Ctrl+Y',
                'Ctrl+Shift+Z', 'Ctrl+Insert', 'Shift+Insert', 'Shift+Del',
                'Ctrl+Tab', 'Ctrl+Shift+Tab', 'Alt+Tab', 'Alt+Shift+Tab',
                'Alt+F4', 'Ctrl+Alt+Del', 'Ctrl+Alt+L'}
    for key, _title, default in SPECS:
        try:
            value = canonical(values.get(key, default))
            normalized[key] = value
            if not value:
                continue
            combination = QKeySequence(value)[0]
            code, modifiers = combination.key(), combination.keyboardModifiers()
            if modifiers & Qt.MetaModifier:
                raise ValueError('Super／Meta 留给桌面环境，请换一组快捷键')
            if value in reserved or (modifiers & Qt.ControlModifier and modifiers & Qt.AltModifier and Qt.Key_F1 <= code <= Qt.Key_F12):
                raise ValueError('此组合用于文本编辑或桌面操作，请换一组快捷键')
            if not modifiers & (Qt.ControlModifier | Qt.AltModifier) and not (Qt.Key_F1 <= code <= Qt.Key_F35 or code == Qt.Key_Escape):
                raise ValueError('字母和导航键需要 Ctrl 或 Alt，避免影响输入与焦点移动')
            if value in owners:
                other = owners[value]
                errors[key] = f'与「{TITLES[other]}」重复'
                errors[other] = f'与「{TITLES[key]}」重复'
            else:
                owners[value] = key
        except ValueError as error:
            errors[key] = str(error)
    return normalized, errors


class ShortcutRecorder(QKeySequenceEdit):
    """Recording must receive keys even when they are already app shortcuts."""
    def __init__(self, sequence, parent=None):
        super().__init__(QKeySequence(sequence), parent)
        self.setAttribute(Qt.WA_StyledBackground, True)
        if hasattr(self, 'setMaximumSequenceLength'):
            self.setMaximumSequenceLength(1)
        self.setClearButtonEnabled(True)

    def event(self, event):
        if event.type() == QEvent.ShortcutOverride:
            event.accept()
            return True
        return super().event(event)


class ShortcutController:
    def __init__(self, settings, callback):
        self.settings, self.callback = settings, callback
        self.windows, self.bindings = {}, {}
        self.load_error = None
        try:
            raw = json.loads(str(settings.value(SETTINGS_KEY, '{}')))
            if not isinstance(raw, dict):
                raise ValueError('配置不是对象')
            self.values, errors = validate(raw)
            if errors:
                raise ValueError('存在重复或无效组合')
        except (ValueError, TypeError):
            self.values = dict(DEFAULTS)
            self.load_error = '已保存的快捷键配置无效，当前使用默认值；应用修改后可重新保存。'

    def attach(self, window):
        identity = id(window)
        if identity in self.windows:
            return
        self.windows[identity] = weakref.ref(window)
        window.destroyed.connect(lambda *_args: self.detach(identity))
        self._bind(window)

    def detach(self, identity):
        self.windows.pop(identity, None)
        self.bindings.pop(identity, None)

    def _bind(self, window):
        identity = id(window)
        for shortcut in self.bindings.pop(identity, []):
            shortcut.setEnabled(False)
            shortcut.deleteLater()
        shortcuts = []
        for action, sequence in self.values.items():
            if not sequence:
                continue
            shortcut = QShortcut(QKeySequence(sequence), window)
            shortcut.setContext(Qt.WindowShortcut)
            shortcut.setAutoRepeat(False)
            shortcut.activated.connect(lambda action=action, window=window: self.dispatch(action, window))
            shortcuts.append(shortcut)
        self.bindings[identity] = shortcuts

    def dispatch(self, action, window):
        if QApplication.activeModalWidget() is not None:
            return
        focus = QApplication.focusWidget()
        while focus is not None:
            if isinstance(focus, QKeySequenceEdit):
                return
            focus = focus.parentWidget()
        self.callback(action, window)

    def save(self, values):
        normalized, errors = validate(values)
        if errors:
            return errors
        overrides = {key: value for key, value in normalized.items() if value != DEFAULTS[key]}
        self.settings.setValue(SETTINGS_KEY, json.dumps(overrides))
        self.settings.sync()
        if self.settings.status() != QSettings.Status.NoError:
            return {'storage': '快捷键保存失败，请检查配置目录权限'}
        self.values = normalized
        self.load_error = None
        for reference in list(self.windows.values()):
            window = reference()
            if window is not None:
                self._bind(window)
        return {}

    def display(self, action):
        return QKeySequence(self.values[action]).toString(QKeySequence.NativeText) or '未分配'
