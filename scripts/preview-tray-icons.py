#!/usr/bin/env python3
"""Render the shipped artwork on light/dark panels at real tray sizes."""
import os
os.environ['QT_QPA_PLATFORM'] = 'offscreen'
from pathlib import Path
from PySide6.QtWidgets import QApplication
from PySide6.QtGui import QImage, QPainter, QColor, QFont
from PySide6.QtCore import QRectF, Qt
app = QApplication([])
root = Path(__file__).resolve().parents[1]
canvas = QImage(1020, 470, QImage.Format_ARGB32)
canvas.fill(QColor('#f4f5f1'))
p = QPainter(canvas)
p.setRenderHint(QPainter.Antialiasing)
def text(x,y,value,size=14,color='#263a35'):
    p.setPen(QColor(color)); p.setFont(QFont('Noto Sans CJK SC',size));p.drawText(x,y,value)
text(35,48,'devtray  /  托盘状态图标',22)
text(35,78,'同一轮廓，四种状态。颜色 + 符号，在深浅面板下保持可辨。',12,'#78887c')
for i,(name,title,detail) in enumerate([
    ('devtray','正常','运行 / 就绪 · 勾号'),('devtray-starting','启动中','编译 / 启动 · 时钟'),
    ('devtray-error','异常','服务失败 / 配置错误 · 叹号'),('devtray-idle','空闲','全部停止 / 未启动 · 横线'),
]):
    x=30+i*245
    p.setPen(Qt.NoPen);p.setBrush(QColor('#fff'));p.drawRoundedRect(QRectF(x,102,230,335),12,12)
    p.drawImage(QRectF(x+88,122,54,54),QImage(str(root/f'assets/icons/hicolor/256x256/apps/{name}.png')))
    text(x+18,210,title,17);text(x+18,238,detail,10,'#78887c')
    for y,bg in [(260,'#eef0eb'),(322,'#192b23')]:
        p.setPen(Qt.NoPen);p.setBrush(QColor(bg));p.drawRoundedRect(QRectF(x+13,y,204,48),7,7)
        for j,size in enumerate((16,22,24,32)):
            p.drawImage(QRectF(x+30+j*47,y+(48-size)/2,size,size),QImage(str(root/f'assets/icons/hicolor/{size}x{size}/apps/{name}.png')))
    text(x+21,401,'16 px     22 px      24 px      32 px',10,'#78887c')
p.end()
dest=root/'docs/screenshots/tray-icons.png';dest.parent.mkdir(exist_ok=True,parents=True);canvas.save(str(dest));print(dest)
