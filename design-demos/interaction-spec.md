# 松绿色方向内的交互细化

本次沿用 direction-approved.md 已确认的方向 A。页面仍保留全局项目／服务树，资源三列与业务操作入口。

Form 来自实际操作：服务状态、资源扫描和快捷键草稿需要持续可见，因此使用状态邻近的按钮、可辨的边框以及不挤动内容的反馈浮层。

| 设计问题 | 决策 |
| --- | --- |
| 叙事角色 | 开发工作台的操作界面，强调能看清当前范围和执行结果 |
| 观众距离 | 桌面近距离使用，支持鼠标与键盘 |
| 视觉温度 | 保留深松绿侧栏、浅色工作区与克制的圆角 |
| 信息容量 | 保留服务卡片、资源三列；快捷键与工作台信息分为两个设置页签 |
| 视觉母题 | 细线分区、状态边框、按钮层级与明确焦点，而非额外装饰 |

边框区分容器、输入、选中与危险状态；主要按钮使用松绿实底，常用次级操作保留描边，工具栏低频动作使用轻量按钮。错误字段同时显示红边和文本解释；成功和错误反馈采用不同底色，进行中状态留在操作位置并禁用重复提交。

默认键方案参考 GNOME 的查找、刷新、新增、设置与帮助约定；应用专用的模块和日志按键在完整设置表中明确列出。实现与验收说明见 ../docs/keyboard-shortcuts.md，原生 Qt 截图位于 ../docs/screenshots/interaction-*.png。

事实来源：现有 ui/panel.py、ui/quick_menu.py 与 QSettings 配置；GNOME https://developer.gnome.org/hig/reference/keyboard.html 及 https://developer.gnome.org/hig/guidelines/keyboard.html ；Qt https://doc.qt.io/qtforpython-6/PySide6/QtWidgets/QKeySequenceEdit.html 。
