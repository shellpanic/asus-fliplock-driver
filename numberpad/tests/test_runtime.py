import array
import fcntl
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from asus_numberpad import manage
from asus_numberpad.inhibit import DelayInhibitor
from asus_numberpad.runtime import Slots, tablet_mode


class KernelStateTests(unittest.TestCase):
    def test_resync_restores_selector_ids_and_positions_without_replaying_keys(self):
        codes = SimpleNamespace(ABS_MT_SLOT=47, ABS_MT_TRACKING_ID=57,
                                ABS_MT_POSITION_X=53, ABS_MT_POSITION_Y=54)
        device = SimpleNamespace(fd=123, absinfo=lambda _: SimpleNamespace(max=4, value=3))
        def ioctl(fd, request, values, mutate):
            self.assertEqual(fd, 123)
            self.assertTrue(mutate)
            states = {57: [-1, -1, -1, 19, -1], 53: [0, 0, 0, 1900, 0], 54: [0, 0, 0, 50, 0]}
            values[1:] = array.array('i', states[values[0]])
        with patch('asus_numberpad.runtime.fcntl.ioctl', side_effect=ioctl):
            slots = Slots(device, codes)
        self.assertEqual(slots.slot, 3)
        self.assertEqual(slots.contacts(), [(19, 1900, 50)])
    def test_tablet_state_uses_switch_ioctl(self):
        def ioctl(fd, request, values, mutate):
            values[0] = 2
        with patch('asus_numberpad.runtime.fcntl.ioctl', side_effect=ioctl):
            self.assertTrue(tablet_mode(SimpleNamespace(fd=123)))


class PermissionTests(unittest.TestCase):
    def test_ambiguous_probe_still_revokes_owned_nodes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            rule = root / 'rule'
            sys = root / 'class'
            event = sys / 'input/event919'
            event.parent.mkdir(parents=True)
            event.touch()
            account = SimpleNamespace(pw_uid=1001)
            real_stat = Path.stat
            def stat(path, *args, **kwargs):
                if str(path) == '/dev/input/event919':
                    return SimpleNamespace(st_uid=1001)
                return real_stat(path, *args, **kwargs)
            with patch.object(manage, 'RULE', rule), patch.object(manage, 'SYS_CLASS', sys), patch.object(manage, 'ENROLLMENT', root / 'enrollment.json'), \
                 patch.object(manage.os, 'getuid', return_value=0), \
                 patch.object(manage.pwd, 'getpwnam', return_value=account), \
                 patch.object(manage, 'probe', side_effect=ValueError('ambiguous')), \
                 patch.object(manage.subprocess, 'run', return_value=SimpleNamespace(stdout='ASUS_NUMBERPAD_OWNER=testuser\nDEVNAME=/dev/input/event919\n')) as command, \
                 patch.object(manage.os, 'chown') as chown, patch.object(manage.os, 'chmod') as chmod, \
                 patch.object(Path, 'stat', stat):
                rule.write_text(manage.rules('testuser'))
                manage.permissions('testuser', False)
                self.assertFalse(rule.exists())
                previous = command.call_count
                manage.permissions('testuser', False)
                self.assertEqual(command.call_count, previous, 'Repeated disable must leave stale markers alone')
                chown.assert_called_once_with('/dev/input/event919', 0, 0)
                chmod.assert_any_call('/dev/input/event919', 0o600)
                self.assertIn(['udevadm', 'trigger', '--action=change', str(event.resolve())],
                              [call.args[0] for call in command.call_args_list])
    def test_setup_repeat_and_disable_retain_custom_configuration(self):
        with tempfile.TemporaryDirectory() as temporary:
            config = Path(temporary)
            custom = config / 'asus-numberpad/config.json'
            custom.parent.mkdir()
            custom.write_text('{"hold_seconds": 2.5, "personal": "retained"}')
            with patch.dict(os.environ, XDG_CONFIG_HOME=temporary, XDG_RUNTIME_DIR=temporary), \
                 patch.object(manage.os, 'getuid', return_value=1001), \
                 patch.object(manage.pwd, 'getpwuid', return_value=SimpleNamespace(pw_name='testuser')), \
                 patch.object(manage, 'probe', return_value={'profile': 'b7402'}), \
                 patch.object(manage.shutil, 'which', return_value='/fake/tool'), \
                 patch.object(manage.subprocess, 'run'), \
                 patch.dict('sys.modules', evdev=SimpleNamespace(InputDevice=object), gi=SimpleNamespace(require_version=lambda *_: None)), \
                 patch.object(manage.sys.stdin, 'isatty', return_value=True), \
                 patch('builtins.input', return_value='apply'), \
                 patch.object(manage, 'auth') as auth, patch.object(manage, 'stop_session'):
                for _ in range(2):
                    manage.setup(True, True)
                manage.setup(False, True)
                self.assertEqual(auth.call_count, 3)
                self.assertEqual(custom.read_text(), '{"hold_seconds": 2.5, "personal": "retained"}')
                self.assertTrue(all(not path.exists() for path in manage.user_files()))


class StoppedSessionTests(unittest.TestCase):
    def test_disable_after_supervisor_exit_revokes_and_preserves_config(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            runtime = root / 'runtime'
            (runtime / 'asus-numberpad-runit/driver/supervise').mkdir(parents=True)
            config = root / 'config'
            saved = config / 'asus-numberpad/config.json'
            saved.parent.mkdir(parents=True)
            saved.write_text('{"hold_seconds": 2.5}')
            def which(name):
                return '/usr/bin/sv' if name == 'sv' else None
            with patch.dict(os.environ, XDG_RUNTIME_DIR=str(runtime), XDG_CONFIG_HOME=str(config)), \
                 patch.object(manage.os, 'getuid', return_value=1001), \
                 patch.object(manage.pwd, 'getpwuid', return_value=SimpleNamespace(pw_name='testuser')), \
                 patch.object(manage.shutil, 'which', side_effect=which), \
                 patch.object(manage.subprocess, 'run', return_value=SimpleNamespace(returncode=1)), \
                 patch.object(manage, 'recover') as recover, patch.object(manage, 'auth') as auth, \
                 patch.object(manage.sys.stdin, 'isatty', return_value=True), \
                 patch('builtins.input', return_value='apply'):
                for path, value in manage.user_files().items():
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(value)
                manage.setup(False, True)
                auth.assert_called_once_with(False)
                recover.assert_called_once_with(missing_ok=True)
                self.assertTrue(all(not path.exists() for path in manage.user_files()))
                self.assertEqual(saved.read_text(), '{"hold_seconds": 2.5}')
    def test_live_parent_without_runsv_blocks_disable_before_revocation(self):
        with tempfile.TemporaryDirectory() as temporary:
            with patch.dict(os.environ, XDG_RUNTIME_DIR=temporary, XDG_CONFIG_HOME=temporary), \
                 patch.object(manage.os, 'getuid', return_value=1001), \
                 patch.object(manage.pwd, 'getpwuid', return_value=SimpleNamespace(pw_name='testuser')), \
                 patch.object(manage.sys.stdin, 'isatty', return_value=True), \
                 patch('builtins.input', return_value='apply'), \
                 patch.object(manage, 'auth') as auth, patch.object(manage, 'stop_session') as stop:
                # Same guard held by session() even before runsvdir creates a child.
                with manage.session_guard():
                    with self.assertRaisesRegex(ValueError, 'session supervisor'):
                        manage.setup(False, True)
                auth.assert_not_called()
                stop.assert_not_called()
    def test_disable_guard_remains_held_during_revocation(self):
        with tempfile.TemporaryDirectory() as temporary:
            def revoke(enable):
                self.assertFalse(enable)
                with self.assertRaisesRegex(ValueError, 'session supervisor'):
                    with manage.session_guard():
                        self.fail('session start entered during revocation')
            with patch.dict(os.environ, XDG_RUNTIME_DIR=temporary, XDG_CONFIG_HOME=temporary), \
                 patch.object(manage.os, 'getuid', return_value=1001), \
                 patch.object(manage.pwd, 'getpwuid', return_value=SimpleNamespace(pw_name='testuser')), \
                 patch.object(manage.sys.stdin, 'isatty', return_value=True), \
                 patch('builtins.input', return_value='apply'), \
                 patch.object(manage, 'auth', side_effect=revoke), patch.object(manage, 'stop_session'):
                manage.setup(False, True)
    def test_failed_stop_of_live_supervisor_refuses_recovery(self):
        with tempfile.TemporaryDirectory() as temporary:
            runtime = Path(temporary) / 'asus-numberpad-runit'
            supervise = runtime / 'driver/supervise'
            supervise.mkdir(parents=True)
            with (supervise / 'lock').open('a') as held:
                fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with patch.dict(os.environ, XDG_RUNTIME_DIR=temporary), \
                     patch.object(manage.shutil, 'which', side_effect=lambda name: '/usr/bin/sv' if name == 'sv' else None), \
                     patch.object(manage.subprocess, 'run', return_value=SimpleNamespace(returncode=1)), \
                     patch.object(manage, 'recover') as recover:
                    with self.assertRaisesRegex(ValueError, 'live runit supervisor'):
                        manage.stop_session()
                    recover.assert_not_called()
    def test_stale_supervisor_does_not_bypass_live_foreground_lock(self):
        with tempfile.TemporaryDirectory() as temporary:
            runtime = Path(temporary)
            (runtime / 'asus-numberpad-runit/driver/supervise').mkdir(parents=True)
            real_stat = Path.stat
            def stat(path, *args, **kwargs):
                result = real_stat(path, *args, **kwargs)
                if path == runtime:
                    values = list(result)
                    values[4] = 1001
                    return os.stat_result(values)
                return result
            with (runtime / 'asus-numberpad.lock').open('a') as held:
                fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with patch.dict(os.environ, XDG_RUNTIME_DIR=temporary), \
                     patch.object(manage.os, 'getuid', return_value=1001), \
                     patch.object(Path, 'stat', stat), \
                     patch.object(manage.shutil, 'which', side_effect=lambda name: '/usr/bin/sv' if name == 'sv' else None), \
                     patch.object(manage.subprocess, 'run', return_value=SimpleNamespace(returncode=1)), \
                     patch.object(manage, 'probe') as probe:
                    with self.assertRaisesRegex(ValueError, 'Driver still running'):
                        manage.stop_session()
                    probe.assert_not_called()


class ProcessTests(unittest.TestCase):
    def utility(self, directory):
        utility = directory / 'inhibit'
        utility.write_text('#!/bin/sh\nwhile [ "$#" -gt 0 ]; do case "$1" in --*) shift;; *) break;; esac; done\nexec "$@"\n')
        utility.chmod(0o755)
        return utility
    def test_inhibitor_child_exits_when_pipe_closes(self):
        with tempfile.TemporaryDirectory() as temporary:
            utility = self.utility(Path(temporary))
            with patch('asus_numberpad.inhibit.shutil.which', return_value=str(utility)):
                inhibitor = DelayInhibitor()
                self.assertIsNone(inhibitor.child.poll())
                inhibitor.close()
                self.assertEqual(inhibitor.child.returncode, 0)
    def test_parent_sigkill_does_not_leave_inhibitor_child(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            utility = self.utility(root)
            program = root / 'parent.py'
            program.write_text('from unittest.mock import patch\nfrom asus_numberpad.inhibit import DelayInhibitor\nimport time\n'
                               f'with patch("asus_numberpad.inhibit.shutil.which", return_value={str(utility)!r}):\n'
                               '    item = DelayInhibitor()\n    print(item.child.pid, flush=True)\n    time.sleep(30)\n')
            parent = subprocess.Popen(['python3', str(program)], stdout=subprocess.PIPE, text=True)
            child = int(parent.stdout.readline())
            parent.kill()
            parent.wait(timeout=3)
            parent.stdout.close()
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                proc = Path(f'/proc/{child}/stat')
                try:
                    state = proc.read_text().split()[2]
                except (FileNotFoundError, ProcessLookupError):
                    break
                if state == 'Z':
                    break
                time.sleep(.05)
            else:
                self.fail('Sleep inhibitor child survived parent death')
