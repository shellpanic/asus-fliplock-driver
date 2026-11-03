#!/bin/sh
# Real authenticated X connection only; disposable container, no input/I2C nodes.
set -eu
if [ "${NUMBERPAD_DISPOSABLE_TEST:-}" != 1 ] || [ ! -f /.dockerenv ]; then
    echo 'Disposable container only' >&2
    exit 1
fi
useradd -m -u 1988 numberpadxqa
cat > /tmp/numberpad-startx-fixture.py <<'PYTEST'
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
from asus_numberpad.keymap import session_map, resolve

with tempfile.TemporaryDirectory() as temporary:
    auth = Path(temporary) / 'Xauthority'
    auth.touch()
    subprocess.run(['xauth', '-f', str(auth), 'add', ':93', '.', secrets.token_hex(16)], check=True)
    os.environ.update(XDG_SESSION_TYPE='tty', DISPLAY=':93', XAUTHORITY=str(auth))
    for name in ('WAYLAND_DISPLAY', 'SSH_CONNECTION', 'SSH_CLIENT', 'SSH_TTY'):
        os.environ.pop(name, None)
    server = subprocess.Popen(['Xvfb', ':93', '-screen', '0', '800x600x24',
                               '-nolisten', 'tcp', '-auth', str(auth)],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        deadline = time.monotonic() + 8
        while True:
            try:
                source = session_map()
                break
            except subprocess.SubprocessError:
                if server.poll() is not None or time.monotonic() >= deadline:
                    raise
                time.sleep(.1)
        resolve(source)
        assert os.environ['XDG_SESSION_TYPE'] == 'tty'
        denied = Path(temporary) / 'denied'
        denied.touch()
        os.environ['XAUTHORITY'] = str(denied)
        try:
            session_map()
        except subprocess.SubprocessError:
            pass
        else:
            raise AssertionError('X server accepted missing authentication')
        print('PASS tty/startx authenticated X keymap; missing cookie refused; no hardware input tested')
    finally:
        server.terminate()
        server.wait(timeout=5)
PYTEST
su -s /bin/sh numberpadxqa -c 'PYTHONPATH=/source/numberpad python3 /tmp/numberpad-startx-fixture.py'
