"""Explicit, owned permission enrollment and user-session startup."""
import json
from contextlib import contextmanager, nullcontext
import os
from pathlib import Path
import pwd
import re
import shutil
import subprocess
import sys

from .discovery import probe

BIN = '/usr/bin/asus-numberpad'
RULE = Path('/etc/udev/rules.d/70-asus-numberpad.rules')
SYS_CLASS = Path('/sys/class')
ENROLLMENT = Path('/var/lib/asus-numberpad/enrollment.json')
MARKER = '# Owned by shellpanic/asus-fliplock-driver NumberPad v2\n'
AUTOSTART = ('[Desktop Entry]\nType=Application\nName=ASUS NumberPad\n'
             'Exec=/usr/bin/asus-numberpad session\nTerminal=false\n'
             'Comment=Optional numbers-only keypad, initially off\n')
UNIT = ('[Unit]\nDescription=ASUS NumberPad in the enrolled graphical session\n'
        '[Service]\nType=simple\nExecStart=/usr/bin/asus-numberpad run\n'
        'Restart=on-failure\nRestartSec=3\nTimeoutStopSec=8\n')


def user_files():
    home = Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home() / '.config')))
    return {home / 'autostart/asus-numberpad.desktop': AUTOSTART,
            home / 'systemd/user/asus-numberpad.service': UNIT}


def check_owned(path, value):
    if path.is_symlink() or (path.exists() and path.read_text() != value):
        raise ValueError(f'Preserve existing unowned file: {path}')


def rules(user):
    if not re.fullmatch(r'[a-z_][a-z0-9_-]*\$?', user):
        raise ValueError('Unsupported account name')
    account = pwd.getpwnam(user)
    if not account.pw_uid:
        raise ValueError('Select a regular graphical user')
    match = f'PROGRAM="{BIN} _match %p"'
    return (MARKER + f'# Selected account: {user}\n'
            f'SUBSYSTEM=="input", KERNEL=="event*", {match}, RESULT=="input", RUN+="{BIN} _refresh %p", OWNER="{user}", MODE="0600", ENV{{ASUS_NUMBERPAD_OWNER}}="{user}"\n'
            f'SUBSYSTEM=="i2c-dev", {match}, RESULT=="adapter", OWNER="{user}", MODE="0600", ENV{{ASUS_NUMBERPAD_OWNER}}="{user}"\n'
            f'SUBSYSTEM=="misc", KERNEL=="uinput", OWNER="{user}", MODE="0600", ENV{{ASUS_NUMBERPAD_OWNER}}="{user}"\n')


def permissions(user, enable):
    if os.getuid() != 0:
        raise ValueError('Permission enrollment requires native authentication')
    content = rules(user)
    state = json.dumps({'owner': user}, sort_keys=True) + '\n'
    clearing = MARKER + f'ENV{{ASUS_NUMBERPAD_OWNER}}=="{user}", ENV{{ASUS_NUMBERPAD_OWNER}}=""\n'
    if RULE.is_symlink() or (RULE.exists() and RULE.read_text() not in (content, clearing)):
        raise ValueError(f'Preserve existing unowned file: {RULE}')
    check_owned(ENROLLMENT, state)
    if not enable and not RULE.exists() and not ENROLLMENT.exists():
        return  # already disabled; never act on stale udev properties
    if enable:
        if RULE.exists() and RULE.read_text() != content:
            raise ValueError('Finish interrupted disable before enrolling again')
    device = None
    try:
        device = probe()
    except (ValueError, OSError):
        if enable:
            raise
    if enable:
        ENROLLMENT.parent.mkdir(parents=True, exist_ok=True)
        if not ENROLLMENT.exists():
            with ENROLLMENT.open('x') as stream:
                stream.write(state)
        RULE.parent.mkdir(parents=True, exist_ok=True)
        if not RULE.exists():
            with RULE.open('x') as stream:
                stream.write(content)
            RULE.chmod(0o644)
    else:
        RULE.parent.mkdir(parents=True, exist_ok=True)
        RULE.write_text(clearing)
        RULE.chmod(0o644)
    subprocess.run(['udevadm', 'control', '--reload-rules'], check=True)
    paths = [Path('/sys/class/misc/uinput').resolve()]
    if device:
        paths += [Path(device['sysfs']), Path(device['adapter_sysfs']), Path(device['switch']['sysfs'])]
    # udev's ephemeral ownership marker survives a failed/ambiguous probe;
    # revocation must not depend on still having exactly one discoverable device.
    if not enable:
        for cls in ('input/event*', 'i2c-dev/i2c-*', 'misc/uinput'):
            for entry in SYS_CLASS.glob(cls):
                result = subprocess.run(['udevadm', 'info', '--query=property', '--path', str(entry)],
                                        check=True, capture_output=True, text=True)
                properties = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
                if properties.get('ASUS_NUMBERPAD_OWNER') != user:
                    continue
                node = properties.get('DEVNAME', '')
                if not node.startswith('/dev/') or '..' in Path(node).parts:
                    raise ValueError('Invalid owned udev node')
                try:
                    if Path(node).stat().st_uid == pwd.getpwnam(user).pw_uid:
                        os.chown(node, 0, 0)
                        os.chmod(node, 0o600)
                except FileNotFoundError:
                    pass
                paths.append(entry.resolve())
    for path in paths:
        if path.exists():
            subprocess.run(['udevadm', 'trigger', '--action=change', str(path)], check=True)
    subprocess.run(['udevadm', 'settle', '--timeout=5'], check=True)
    if not enable:
        RULE.unlink()
        subprocess.run(['udevadm', 'control', '--reload-rules'], check=True)
        if ENROLLMENT.exists():
            ENROLLMENT.unlink()


def match(path):
    try:
        device = probe(require_switch=False)
    except (ValueError, OSError):
        return
    path = str(Path('/sys' + path).resolve())
    if path == device['sysfs'] or (device['switch'] and path == device['switch']['sysfs']):
        print('input')
    elif path == device['adapter_sysfs']:
        print('adapter')


def refresh(path):
    if os.getuid() != 0:
        raise ValueError('Device-arrival permission refresh requires root')
    try:
        device = probe(require_switch=False)
    except (ValueError, OSError):
        return
    if str(Path('/sys' + path).resolve()) == device['sysfs']:
        # Adapter add precedes touchpad add. Reprocess only its resolved node
        # after input discovery is possible, with no persistent bus number.
        subprocess.run(['udevadm', 'trigger', '--action=change', device['adapter_sysfs']], check=True)


def auth(enable):
    utility = shutil.which('sudo') or shutil.which('doas')
    if not utility:
        raise ValueError('Install sudo or doas for explicit permission enrollment')
    subprocess.run([utility, BIN, '_permissions', pwd.getpwuid(os.getuid()).pw_name, 'enable' if enable else 'disable'], check=True)


def setup(enable, apply):
    if os.getuid() == 0:
        raise ValueError('Run setup/disable as the intended graphical user')
    files = user_files()
    for path, value in files.items():
        check_owned(path, value)
    if enable:
        print(json.dumps(probe(), indent=2))
        for binary in ('udevadm', 'loginctl', 'xkbcli'):
            if not shutil.which(binary):
                raise ValueError(f'Missing dependency: {binary}')
        if not (shutil.which('systemd-inhibit') or shutil.which('elogind-inhibit')):
            raise ValueError('Missing logind/elogind delay inhibitor')
        for kind in ('x11', 'wayland'):
            subprocess.run(['xkbcli', 'dump-keymap-' + kind, '--help'], check=True, stdout=subprocess.DEVNULL)
        import gi
        gi.require_version('Gio', '2.0')
        from evdev import InputDevice  # dependency check only; no device opened
        del InputDevice
        if not shutil.which('systemctl') and not shutil.which('runsvdir'):
            raise ValueError('Install runit or systemd user supervision')
    print(f'Root enrollment files: {RULE}, {ENROLLMENT}; no event/bus numbers retained')
    print(('Enroll' if enable else 'Remove') + ' device-scoped permissions for ' + pwd.getpwuid(os.getuid()).pw_name)
    print('Permission scope: selected touchpad and tablet switch; entire resolved I2C adapter (all its addresses); uinput can inject arbitrary input. No physical keyboard or input-group membership.')
    print(('Create' if enable else 'Remove owned') + ' user files: ' + ', '.join(str(p) for p in files))
    config = next(iter(files)).parents[1] / 'asus-numberpad/config.json'
    print(f'Configuration: {config}; preserve existing contents. Startup: off. No session restart.')
    print('XDG autostart starts supervision on the next graphical login. For sessions without XDG autostart, invoke asus-numberpad session from that session startup. Multiple XKB groups currently refused.')
    if not apply:
        return
    if not sys.stdin.isatty() or input('Type apply to accept these changes: ') != 'apply':
        raise ValueError('Setup cancelled')
    with session_guard() if not enable else nullcontext():
        if not enable:
            stop_session()
        auth(enable)
        for path, value in files.items():
            if enable:
                path.parent.mkdir(parents=True, exist_ok=True)
                if not path.exists():
                    with path.open('x') as stream:
                        stream.write(value)
            elif path.exists():
                path.unlink()
        if enable and not config.exists():
            config.parent.mkdir(parents=True, exist_ok=True)
            with config.open('x') as stream:
                json.dump({'hold_seconds': 1.0}, stream)
    print('Setup complete. Configuration retained; start in your current session explicitly with asus-numberpad session.' if enable else 'Disabled; user configuration retained.')


@contextmanager
def session_guard():
    """Serialize runit session lifetime against disable, including revocation."""
    import fcntl
    runtime = Path(os.environ['XDG_RUNTIME_DIR'])
    with (runtime / 'asus-numberpad-session.lock').open('a') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError('Stop the NumberPad session supervisor before disabling or starting another session') from None
        yield


def stop_session():
    if shutil.which('systemctl'):
        subprocess.run(['systemctl', '--user', 'stop', 'asus-numberpad.service'], check=False)
    runtime = Path(os.environ['XDG_RUNTIME_DIR']) / 'asus-numberpad-runit'
    if runtime.exists() and shutil.which('sv'):
        result = subprocess.run(['sv', 'down', str(runtime / 'driver')], check=False)
        if result.returncode:
            # session() leaves its directory after runsv exits. A stale directory
            # must not prevent revocation, but a live unresponsive runsv must.
            # runsv holds this flock for its entire lifetime; retain it while
            # recovery checks the separate foreground-runtime lock.
            import fcntl
            supervise = runtime / 'driver' / 'supervise'
            supervise.mkdir(parents=True, exist_ok=True)
            with (supervise / 'lock').open('a') as lock:
                try:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    raise ValueError('Unable to stop live runit supervisor; retry disable after stopping it') from None
                recover(missing_ok=True)
            return
    # Lock ownership coordinates recovery with a possibly foreground runtime.
    recover(missing_ok=True)


def recover(missing_ok=False):
    import fcntl
    runtime = Path(os.environ['XDG_RUNTIME_DIR'])
    if runtime.stat().st_uid != os.getuid() or os.getuid() == 0:
        raise ValueError('Recovery requires the enrolled user runtime directory')
    with (runtime / 'asus-numberpad.lock').open('a') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError('Driver still running; stop its supervisor/foreground process before recovery') from None
        from .hardware import illuminate
        try:
            device = probe()
        except (OSError, ValueError):
            if not missing_ok:
                raise
        else:
            illuminate(device['adapter'], False)
    print('Illumination off. Exited processes release their generated keys and exclusive grabs in the kernel.')


def session():
    if os.getuid() == 0:
        raise ValueError('Start in the selected user graphical session')
    if shutil.which('systemctl') and subprocess.run(['systemctl', '--user', 'show-environment'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
        # Explicit allowlist of the current session; no baked-in credentials.
        names = ['DISPLAY', 'WAYLAND_DISPLAY', 'XAUTHORITY', 'XDG_SESSION_TYPE',
                 'XDG_SESSION_ID', 'XDG_RUNTIME_DIR', 'DBUS_SESSION_BUS_ADDRESS', 'XDG_CONFIG_HOME']
        subprocess.run(['systemctl', '--user', 'unset-environment', *names], check=True)
        subprocess.run(['systemctl', '--user', 'import-environment', *[n for n in names if n in os.environ]], check=True)
        subprocess.run(['systemctl', '--user', 'daemon-reload'], check=True)
        subprocess.run(['systemctl', '--user', 'start', 'asus-numberpad.service'], check=True)
        return
    with session_guard():
        runtime = Path(os.environ['XDG_RUNTIME_DIR']) / 'asus-numberpad-runit'
        if runtime.parent.stat().st_uid != os.getuid():
            raise ValueError('Runtime directory owner mismatch')
        driver = runtime / 'driver'
        driver.mkdir(parents=True, exist_ok=True)
        run = driver / 'run'
        run.write_text('#!/bin/sh\nsleep 2\nexec /usr/bin/asus-numberpad run\n')
        run.chmod(0o700)
        # runsvdir inherits this graphical session environment. Its child runtime
        # exits and releases grabs on session loss. No /var/service root daemon.
        from .runtime import session_active
        import time
        import signal
        session_id = os.environ['XDG_SESSION_ID']
        if not session_active(session_id):
            raise ValueError('An active unlocked graphical session is required')
        child = subprocess.Popen(['runsvdir', str(runtime)])
        stopping = False
        def stop(*_):
            nonlocal stopping
            stopping = True
        previous = {sig: signal.signal(sig, stop) for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP)}
        try:
            while not stopping and child.poll() is None and session_active(session_id):
                time.sleep(.5)
        finally:
            subprocess.run(['sv', 'down', str(driver)], check=False)
            subprocess.run(['sv', 'exit', str(driver)], check=False)
            child.send_signal(signal.SIGHUP)
            child.wait(timeout=5)
            for sig, handler in previous.items():
                signal.signal(sig, handler)
