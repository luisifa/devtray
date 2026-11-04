"""Feedback overlay: no layout jumps, no focus stealing, no implicit alert ack."""
from PySide6.QtCore import QEvent, QPropertyAnimation, QTimer, Qt
from PySide6.QtWidgets import QFrame, QGraphicsOpacityEffect, QHBoxLayout, QLabel, QPushButton


class FeedbackToast(QFrame):
    def __init__(self, window, close):
        super().__init__(window)
        self.setObjectName('feedbackToast')
        layout = QHBoxLayout(self)
        layout.setContentsMargins(14, 12, 12, 12)
        layout.setSpacing(12)
        self.message = QLabel()
        self.message.setTextFormat(Qt.PlainText)
        self.message.setWordWrap(True)
        self.message.setTextInteractionFlags(Qt.TextSelectableByMouse)
        self.message.setAccessibleName('操作反馈')
        layout.addWidget(self.message, 1)
        self.close_button = QPushButton('关闭提示')
        self.close_button.setAccessibleName('关闭操作提示')
        self.close_button.setToolTip('仅关闭此消息；服务异常可在对应位置标记已读')
        self.close_button.clicked.connect(close)
        layout.addWidget(self.close_button)
        self.timer = QTimer(self)
        self.timer.setSingleShot(True)
        self.timer.timeout.connect(close)
        effect = QGraphicsOpacityEffect(self)
        self.setGraphicsEffect(effect)
        self.fade = QPropertyAnimation(effect, b'opacity', self)
        self.fade.setDuration(140)
        self.message.hide()
        self.close_button.hide()
        self.hide()
        window.installEventFilter(self)

    def eventFilter(self, watched, event):
        if watched is self.parentWidget() and event.type() == QEvent.Resize and self.isVisible():
            self.reposition()
        return super().eventFilter(watched, event)

    def reposition(self):
        parent = self.parentWidget()
        available = max(260, parent.width() - 264)
        self.setFixedWidth(min(620, available))
        self.adjustSize()
        height = self.layout().totalHeightForWidth(self.width())
        if height > 0:
            self.resize(self.width(), max(64, height))
        self.move(max(12, parent.width() - self.width() - 20), max(12, parent.height() - self.height() - 70))

    def present(self, message, error=False):
        self.timer.stop()
        self.setProperty('feedbackKind', 'error' if error else 'success')
        for widget in (self, self.message):
            widget.style().unpolish(widget)
            widget.style().polish(widget)
        self.message.setText(('操作失败：' if error else '完成：') + message)
        self.message.show()
        self.close_button.show()
        self.reposition()
        self.show()
        self.raise_()
        self.fade.stop()
        self.fade.setStartValue(.65)
        self.fade.setEndValue(1.0)
        self.fade.start()
        if not error:
            self.timer.start(5000)

    def clear(self):
        self.timer.stop()
        self.fade.stop()
        self.message.clear()
        self.message.hide()
        self.close_button.hide()
        self.hide()
