"""Approved pine-green workspace theme. The light prototype remains a design alternative."""
STYLE = """
* { font-family: "Noto Sans CJK SC", "Noto Sans", sans-serif; font-size: 14px; }
QMainWindow, QDialog { background: #f5f2ea; color: #202722; }
QWidget { color: #202722; }
QFrame#sidebar { background: #214b3d; border: none; }
QFrame#sidebar QLabel { color: #e1ece5; }
QLabel#brand { font-size: 23px; font-weight: 750; }
QLabel#sideCaption { color: #d0ded5; font-size: 12px; }
QPushButton { background: #ffffff; border: 1px solid #b9bdb5; border-radius: 4px; padding: 8px 11px; color: #294335; min-height: 22px; }
QPushButton:hover { background: #edf3eb; border-color: #718b7b; }
QPushButton:pressed { background: #dce8df; }
QPushButton:focus { border-color: #1967d2; }
QPushButton:disabled { color: #727a73; background: #eeeee7; border-color: #d8d7cf; }
QPushButton#primary { background: #2d6a52; color: white; border-color: #2d6a52; font-weight: 600; }
QPushButton#primary:hover { background: #205441; }
QPushButton#danger { color: #9b3c34; }
QPushButton#nav { background: transparent; border: none; color: #e3eee5; text-align: left; padding: 9px 10px; }
QPushButton#nav:checked { background: #416450; color: #ffffff; border-left: 3px solid #f0c770; }
QPushButton#nav:hover { background: #375b48; }
QTreeWidget { background: transparent; border: none; outline: none; color: #202722; }
QTreeWidget::item { padding: 6px 3px; border-radius: 3px; }
QTreeWidget::item:selected { background: #dce8df; color: #183b30; }
QTreeWidget::item:hover { background: #edf2e9; }
QFrame#sidebar QTreeWidget { color: #e3eee5; }
QFrame#sidebar QTreeWidget::item:selected { background: #416450; color: white; }
QFrame#sidebar QTreeWidget::item:hover { background: #375b48; }
QFrame#card, QFrame#resourcePane { background: #fbfaf6; border: 1px solid #d8d7cf; border-radius: 4px; }
QFrame#hero, QFrame#pageIntro { background: transparent; border: none; }
QFrame#cleanupPane { background: #eef3ed; border: 1px solid #c8d3c8; border-radius: 4px; }
QLabel#title { font-size: 23px; font-weight: 700; }
QLabel#sectionTitle { font-size: 16px; font-weight: 650; }
QLabel#muted { color: #626a64; font-size: 12px; }
QLabel#number { font-size: 28px; font-weight: 650; }
QLabel#selectionSize { font-size: 23px; font-weight: 700; color: #205441; }
QLabel#mono { font-family: "JetBrains Mono", "DejaVu Sans Mono", monospace; font-size: 12px; color: #58655d; }
QLabel#badge { border-radius: 3px; padding: 3px 7px; font-size: 12px; }
QLabel#errorBanner { background: #fff5d9; color: #6a4b18; padding: 10px; border: 1px solid #dcc58e; border-radius: 4px; }
QLineEdit, QComboBox { background: #ffffff; border: 1px solid #b9bdb5; border-radius: 4px; padding: 8px 9px; selection-background-color: #b7d0bc; }
QLineEdit:focus, QComboBox:focus { border-color: #1967d2; }
QComboBox::drop-down { border: none; width: 22px; }
QComboBox QAbstractItemView { background: white; color: #202722; selection-background-color: #dce8df; }
QTableWidget { background: #fbfaf6; alternate-background-color: #f3f4ed; color: #202722; gridline-color: #e2e2da; border: none; selection-background-color: #dce8df; selection-color: #183b30; }
QHeaderView::section { background: #efeee7; color: #465249; border: none; border-bottom: 1px solid #d8d7cf; padding: 8px 5px; font-size: 12px; }
QPlainTextEdit { background: #fcfdfa; border: 1px solid #d8d7cf; border-radius: 4px; padding: 10px; selection-background-color: #c4dcc4; selection-color: #183b30; }
QPlainTextEdit#logView { background: #183b30; color: #d6e1d9; border: 1px solid #315743; border-radius: 4px; font-family: "JetBrains Mono", "DejaVu Sans Mono", monospace; font-size: 12px; padding: 14px; selection-background-color: #52775f; selection-color: #ffffff; }
QScrollArea { border: none; background: transparent; }
QScrollArea > QWidget > QWidget { background: transparent; }
QScrollBar:vertical { width: 8px; background: transparent; margin: 2px; }
QScrollBar::handle:vertical { background: #a8b9aa; border-radius: 3px; min-height: 24px; }
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical { height: 0; }
QSplitter::handle { background: #d8d7cf; width: 4px; }
QCheckBox { spacing: 7px; }
QCheckBox::indicator { width: 17px; height: 17px; border: 1px solid #8caa94; border-radius: 3px; background: white; }
QCheckBox::indicator:checked { background: #2d6a52; border-color: #2d6a52; }
QToolTip { background: #214b3d; color: white; padding: 7px; border: none; }
QMenu { background: #ffffff; border: 1px solid #b9bdb5; padding: 5px; }
QMenu::item { padding: 9px 16px; }
QMenu::item:selected { background: #dce8df; }
"""
