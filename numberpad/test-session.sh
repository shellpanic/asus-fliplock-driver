#!/bin/sh
# Supervisor plumbing only, with a fixture child; never open hardware devices.
set -eu
if [ "${NUMBERPAD_DISPOSABLE_TEST:-}" != 1 ] || [ ! -f /.dockerenv ]; then
    echo 'Disposable container only' >&2
    exit 1
fi
cd /source
python3 - <<'PY'
from pathlib import Path
from asus_numberpad.manage import UNIT
Path('/tmp/asus-numberpad.service').write_text(UNIT)
PY
if command -v systemd-analyze >/dev/null 2>&1; then
    # Installed executable is staged; verify the unit with a harmless command.
    sed 's@ExecStart=/usr/bin/asus-numberpad run@ExecStart=/bin/true@' /tmp/asus-numberpad.service > /tmp/asus-numberpad-test.service
    systemd-analyze verify /tmp/asus-numberpad-test.service
fi
useradd -m -u 1987 numberpadqa
mkdir -p /tmp/numberpadqa-runtime
chown numberpadqa /tmp/numberpadqa-runtime
chmod 700 /tmp/numberpadqa-runtime
# This test-only executable measures start/stop of the actual runit process
# tree. It is not the hardware driver and makes no input-integration claim.
cat > /usr/bin/asus-numberpad <<'PY'
#!/usr/bin/python3
import os, signal, time
from pathlib import Path
path = Path(os.environ['XDG_RUNTIME_DIR']) / 'events'
with path.open('a') as stream: stream.write('start\n')
def stop(*_):
    with path.open('a') as stream: stream.write('stop\n')
    raise SystemExit(0)
signal.signal(signal.SIGTERM, stop)
while True: time.sleep(.1)
PY
chmod 755 /usr/bin/asus-numberpad
cat > /tmp/numberpadqa-session.py <<'PY'
import time
from pathlib import Path
from asus_numberpad import manage, runtime
from unittest.mock import patch
for repeat in range(2):
    deadline = time.monotonic() + 4
    runtime.session_active = lambda _: time.monotonic() < deadline
    manage.session()
    time.sleep(.5)
with patch.object(manage, 'recover') as recover:
    manage.stop_session()
    recover.assert_called_once_with(missing_ok=True)
events = Path('/tmp/numberpadqa-runtime/events').read_text().splitlines()
assert events == ['start', 'stop', 'start', 'stop'], events
print('PASS real runit session supervisor start/stop/restart; fixture child only')
PY
su -s /bin/sh numberpadqa -c 'PYTHONPATH=/source/numberpad XDG_SESSION_ID=fixture XDG_RUNTIME_DIR=/tmp/numberpadqa-runtime python3 /tmp/numberpadqa-session.py'
if pgrep -u numberpadqa -x runsv >/dev/null 2>&1; then echo 'Orphaned runsv' >&2; exit 1; fi
# Canonical-root protection is exercised only inside this disposable container.
for root in /./ /tmp/..; do
    if numberpad/install "$root"; then echo 'Canonical root accepted' >&2; exit 1; fi
done
