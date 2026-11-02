"""Resolve printed symbols through the actual session XKB map.

Use xkbcli's compositor/X11 connection; never guess DISPLAY, XAUTHORITY or a
keyboard layout. Multiple layout groups are refused until group tracking exists.
No physical keyboard device is opened. APIs from libxkbcommon, MIT licensed.
"""
import ctypes as C
import os
import subprocess

SYMBOLS = '0123456789/*-%.+='


def session_map():
    kind = os.environ.get('XDG_SESSION_TYPE')
    if kind not in ('x11', 'wayland'):
        raise ValueError('Run in a graphical X11 or Wayland session')
    if not os.environ.get('DISPLAY' if kind == 'x11' else 'WAYLAND_DISPLAY'):
        raise ValueError('Current session display environment is missing')
    option = '--format=v1' if kind == 'x11' else '--raw'
    return subprocess.check_output(['xkbcli', 'dump-keymap-' + kind, option], timeout=.5)


def resolve(source):
    lib = C.CDLL('libxkbcommon.so.0')
    def api(name, result, *args):
        function = getattr(lib, 'xkb_' + name)
        function.restype, function.argtypes = result, args
        return function
    ptr, u = C.c_void_p, C.c_uint32
    context = api('context_new', ptr, C.c_int)(0)
    if not context:
        raise ValueError('Cannot create XKB context')
    keymap = None
    try:
        keymap = api('keymap_new_from_string', ptr, ptr, C.c_char_p, C.c_int, C.c_int)(context, source, 1, 0)
        if not keymap:
            raise ValueError('Cannot parse session keymap')
        if api('keymap_num_layouts', u, ptr)(keymap) != 1:
            raise ValueError('Multiple XKB layout groups need active-group tracking; refusing activation')
        state = api('state_new', ptr, ptr)(keymap)
        if not state:
            raise ValueError('Cannot create XKB state')
        try:
            # Resolve actual modifier keys, including remapped Shift/AltGr.
            # Never synthesize a key merely because its usual evdev code is a
            # modifier: custom XKB options may have changed its meaning.
            update = api('state_update_mask', u, ptr, u, u, u, u, u, u)
            key_update = api('state_update_key', u, ptr, u, C.c_int)
            sym = api('state_key_get_one_sym', u, ptr, u)
            utf = api('state_key_get_utf32', u, ptr, u)
            shifts, thirds = [], []
            for code in range(9, 256):
                value = sym(state, code)
                if value in (0xffe1, 0xffe2):
                    shifts.append(code - 8)
                elif value in (0xfe03, 0xff7e):
                    thirds.append(code - 8)
            combos = [()] + [(key,) for key in shifts + thirds]
            combos += [(shift, third) for shift in shifts for third in thirds]
            result = {}
            # Numeric keypad digit/decimal levels change with NumLock, which
            # belongs to the user's session. Prefer stable non-keypad digits.
            unstable = {71, 72, 73, 75, 76, 77, 79, 80, 81, 82, 83, 95, 121}
            for modifiers in combos:
                update(state, 0, 0, 0, 0, 0, 0)
                for code in modifiers:
                    key_update(state, code + 8, 1)
                for key in range(9, 256):
                    if key - 8 in unstable or key - 8 in modifiers:
                        continue
                    char = utf(state, key)
                    if char and chr(char) in SYMBOLS:
                        result.setdefault(chr(char), (*modifiers, key - 8))
                for code in reversed(modifiers):
                    key_update(state, code + 8, 0)
            if set(SYMBOLS) - result.keys():
                raise ValueError('Session keymap cannot produce every printed b7402 symbol')
            for name, wanted in (('backspace', 0xff08), ('enter', 0xff0d)):
                update(state, 0, 0, 0, 0, 0, 0)
                matches = [key - 8 for key in range(9, 256) if sym(state, key) == wanted]
                if not matches:
                    raise ValueError(f'Session map lacks {name}')
                result[name] = (matches[0],)
            return result
        finally:
            api('state_unref', None, ptr)(state)
    finally:
        if keymap:
            api('keymap_unref', None, ptr)(keymap)
        api('context_unref', None, ptr)(context)
