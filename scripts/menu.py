#!/usr/bin/env python3
"""devtray 菜单验证助手：通过 dbusmenu 直接 dump 菜单树 / 模拟点击某一项。

托盘是否真的按预期渲染，不用靠肉眼——宿主（noctalia）就是这么读我们的菜单的。

用法:
  scripts/menu.py <SNI bus name> dump
  scripts/menu.py <SNI bus name> click <标签> [--under <子菜单标签前缀>]

bus name 形如 org.kde.StatusNotifierItem-<pid>-1，可用下面这行拿到：
  gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.ListNames | tr "," "\n" | grep StatusNotifierItem-$(pgrep -x devtray)
"""

import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

bus = sys.argv[1]
mode = sys.argv[2] if len(sys.argv) > 2 else "dump"
label = sys.argv[3] if len(sys.argv) > 3 else None
under = None
if "--under" in sys.argv:
    under = sys.argv[sys.argv.index("--under") + 1]

conn = Gio.bus_get_sync(Gio.BusType.SESSION, None)
MENU = "/MenuBar"


def get_layout():
    return conn.call_sync(
        bus, MENU, "com.canonical.dbusmenu", "GetLayout",
        GLib.Variant("(iias)", (0, -1, [])), None,
        Gio.DBusCallFlags.NONE, 5000, None).unpack()[1]


def lab(node):
    v = node[1].get("label")
    return v if isinstance(v, str) else (v.get_string() if v is not None else "")


def dump(node, depth=0):
    print(f"{'  ' * depth}- id={node[0]:<4} {lab(node)!r}")
    for c in node[2]:
        dump(c, depth + 1)


def find(node, want, parent_ok=True):
    if parent_ok and lab(node) == want:
        return node[0]
    for c in node[2]:
        inside = parent_ok
        if under and lab(node).startswith(under):
            inside = True
        got = find(c, want, inside)
        if got is not None:
            return got
    return None


root = get_layout()
if mode == "dump":
    dump(root)
else:
    iid = find(root, label, under is None)
    print(f"click {label!r} -> id={iid}")
    if iid is not None:
        conn.call_sync(
            bus, MENU, "com.canonical.dbusmenu", "Event",
            GLib.Variant("(isvu)", (iid, "clicked", GLib.Variant("s", ""), 0)),
            None, Gio.DBusCallFlags.NONE, 5000, None)
