"""Real private system-style D-Bus; no host bus or physical devices."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

try:
    import gi
    HAVE_GI = True
except ImportError:
    HAVE_GI = False


@unittest.skipUnless(HAVE_GI and shutil.which('dbus-daemon'), 'PyGObject and private dbus-daemon required')
class BusTests(unittest.TestCase):
    def test_sleep_signal_without_monitoring_privileges(self):
        config = '''<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>system</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>
<policy context="default"><allow user="*"/><deny own="*"/><allow own="org.freedesktop.login1"/>
<deny send_type="method_call"/><allow send_type="signal"/>
<allow send_requested_reply="true" send_type="method_return"/><allow send_requested_reply="true" send_type="error"/>
<allow receive_type="method_call"/><allow receive_type="method_return"/><allow receive_type="error"/><allow receive_type="signal"/>
<allow send_destination="org.freedesktop.DBus" send_interface="org.freedesktop.DBus"/>
</policy></busconfig>'''
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / 'system.conf'
            path.write_text(config)
            daemon = subprocess.Popen(['dbus-daemon', '--nofork', '--config-file=' + str(path), '--print-address'],
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            address = daemon.stdout.readline().strip()
            self.assertTrue(address)
            try:
                program = '''import time
import gi
gi.require_version('Gio', '2.0')
from gi.repository import Gio, GLib
from asus_numberpad.sleep import SleepSignals
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
try:
    bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
        'org.freedesktop.DBus.Monitoring', 'BecomeMonitor', GLib.Variant('(asu)', ([], 0)),
        None, Gio.DBusCallFlags.NONE, 500, None)
except GLib.Error:
    pass
else:
    raise AssertionError('Test policy unexpectedly permits monitoring')
bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
    'org.freedesktop.DBus', 'RequestName', GLib.Variant('(su)', ('org.freedesktop.login1', 0)),
    None, Gio.DBusCallFlags.NONE, 500, None)
item = SleepSignals()
assert not item.poll()
bus.emit_signal(None, '/org/freedesktop/login1', 'org.freedesktop.login1.Manager',
    'PrepareForSleep', GLib.Variant('(b)', (True,)))
bus.flush_sync(None)
deadline = time.monotonic() + 2
while not item.poll() and time.monotonic() < deadline:
    time.sleep(.01)
assert item.poll(), 'Ordinary signal subscription failed'
item.close()
print('PASS private system bus: Monitoring denied, PrepareForSleep received')
'''
                result = subprocess.run(['python3', '-c', program], env=os.environ | {'DBUS_SYSTEM_BUS_ADDRESS': address},
                                        capture_output=True, text=True, timeout=8)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            finally:
                daemon.terminate()
                daemon.wait(timeout=3)
                daemon.stdout.close()
                daemon.stderr.close()
