"""Approved pine-green workspace theme. The light prototype remains a design alternative."""
STYLE = """
* { font-family: "Noto Sans CJK SC", "Noto Sans", sans-serif; font-size: 14px; }
QMainWindow, QDialog { background: #f5f2ea; color: #202722; }
QWidget { color: #202722; }
QFrame#sidebar { background: #214b3d; border: none; }
QFrame#sidebar QLabel { color: #e1ece5; }
QLabel#brand { font-size: 23px; font-weight: 750; }
QLabel#sideCaption { color: #d0ded5; font-size: 12px; }
QPushButton { background: #ffffff; border: 1px solid #b4c1b7; border-radius: 6px; padding: 8px 11px; color: #294335; min-height: 22px; }
QPushButton:hover { background: #edf4ee; border-color: #6d8e7b; }
QPushButton:pressed { background: #d3e4d8; border-color: #35634b; }
QPushButton:focus { border-color: #19744f; }
QPushButton:disabled { color: #727a73; background: #eeeee7; border-color: #d8d7cf; }
QPushButton#primary { background: #2d6a52; color: white; border-color: #2d6a52; font-weight: 600; }
QPushButton#primary:hover { background: #245d48; border-color: #245d48; }
QPushButton#primary:pressed { background: #174b38; border-color: #174b38; }
QPushButton#primary:disabled { background: #dce5de; color: #63786b; border-color: #c4d0c6; }
QPushButton#danger { color: #a03730; border-color: #d9b7b0; background: #fffaf8; }
QPushButton#danger:hover { color: #922c27; border-color: #b86255; background: #fcebe6; }
QPushButton#danger:pressed { background: #f3d4cc; }
QPushButton#danger:disabled { color: #8a7975; border-color: #d8d7cf; background: #eeeee7; }
QPushButton#quiet { background: transparent; border-color: transparent; }
QPushButton#quiet:hover { background: #e7eee7; border-color: #b5c5b8; }
QPushButton#serviceTitle { border-color: transparent; background: transparent; text-align: left; padding: 2px 4px; font-size: 16px; font-weight: 700; }
QPushButton#serviceTitle:hover { background: #edf4ee; border-color: #b5c5b8; }
QPushButton#nav { background: transparent; border: 1px solid transparent; border-left: 3px solid transparent; color: #e3eee5; text-align: left; padding: 9px 10px; }
QPushButton#nav:checked { background: #416450; color: #ffffff; border-left: 3px solid #f0c770; }
QPushButton#nav:hover { background: #375b48; }
QTreeWidget { background: transparent; border: none; outline: none; color: #202722; }
QTreeWidget::item { padding: 6px 3px; border-radius: 3px; }
QTreeWidget::item:selected { background: #dce8df; color: #183b30; }
QTreeWidget::item:hover { background: #edf2e9; }
QFrame#sidebar QTreeWidget { color: #e3eee5; }
QFrame#sidebar QTreeWidget::item:selected { background: #416450; color: white; }
QFrame#sidebar QTreeWidget::item:hover { background: #375b48; }
QFrame#card, QFrame#resourcePane { background: #fbfaf6; border: 1px solid #d5ded3; border-radius: 7px; }
QFrame#hero, QFrame#pageIntro { background: transparent; border: none; }
QFrame#card[busy="true"] { border-color: #5b9777; background: #f2f7f0; }
QFrame#cleanupPane { background: #eef3ed; border: 1px solid #c8d3c8; border-radius: 4px; }
QLabel#title { font-size: 23px; font-weight: 700; }
QLabel#sectionTitle { font-size: 16px; font-weight: 650; }
QLabel#muted { color: #626a64; font-size: 12px; }
QLabel#number { font-size: 28px; font-weight: 650; }
QLabel#selectionSize { font-size: 23px; font-weight: 700; color: #205441; }
QLabel#mono { font-family: "JetBrains Mono", "DejaVu Sans Mono", monospace; font-size: 12px; color: #58655d; }
QLabel#badge { border-radius: 3px; padding: 3px 7px; font-size: 12px; }
QLabel#errorBanner { background: #fff5d9; color: #6a4b18; padding: 10px; border: 1px solid #dcc58e; border-radius: 4px; }
QLineEdit, QComboBox, QKeySequenceEdit { background: #ffffff; border: 1px solid #b9bdb5; border-radius: 4px; padding: 8px 9px; selection-background-color: #b7d0bc; }
QLineEdit:focus, QComboBox:focus, QKeySequenceEdit:focus { border-color: #19744f; }
QComboBox::drop-down { border: none; width: 22px; }
QComboBox QAbstractItemView { background: white; color: #202722; selection-background-color: #dce8df; }
QTableWidget { border: 1px solid #d5ded3; border-radius: 5px; background: #fbfaf6; alternate-background-color: #f3f4ed; color: #202722; gridline-color: #e2e6dd; selection-background-color: #dce8df; selection-color: #183b30; }
QHeaderView::section { background: #efeee7; color: #465249; border: none; border-bottom: 1px solid #d8d7cf; padding: 8px 5px; font-size: 12px; }
QPlainTextEdit { background: #fcfdfa; border: 1px solid #d8d7cf; border-radius: 4px; padding: 10px; selection-background-color: #c4dcc4; selection-color: #183b30; }
QPlainTextEdit#logView { background: #183b30; color: #d6e1d9; border: 1px solid #315743; border-radius: 4px; font-family: "JetBrains Mono", "DejaVu Sans Mono", monospace; font-size: 12px; padding: 14px; selection-background-color: #52775f; selection-color: #ffffff; }
QPlainTextEdit#diffView { background: #fcfdfa; color: #2b322d; border: 1px solid #d8d7cf; border-radius: 5px; font-family: "JetBrains Mono", "DejaVu Sans Mono", monospace; font-size: 12px; padding: 10px; selection-background-color: #c4dcc4; selection-color: #183b30; }
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
QPushButton:focus, QPushButton#primary:focus, QPushButton#danger:focus, QPushButton#quiet:focus, QPushButton#serviceTitle:focus { border: 2px solid #19744f; padding: 7px 10px; }
QPushButton#nav:focus { border-color: #d4e9d9; }
QPushButton#primary:focus { border-color: #87bc9c; }
QLineEdit#projectSearch { background: #263d33; color: #e4efe7; border-color: #587362; }
QLineEdit#projectSearch:focus { border-color: #c3ddca; background: #2b4939; }
QTableWidget:focus, QTreeWidget:focus { border: 1px solid #3a805c; }
QFrame#sidebar QTreeWidget:focus { border-color: #a6c4ae; }
QKeySequenceEdit { background: #fff; border: 1px solid #b4c1b7; border-radius: 5px; padding: 3px 6px; }
QKeySequenceEdit QLineEdit { border: none; background: transparent; padding: 6px 2px; }
QKeySequenceEdit[invalid="true"] { border: 2px solid #b34c40; background: #fff2ed; }
QLabel#validationMessage { color: #963a30; background: #fff1eb; border: 1px solid #e1b9ad; border-radius: 5px; padding: 8px; }
QTabWidget::pane { border: 1px solid #d5ded3; background: #fbfaf6; border-radius: 6px; }
QTabBar::tab { background: #ecefe7; color: #465b4d; padding: 10px 22px; border: 1px solid #d5ded3; border-bottom: none; border-top-left-radius: 5px; border-top-right-radius: 5px; }
QTabBar::tab:selected { background: #fbfaf6; color: #20513b; font-weight: 600; }
QTabBar::tab:hover { background: #e1ecdf; }
QFrame#feedbackToast { background: #edf6eb; border: 1px solid #88b193; border-radius: 8px; }
QFrame#feedbackToast QLabel { background: transparent; color: #24543a; font-size: 14px; }
QFrame#feedbackToast[feedbackKind="error"] { background: #fff0eb; border-color: #c57d6e; }
QFrame#feedbackToast[feedbackKind="error"] QLabel { color: #8b3028; }

"""
