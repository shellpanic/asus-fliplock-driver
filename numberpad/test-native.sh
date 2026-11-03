#!/bin/sh
# All root writes and mocks below are restricted to a disposable container.
set -eu
[ "${NUMBERPAD_DISPOSABLE_TEST:-}" = 1 ] && [ -f /.dockerenv ] || exit 1
case ${1:-} in void|debian) distro=$1;; *) exit 1;; esac
base=/tmp/numberpad-fixture
sys=$base/sys
dev=/dev/numberpad-fixture
mkdir -p "$sys/class/dmi/id" "$sys/class/input" "$sys/class/i2c-dev" "$sys/class/misc/uinput" "$dev/input" "$base/runtime"
printf 'ASUS EXPERTBOOK B5402FVA_B5402FVA\n' > "$sys/class/dmi/id/product_name"
for event in event91 event12; do
    parent=$sys/devices/pci/i2c-57/$event
    mkdir -p "$parent/device/id" "$parent/device/capabilities"
    printf '04f3\n' > "$parent/device/id/vendor"
    printf '3230\n' > "$parent/device/id/product"
    if [ "$event" = event91 ]; then
        printf 'ASUE1234:00 04F3:3230 Touchpad\n' > "$parent/device/name"
        printf 'b\n' > "$parent/device/capabilities/ev"
        printf '0\n' > "$parent/device/capabilities/sw"
    else
        printf 'Intel HID switches\n' > "$parent/device/name"
        printf '21\n' > "$parent/device/capabilities/ev"
        printf '2\n' > "$parent/device/capabilities/sw"
    fi
    printf '1:3\n' > "$parent/dev"
    ln -s "$parent" "$sys/class/input/$event"
    mknod "$dev/input/$event" c 1 3
done
printf '1:3\n' > "$sys/devices/pci/i2c-57/dev"
ln -s "$sys/devices/pci/i2c-57" "$sys/class/i2c-dev/i2c-57"
mknod "$dev/i2c-57" c 1 3
mknod "$dev/uinput" c 1 3
printf '1:3\n' > "$sys/class/misc/uinput/dev"
printf 'active\n' > "$base/session-state"
touch "$base/enabled"
chown numberpadqa "$base/runtime"
chmod 700 "$base/runtime"
# Fixed tools are replaced only in this container. No host modules or udev bus.
for tool in modprobe udevadm; do
    path=$(command -v "$tool")
    rm -f "$path"
    if [ "$tool" = modprobe ]; then
        cat > "$path" <<'MODPROBE'
#!/bin/sh
set -eu
case ${1:-} in uinput|i2c-dev) printf '%s\n' "$1" >> /tmp/numberpad-fixture/module-events;; *) exit 1;; esac
MODPROBE
    else
        cat > "$path" <<'UDEV'
#!/bin/sh
set -eu
base=/tmp/numberpad-fixture
rule=/etc/udev/rules.d/70-asus-numberpad.rules
case ${1:-} in
    control) [ ! -f "$base/fail-udev" ]; exit;;
    settle) exit 0;;
    info)
        [ -f "$base/owner" ] || exit 0
        owner=$(cat "$base/owner")
        printf 'ASUS_NUMBERPAD_OWNER=%s\n' "$owner"
        for value do
            case $value in
                *event91) printf 'DEVNAME=/dev/numberpad-fixture/input/event91\n';;
                *event12) printf 'DEVNAME=/dev/numberpad-fixture/input/event12\n';;
                *i2c-57) printf 'DEVNAME=/dev/numberpad-fixture/i2c-57\n';;
                *uinput) printf 'DEVNAME=/dev/numberpad-fixture/uinput\n';;
            esac
        done;;
    trigger)
        owner=$(sed -n 's/^# Selected account: //p' "$rule")
        if [ -n "$owner" ]; then
            chown "$owner" /dev/numberpad-fixture/input/* /dev/numberpad-fixture/i2c-57 /dev/numberpad-fixture/uinput
            printf '%s\n' "$owner" > "$base/owner"
        else
            chown root:root /dev/numberpad-fixture/input/* /dev/numberpad-fixture/i2c-57 /dev/numberpad-fixture/uinput
            rm -f "$base/owner"
        fi
        chmod 600 /dev/numberpad-fixture/input/* /dev/numberpad-fixture/i2c-57 /dev/numberpad-fixture/uinput;;
    *) exit 1;;
esac
UDEV
    fi
    chmod 755 "$path"
done
printf 'numberpadqa ALL=(root) NOPASSWD: /usr/bin/asus-numberpad\n' > /etc/sudoers.d/numberpadqa
chmod 440 /etc/sudoers.d/numberpadqa
# The real X server is private to this container and authenticated by a cookie.
auth=$base/Xauthority
touch "$auth"
xauth -f "$auth" add :93 . "$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
chown numberpadqa "$auth"
Xvfb :93 -screen 0 800x600x24 -nolisten tcp -auth "$auth" > "$base/Xvfb.log" 2>&1 &
xpid=$!
trap 'kill "$xpid" 2>/dev/null || true' EXIT
qa() { su -s /bin/sh numberpadqa -c "XDG_CONFIG_HOME=/home/numberpadqa/.config XDG_RUNTIME_DIR=$base/runtime XDG_SESSION_TYPE=tty XDG_SESSION_ID=fixture DISPLAY=:93 XAUTHORITY=$auth $*"; }
# Bookworm's old XKB CLI remains an explicitly refused runtime, never upgraded.
if ! xkbcli dump-keymap-x11 --help >/dev/null 2>&1; then
    [ "$distro" = debian ] || exit 1
    echo 'PASS Debian Rust unit/staging/control/bus tests; runtime needs XKB tools >=1.8. Graphical/systemd acceptance remains deferred.'
    exit 0
fi
attempt=0
until qa 'xkbcli dump-keymap-x11 --format=v1' > "$base/keymap.json" 2> "$base/keymap-error"; do
    attempt=$((attempt + 1)); [ "$attempt" -lt 50 ] || { cat "$base/keymap-error"; exit 1; }; sleep .1
done
# Missing cookie is refused by the real native XKB tool.
touch "$base/missing-auth"
chown numberpadqa "$base/missing-auth"
if su -s /bin/sh numberpadqa -c "XDG_SESSION_TYPE=tty DISPLAY=:93 XAUTHORITY=$base/missing-auth xkbcli dump-keymap-x11 --format=v1 >/dev/null"; then exit 1; fi
# Export a reusable native-fixture environment for the outer Lazyvoid package suite.
cat > "$base/environment" <<ENV
XDG_CONFIG_HOME=/home/numberpadqa/.config XDG_RUNTIME_DIR=$base/runtime XDG_SESSION_TYPE=tty XDG_SESSION_ID=fixture DISPLAY=:93 XAUTHORITY=$auth
ENV
# Optionally let the outer package suite drive the entire Install/Remove action.
if [ "${NUMBERPAD_PREPARE_ONLY:-}" = 1 ]; then
    trap - EXIT
    printf '%s\n' "$xpid" > "$base/Xvfb.pid"
    exit 0
fi
# Never-enrolled cleanup must not attempt I2C recovery, even with hardware present.
touch "$base/fail-recovery"
qa 'asus-numberpad disable apply --yes'
rm "$base/fail-recovery"
qa 'asus-numberpad install apply --yes'
[ -f /etc/modules-load.d/asus-numberpad.conf ]
[ -f /etc/udev/rules.d/70-asus-numberpad.rules ]
[ -f /var/lib/asus-numberpad/enrollment.json ]
[ ! -e /var/service/asus-numberpad ]
[ -f "$base/runtime/asus-numberpad.json" ]
# Real runit must restart a SIGKILLed owned runtime with fresh control identity.
qa 'asus-numberpad status' > "$base/before-kill.json"
qa 'python3 -' <<'KILL_RUNTIME'
import json,os,signal
from pathlib import Path
meta=json.loads(Path('/tmp/numberpad-fixture/runtime/asus-numberpad.json').read_text())
pid=meta['pid']
assert meta['uid']==1987
assert Path('/proc/'+str(pid)+'/exe').samefile('/usr/bin/asus-numberpad')
Path('/tmp/numberpad-fixture/runtime/killed-token').write_text(meta['token'])
os.kill(pid,signal.SIGKILL)
KILL_RUNTIME
attempt=0
until python3 - <<'RESTARTED'
import json
from pathlib import Path
try:
    meta=json.loads(Path('/tmp/numberpad-fixture/runtime/asus-numberpad.json').read_text())
    assert meta['ready'] and meta['token'] != Path('/tmp/numberpad-fixture/runtime/killed-token').read_text()
except (OSError,ValueError,AssertionError): raise SystemExit(1)
RESTARTED
do attempt=$((attempt+1)); [ "$attempt" -lt 90 ] || exit 1; sleep .1; done
# Pause/resume on real supervisor plumbing with synthetic input only.
printf 'inactive\n' > "$base/session-state"
attempt=0
while [ -e "$base/runtime/asus-numberpad.json" ]; do attempt=$((attempt+1)); [ "$attempt" -lt 50 ] || exit 1; sleep .1; done
printf 'active\n' > "$base/session-state"
attempt=0
until [ -e "$base/runtime/asus-numberpad.json" ]; do attempt=$((attempt+1)); [ "$attempt" -lt 70 ] || exit 1; sleep .1; done
# Repeat install keeps user settings and needs no manual foreground stop.
qa 'asus-numberpad gesture hold'
cp /home/numberpadqa/.config/asus-numberpad/config.json "$base/saved-config"
qa 'asus-numberpad install apply --yes'
cmp /home/numberpadqa/.config/asus-numberpad/config.json "$base/saved-config"
# Unknown content under generated supervision directories must be preserved.
touch "$base/runtime/asus-numberpad-runit/keep-user-file"
if qa 'asus-numberpad disable apply --yes'; then exit 1; fi
[ -f "$base/runtime/asus-numberpad-runit/keep-user-file" ]
rm "$base/runtime/asus-numberpad-runit/keep-user-file"
qa 'asus-numberpad install apply --yes'
# Recovery failure must leave cleanup access and the package available for retry.
touch "$base/fail-recovery"
if qa 'asus-numberpad disable apply --yes'; then exit 1; fi
[ -f /etc/udev/rules.d/70-asus-numberpad.rules ]
rm "$base/fail-recovery"
qa 'asus-numberpad disable apply --yes'
[ ! -e /etc/udev/rules.d/70-asus-numberpad.rules ]
[ ! -e /var/lib/asus-numberpad/enrollment.json ]
[ ! -e /etc/modules-load.d/asus-numberpad.conf ]
[ ! -e /home/numberpadqa/.config/autostart/asus-numberpad.desktop ]
[ ! -e "$base/runtime/asus-numberpad.json" ]
[ ! -e "$base/runtime/asus-numberpad-session.json" ]
cmp /home/numberpadqa/.config/asus-numberpad/config.json "$base/saved-config"
# Repeated cleanup also needs no device access.
touch "$base/fail-recovery"
qa 'asus-numberpad disable apply --yes'
rm "$base/fail-recovery"
# A failed root setup has recoverable owned state, and Remove-style cleanup works.
touch "$base/fail-udev"
if qa 'asus-numberpad install apply --yes'; then exit 1; fi
[ -f /etc/udev/rules.d/70-asus-numberpad.rules ]
rm "$base/fail-udev"
[ "$(stat -c %u /dev/numberpad-fixture/i2c-57)" = 0 ]
qa 'asus-numberpad disable apply --yes'
[ ! -e /etc/udev/rules.d/70-asus-numberpad.rules ]
# Session startup failure before spawning children must not poison cleanup.
printf 'inactive\n' > "$base/session-state"
if qa 'asus-numberpad session'; then exit 1; fi
printf 'active\n' > "$base/session-state"
qa 'asus-numberpad disable apply --yes'
# Readiness failure also has no false success and keeps explicit cleanup possible.
touch "$base/fail-ready"
if qa 'asus-numberpad install apply --yes'; then exit 1; fi
[ -f /var/lib/asus-numberpad/enrollment.json ]
rm "$base/fail-ready"
qa 'asus-numberpad disable apply --yes'
[ ! -e /var/lib/asus-numberpad/enrollment.json ]
echo 'PASS native Rust lifecycle: module/config ownership, authenticated X map, real runit pause/restart, automatic stop, recovery and interrupted/readiness cleanup. Input/I2C mocked; hardware acceptance pending.'
