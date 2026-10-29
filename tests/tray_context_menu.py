"""Real tray/embedded-window regression on a private bus; never controls user services.
Run: dbus-run-session -- python3 tests/tray_context_menu.py
"""
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
import time

import gi
gi.require_version('Gio', '2.0')
from gi.repository import Gio, GLib
from PySide6.QtCore import QCoreApplication
from PySide6.QtNetwork import QLocalSocket

ROOT = Path(__file__).resolve().parents[1]
APP = QCoreApplication([])
CONN = Gio.bus_get_sync(Gio.BusType.SESSION, None)
SNI = 'org.kde.StatusNotifierItem'
WATCHER = 'org.kde.StatusNotifierWatcher'
XML = '''<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
</interface></node>'''


def call(bus, path, interface, method, args=None):
    return CONN.call_sync(bus, path, interface, method, args, None,
                          Gio.DBusCallFlags.NONE, 3000, None).unpack()


def wait_for(check, description):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        APP.processEvents()
        value = check()
        if value:
            return value
        time.sleep(.04)
    raise AssertionError(description)


# Do not replace any real desktop watcher if this test is accidentally run
# without dbus-run-session.
assert not call('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
                'NameHasOwner', GLib.Variant('(s)', (WATCHER,)))[0], 'Use a private dbus-run-session'


def method(conn, sender, path, interface, name, args, invocation):
    invocation.return_value(GLib.Variant('()', ()))


def property_value(conn, sender, path, interface, name):
    return {'IsStatusNotifierHostRegistered': GLib.Variant('b', True),
            'RegisteredStatusNotifierItems': GLib.Variant('as', []),
            'ProtocolVersion': GLib.Variant('i', 0)}[name]


registration = CONN.register_object('/StatusNotifierWatcher',
    Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0], method, property_value, None)
call('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
     'RequestName', GLib.Variant('(su)', (WATCHER, 4)))
loop = GLib.MainLoop()
thread = threading.Thread(target=loop.run, daemon=True)
thread.start()

try:
    for mode in ('native', 'missing-python', 'acknowledgement'):
        with tempfile.TemporaryDirectory(prefix='devtray-tray-context-') as temp:
            root = Path(temp)
            config = root/'config.toml'
            config.write_text('[settings]\nnotify=false\n')
            env = dict(os.environ, DEVTRAY_CONFIG=str(config), XDG_RUNTIME_DIR=temp,
                       XDG_CONFIG_HOME=temp, XDG_STATE_HOME=temp, QT_QPA_PLATFORM='offscreen')
            env.pop('DEVTRAY_NATIVE_MENU', None)
            env.pop('DEVTRAY_PYTHON', None)
            if mode == 'missing-python':
                env['DEVTRAY_PYTHON'] = '/nonexistent/devtray-test-python'
            if mode == 'acknowledgement':
                config.write_text(f'[settings]\nnotify=false\npoll_ms=100\n[[project]]\nname="hb_ecom"\n[[project.service]]\nname="web"\ncwd="{root}"\ncmd="false"\n')
                snapshot = root/'snapshot'
                snapshot.write_text('devtray-hb_ecom-web\t1\t1\t4242\n')
                bindir = root/'bin'
                bindir.mkdir()
                tmux = bindir/'tmux'
                tmux.write_text('#!/bin/sh\ncat "$DEVTRAY_TEST_SNAPSHOT"\n')
                tmux.chmod(0o755)
                env.update(PATH=str(bindir)+':'+env['PATH'], DEVTRAY_TEST_SNAPSHOT=str(snapshot))

            socket_name = 'devtray-panel-' + hashlib.sha256(
                (str(os.getuid()) + ':' + str(config.resolve())).encode()).hexdigest()[:24]

            def send(payload):
                sock = QLocalSocket()
                sock.connectToServer(socket_name)
                if not sock.waitForConnected(100):
                    return None
                sock.write(json.dumps(payload).encode()+b'\n')
                sock.waitForBytesWritten(1000)
                assert sock.waitForReadyRead(2000), 'UI did not acknowledge request'
                result = json.loads(bytes(sock.readAll()))
                sock.disconnectFromServer()
                return result

            with (root/'tray.log').open('wb') as output:
                process = subprocess.Popen([str(ROOT/'target/debug/devtray')], env=env,
                    stdout=output, stderr=output, start_new_session=True)
                bus = f'org.kde.StatusNotifierItem-{process.pid}-1'
                try:
                    def registered():
                        assert process.poll() is None, (root/'tray.log').read_text()
                        return call('org.freedesktop.DBus', '/org/freedesktop/DBus',
                            'org.freedesktop.DBus', 'NameHasOwner', GLib.Variant('(s)', (bus,)))[0]
                    wait_for(registered, 'Tray did not register')
                    props = call(bus, '/StatusNotifierItem', 'org.freedesktop.DBus.Properties',
                                 'GetAll', GLib.Variant('(s)', (SNI,)))[0]
                    assert props['Menu'] == '/MenuBar', props['Menu']
                    assert props['ItemIsMenu'] is False
                    try:
                        call(bus, '/StatusNotifierItem', SNI, 'ContextMenu', GLib.Variant('(ii)', (0, 0)))
                    except GLib.Error as exc:
                        assert 'UnknownMethod' in str(exc), exc
                    else:
                        raise AssertionError('Right click must use the native menu')
                    layout = call(bus, '/MenuBar', 'com.canonical.dbusmenu', 'GetLayout',
                                  GLib.Variant('(iias)', (0, -1, [])))[1]
                    assert layout[2], 'Native menu is empty'
                    if mode == 'native':
                        def visible(menu, panel):
                            state = send({'inspect': True})
                            return state and state['menu_visible'] == menu and state['visible'] == panel
                        call(bus, '/StatusNotifierItem', SNI, 'Activate', GLib.Variant('(ii)', (500, 30)))
                        wait_for(lambda: visible(True, False), 'Left click did not open default small panel')
                        send({'mode': 'full'})
                        call(bus, '/StatusNotifierItem', SNI, 'Activate', GLib.Variant('(ii)', (500, 30)))
                        wait_for(lambda: visible(False, False), 'Left click did not hide full panel')
                        call(bus, '/StatusNotifierItem', SNI, 'Activate', GLib.Variant('(ii)', (500, 30)))
                        wait_for(lambda: visible(False, True), 'Left click did not remember full mode')
                    if mode == 'acknowledgement':
                        def properties():
                            return call(bus, '/StatusNotifierItem', 'org.freedesktop.DBus.Properties',
                                        'GetAll', GLib.Variant('(s)', (SNI,)))[0]
                        try:
                            wait_for(lambda: '1 失败' in properties()['ToolTip'][3], 'Tray did not load the initial failed service')
                        except AssertionError as exc:
                            raise AssertionError((properties()['ToolTip'][3], (root/'tray.log').read_text())) from exc
                        unread_icon = properties()['IconPixmap']
                        assert unread_icon
                        subprocess.run([str(ROOT/'target/debug/devtray'),'ack','hb_ecom/web'], env=env,
                                       capture_output=True, text=True, check=True)
                        read_icon = wait_for(lambda: (value if (value := properties()['IconPixmap']) != unread_icon else None),
                                             'Marking failed service read did not restore tray icon')
                        assert '已读异常' in properties()['ToolTip'][3]
                        # Restart only this private tray, keeping the same failed pane and state store.
                        os.killpg(process.pid, signal.SIGTERM)
                        process.wait(timeout=5)
                        process = subprocess.Popen([str(ROOT/'target/debug/devtray')], env=env,
                            stdout=output, stderr=output, start_new_session=True)
                        bus = f'org.kde.StatusNotifierItem-{process.pid}-1'
                        wait_for(registered, 'Restarted tray did not register')
                        wait_for(lambda: '已读异常' in properties()['ToolTip'][3], 'Restart did not restore durable acknowledgement')
                        assert properties()['IconPixmap'] == read_icon, 'Restart lost the acknowledgement'
                        snapshot.write_text('devtray-hb_ecom-web\t1\t1\t5252\n')
                        wait_for(lambda: properties()['IconPixmap'] == unread_icon,
                                 'A new failed run did not restore the error icon')
                        print('PASS: real tray icon clears after read, survives restart, and alerts on new failed run', flush=True)
                    print(f'PASS: {mode} tray context-menu routing', flush=True)
                finally:
                    send({'quit': True})
                    # Only the isolated test process group, including any UI child.
                    os.killpg(process.pid, signal.SIGTERM)
                    process.wait(timeout=5)
finally:
    loop.quit()
    thread.join(timeout=2)
    CONN.unregister_object(registration)
