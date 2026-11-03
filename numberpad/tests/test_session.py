import os
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from asus_numberpad.keymap import session_map
from asus_numberpad.runtime import session_active, run
from asus_numberpad.session import display_kind


class SessionTests(unittest.TestCase):
    def env(self, **extra):
        return {'XDG_SESSION_TYPE': 'tty', 'DISPLAY': ':0', 'XDG_SESSION_ID': 'fixture', **extra}
    def properties(self, **extra):
        values = dict(Active='yes', LockedHint='no', Type='tty', User=str(os.getuid()), Remote='no')
        values.update(extra)
        return ''.join(f'{key}={value}\n' for key, value in values.items()).encode()
    def test_startx_map_uses_existing_local_display_without_rewriting_environment(self):
        with patch.dict(os.environ, self.env(), clear=True), \
             patch('asus_numberpad.keymap.subprocess.check_output', return_value=b'actual map') as command:
            self.assertEqual(session_map(), b'actual map')
            command.assert_called_once_with(['xkbcli', 'dump-keymap-x11', '--format=v1'], timeout=.5)
            self.assertEqual(os.environ['XDG_SESSION_TYPE'], 'tty')
    def test_startx_requires_active_unlocked_local_owned_matching_login(self):
        with patch.dict(os.environ, self.env(), clear=True):
            for extra in ({}, {'Active': 'no'}, {'LockedHint': 'yes'}, {'Remote': 'yes'},
                          {'User': str(os.getuid()+1)}, {'Type': 'x11'}):
                with patch('asus_numberpad.runtime.subprocess.check_output', return_value=self.properties(**extra)):
                    self.assertEqual(session_active('fixture'), not bool(extra), extra)
    def test_tty_without_unambiguous_local_x_or_with_ssh_is_refused(self):
        for extra in ({'DISPLAY': ''}, {'DISPLAY': 'localhost:10.0'}, {'DISPLAY': 'host:0'},
                      {'WAYLAND_DISPLAY': 'wayland-1'}, {'SSH_CONNECTION': 'forwarded'},
                      {'SSH_CLIENT': 'forwarded'}, {'SSH_TTY': '/dev/pts/1'}):
            with self.subTest(extra=extra), patch.dict(os.environ, self.env(**extra), clear=True):
                with self.assertRaises(ValueError):
                    display_kind()
                with patch('asus_numberpad.runtime.subprocess.check_output', return_value=self.properties()):
                    self.assertFalse(session_active('fixture'))
    def test_native_x11_and_wayland_and_type_mismatch(self):
        for kind, display in (('x11', {'DISPLAY': ':0'}), ('wayland', {'WAYLAND_DISPLAY': 'wayland-1'})):
            with patch.dict(os.environ, {'XDG_SESSION_TYPE': kind, **display}, clear=True):
                self.assertEqual(display_kind(), kind)
                with patch('asus_numberpad.runtime.subprocess.check_output', return_value=self.properties(Type=kind)):
                    self.assertTrue(session_active('fixture'))
                with patch('asus_numberpad.runtime.subprocess.check_output', return_value=self.properties(Type='tty')):
                    self.assertFalse(session_active('fixture'))
    def test_unreachable_startx_display_never_opens_input_or_writes_i2c(self):
        with tempfile.TemporaryDirectory() as temporary:
            runtime = Path(temporary)
            real_stat = Path.stat
            def stat(path, *args, **kwargs):
                result = real_stat(path, *args, **kwargs)
                if path == runtime:
                    values = list(result)
                    values[4] = 1001
                    return os.stat_result(values)
                return result
            fake = SimpleNamespace(InputDevice=object, UInput=object, ecodes=object)
            with patch.dict(os.environ, self.env(XDG_RUNTIME_DIR=temporary), clear=True), \
                 patch.dict('sys.modules', evdev=fake), \
                 patch('asus_numberpad.runtime.os.getuid', return_value=1001), \
                 patch.object(Path, 'stat', stat), \
                 patch('asus_numberpad.runtime.session_active', return_value=True), \
                 patch('asus_numberpad.runtime.probe', return_value={'adapter': '/dev/fixture'}), \
                 patch('asus_numberpad.runtime.session_map', side_effect=subprocess.TimeoutExpired('xkbcli', .5)), \
                 patch('asus_numberpad.runtime.illuminate') as light:
                with self.assertRaises(subprocess.TimeoutExpired):
                    run()
                light.assert_not_called()
