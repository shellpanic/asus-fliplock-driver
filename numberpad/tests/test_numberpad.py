import ctypes as C
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from asus_numberpad.discovery import MODEL, probe
from asus_numberpad.engine import Engine, KEYS
from asus_numberpad.hardware import illuminate
from asus_numberpad.keymap import resolve
from asus_numberpad import manage


class StateTests(unittest.TestCase):
    def setUp(self):
        self.events = []
        self.engine = Engine(lambda key: self.events.append(('key', key)),
                             lambda on: self.events.append(('grab', on)),
                             lambda on: self.events.append(('light', on)), (0, 2000, 0, 1200))
    def hold(self, start=0):
        self.engine.frame([(1, 1900, 50)], start)
        self.engine.frame([(1, 1900, 50)], start + 1.01)
    def test_default_off_and_continuous_hold(self):
        self.assertFalse(self.engine.active)
        self.engine.frame([(1, 1900, 50)], 0)
        self.engine.frame([(1, 1900, 50)], .99)
        self.assertFalse(self.engine.active)
        self.engine.frame([(1, 1900, 50)], 1.01)
        self.assertTrue(self.engine.active)
        self.assertEqual(self.events, [('grab', True), ('light', True)])
    def test_leave_icon_requires_lift_and_new_hold(self):
        self.engine.frame([(1, 1900, 50)], 0)
        self.engine.frame([(1, 1000, 600)], .5)
        self.engine.frame([(1, 1900, 50)], 2)
        self.assertFalse(self.engine.active)
        self.engine.frame([], 3)
        self.hold(4)
        self.assertTrue(self.engine.active)
    def test_every_printed_key(self):
        self.hold()
        self.engine.frame([], 2)
        for row in range(4):
            for col in range(5):
                self.engine.frame([(10 + row*5+col, 200 + (col+.5)*320, 200+(row+.5)*230)], 3)
                self.assertEqual(self.events[-1], ('key', KEYS[row][col]))
                self.engine.frame([], 4)
                self.assertEqual(self.events[-1], ('key', None))
    def test_toggle_requires_lift_and_never_repeats(self):
        self.hold()
        self.engine.frame([(1, 1900, 50)], 5)
        self.assertTrue(self.engine.active)
        self.engine.frame([], 6)
        self.hold(7)
        self.assertFalse(self.engine.active)
    def test_drop_tablet_and_cleanup(self):
        self.hold()
        self.engine.reset()
        self.assertFalse(self.engine.active)
        self.hold(4)
        self.assertFalse(self.engine.active)
        self.engine.frame([], 6)
        self.hold(7)
        self.assertTrue(self.engine.active)
        self.engine.frame([(1, 1900, 50)], 9, allowed=False)
        self.assertFalse(self.engine.active)
        self.assertEqual(self.events[-3:], [('key', None), ('light', False), ('grab', False)])
    def test_cleanup_attempts_ungrab_when_output_and_light_fail(self):
        def failure(_):
            raise OSError('disconnected')
        self.engine.output = self.engine.light = failure
        with self.assertRaises(OSError):
            self.engine.off()
        self.assertIn(('grab', False), self.events)
    def test_invalid_bounds(self):
        with self.assertRaises(ValueError):
            Engine(None, None, None, (0, 200, 0, 100))
    def test_unlock_before_illumination(self):
        with patch('asus_numberpad.hardware.packet') as packet:
            illuminate('/dev/selected-adapter', True)
            self.assertEqual([call.args for call in packet.call_args_list],
                             [('/dev/selected-adapter', 0x60), ('/dev/selected-adapter', 1)])


class DiscoveryTests(unittest.TestCase):
    def fixture(self, root, model=MODEL, duplicate=False):
        sys = root / 'sys'
        dmi = sys / 'class/dmi/id/product_name'
        dmi.parent.mkdir(parents=True)
        dmi.write_text(model)
        cls = sys / 'class/input'
        cls.mkdir(parents=True)
        for event, name, ev, sw in [('event91', 'ASUE1234:00 04F3:3230 Touchpad', 'b', '0'),
                                    ('event12', 'Intel HID switches', '21', '2')]:
            parent = sys / 'devices/pci0000:00/i2c-57' / event / 'device'
            (parent / 'id').mkdir(parents=True)
            (parent / 'capabilities').mkdir()
            for rel, value in [('name', name), ('id/vendor', '04f3'), ('id/product', '3230'),
                               ('capabilities/ev', ev), ('capabilities/sw', sw)]:
                (parent / rel).write_text(value)
            (cls / event).symlink_to(parent.parent)
        adapter = sys / 'class/i2c-dev/i2c-57'
        adapter.parent.mkdir(parents=True)
        adapter.symlink_to(sys / 'devices/pci0000:00/i2c-57')
        if duplicate:
            (cls / 'event92').symlink_to((cls / 'event91').resolve())
        return sys
    def test_exact_dynamic_match(self):
        with tempfile.TemporaryDirectory() as temporary:
            sys = self.fixture(Path(temporary))
            found = probe(sys, Path('/fake/dev'))
            self.assertEqual(found['profile'], 'b7402')
            self.assertEqual(found['event'], '/fake/dev/event91')
            self.assertEqual(found['adapter'], '/fake/dev/i2c-57')
            self.assertEqual(found['physical_acceptance'], 'pending')
    def test_unknown_and_ambiguous_refused(self):
        for kwargs in ({'model': 'B5402CBA'}, {'duplicate': True}):
            with tempfile.TemporaryDirectory() as temporary:
                with self.assertRaises(ValueError):
                    probe(self.fixture(Path(temporary), **kwargs))
    def test_keyboard_switch_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            sys = self.fixture(Path(temporary))
            (sys / 'class/input/event12/device/capabilities/ev').write_text('23')
            with self.assertRaises(ValueError):
                probe(sys)


class KeymapTests(unittest.TestCase):
    def source(self, layout):
        lib = C.CDLL('libxkbcommon.so.0')
        class Names(C.Structure):
            _fields_ = [(name, C.c_char_p) for name in ('rules', 'model', 'layout', 'variant', 'options')]
        lib.xkb_context_new.argtypes, lib.xkb_context_new.restype = [C.c_int], C.c_void_p
        lib.xkb_keymap_new_from_names.argtypes = [C.c_void_p, C.POINTER(Names), C.c_int]
        lib.xkb_keymap_new_from_names.restype = C.c_void_p
        lib.xkb_keymap_get_as_string.argtypes = [C.c_void_p, C.c_int]
        lib.xkb_keymap_get_as_string.restype = C.c_void_p
        lib.xkb_keymap_unref.argtypes = lib.xkb_context_unref.argtypes = [C.c_void_p]
        context = lib.xkb_context_new(0)
        names = Names(b'evdev', b'pc105', layout.encode(), b'', b'')
        keymap = lib.xkb_keymap_new_from_names(context, C.byref(names), 0)
        self.assertTrue(keymap)
        pointer = lib.xkb_keymap_get_as_string(keymap, 1)
        source = C.string_at(pointer)
        libc = C.CDLL(None)
        libc.free.argtypes = [C.c_void_p]
        libc.free(pointer)
        lib.xkb_keymap_unref(keymap)
        lib.xkb_context_unref(context)
        return source
    def test_us_german_and_french_maps(self):
        maps = {layout: resolve(self.source(layout)) for layout in ('us', 'de', 'fr')}
        self.assertEqual(maps['us']['/'], (53,))
        self.assertNotEqual(maps['de']['/'], maps['us']['/'])
        self.assertEqual(maps['fr']['7'], (42, 8))
        for mapping in maps.values():
            self.assertTrue(set('0123456789/*-%.+=') <= mapping.keys())
    def test_multigroup_refused(self):
        with self.assertRaisesRegex(ValueError, 'Multiple'):
            resolve(self.source('us,de'))


class OwnershipTests(unittest.TestCase):
    def test_modified_and_symlink_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / 'owned'
            path.write_text('custom')
            with self.assertRaises(ValueError):
                manage.check_owned(path, 'expected')
            link = root / 'link'
            link.symlink_to(path)
            with self.assertRaises(ValueError):
                manage.check_owned(link, 'custom')
            self.assertEqual(path.read_text(), 'custom')
    def test_staged_install_does_not_enroll_and_preserves_other_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            marker = root / 'personal-config'
            marker.write_text('retained')
            installer = Path(__file__).resolve().parents[1] / 'install'
            subprocess.run([str(installer), temporary], check=True)
            subprocess.run([str(root / 'usr/bin/asus-numberpad'), '--help'], check=True, stdout=subprocess.DEVNULL)
            self.assertFalse((root / 'etc').exists())
            self.assertFalse((root / 'home').exists())
            self.assertEqual(marker.read_text(), 'retained')
            subprocess.run([str(installer), temporary], check=True)
            self.assertEqual(marker.read_text(), 'retained')


if __name__ == '__main__':
    unittest.main()
