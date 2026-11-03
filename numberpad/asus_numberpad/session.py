"""Select the real display protocol without changing the login environment."""
import os
import re


def display_kind():
    kind = os.environ.get('XDG_SESSION_TYPE')
    if kind in ('x11', 'wayland'):
        if not os.environ.get('DISPLAY' if kind == 'x11' else 'WAYLAND_DISPLAY'):
            raise ValueError('Current session display environment is missing')
        return kind
    # startx can retain the local console login's tty classification. Allow
    # only an unambiguous local X display; session_active still validates the
    # actual elogind login and session_map must authenticate to that X server.
    if (kind == 'tty' and not os.environ.get('WAYLAND_DISPLAY')
            and not any(os.environ.get(name) for name in ('SSH_CONNECTION', 'SSH_CLIENT', 'SSH_TTY'))
            and re.fullmatch(r':[0-9]+(?:\.[0-9]+)?', os.environ.get('DISPLAY', ''))):
        return 'x11'
    raise ValueError('Run in a graphical X11 or Wayland session; tty requires a local startx display')
