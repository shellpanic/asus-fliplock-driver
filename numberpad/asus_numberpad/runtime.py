"""Session-bound runtime. All opened input devices are selected by discovery."""
import array
import fcntl
import os
from pathlib import Path
import selectors
import signal
import subprocess
import time

from .discovery import probe
from .engine import Engine
from .hardware import illuminate
from .keymap import session_map, resolve
from .inhibit import DelayInhibitor


def session_active(session):
    values = subprocess.check_output(['loginctl', 'show-session', session,
                                     '-p', 'Active', '-p', 'LockedHint', '-p', 'Type',
                                     '-p', 'User', '-p', 'Remote'], timeout=.5).decode().splitlines()
    values = dict(item.split('=', 1) for item in values if '=' in item)
    return (values.get('Active') == 'yes' and values.get('LockedHint') == 'no'
            and values.get('Type') == os.environ.get('XDG_SESSION_TYPE')
            and values.get('User') == str(os.getuid()) and values.get('Remote') == 'no')


class Slots:
    def __init__(self, device, codes):
        self.device, self.c = device, codes
        count = device.absinfo(codes.ABS_MT_SLOT).max + 1
        if not 1 <= count <= 32:
            raise ValueError('Unsupported multitouch slot count')
        self.count = count
        self.values, self.slot = {}, 0
        self.resync()

    def resync(self):
        c = self.c
        data = {}
        for code in (c.ABS_MT_TRACKING_ID, c.ABS_MT_POSITION_X, c.ABS_MT_POSITION_Y):
            values = array.array('i', [code] + [0] * self.count)
            # EVIOCGMTSLOTS: restore the physical source after SYN_DROPPED.
            request = (2 << 30) | (len(values) * values.itemsize << 16) | (ord('E') << 8) | 0x0a
            fcntl.ioctl(self.device.fd, request, values, True)
            data[code] = values[1:]
        self.values = {i: {code: data[code][i] for code in data} for i in range(self.count)}
        self.slot = self.device.absinfo(c.ABS_MT_SLOT).value

    def event(self, event):
        if event.code == self.c.ABS_MT_SLOT:
            if not 0 <= event.value < self.count:
                raise ValueError('Invalid multitouch slot')
            self.slot = event.value
        elif event.code in self.values[self.slot]:
            self.values[self.slot][event.code] = event.value

    def contacts(self):
        c = self.c
        return [(v[c.ABS_MT_TRACKING_ID], v[c.ABS_MT_POSITION_X], v[c.ABS_MT_POSITION_Y])
                for v in self.values.values() if v[c.ABS_MT_TRACKING_ID] >= 0]


def tablet_mode(device):
    # EVIOCGSW; python-evdev has no active_switches API.
    bits = array.array('B', [0] * 8)
    fcntl.ioctl(device.fd, (2 << 30) | (len(bits) << 16) | (ord('E') << 8) | 0x1b, bits, True)
    return bool(bits[0] & 2)


def run(hold=1.0):
    from evdev import InputDevice, UInput, ecodes as c
    if os.getuid() == 0:
        raise ValueError('Run as the enrolled graphical user, never root')
    session = os.environ.get('XDG_SESSION_ID', '')
    if not session or not session_active(session):
        raise ValueError('An active unlocked local logind/elogind session is required')
    runtime = Path(os.environ['XDG_RUNTIME_DIR'])
    if runtime.stat().st_uid != os.getuid():
        raise ValueError('Runtime directory must belong to the session user')
    lock = (runtime / 'asus-numberpad.lock').open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    device = probe()
    pressed = []
    touch = switch = output = monitor = engine = inhibitor = None
    grabbed = False
    stopping = False
    old_handlers = {}
    def stop(*_):
        nonlocal stopping
        stopping = True
    def grab(enabled):
        nonlocal grabbed
        if enabled != grabbed:
            (touch.grab if enabled else touch.ungrab)()
            grabbed = enabled
    def send(key):
        while pressed:
            output.write(c.EV_KEY, pressed.pop(), 0)
        if key is not None:
            for code in mapping[key]:
                output.write(c.EV_KEY, code, 1)
                pressed.append(code)
        output.syn()
    try:
        # Reset interrupted illumination before opening/grabbing any input.
        illuminate(device['adapter'], False)
        mapping_source = session_map()
        mapping = resolve(mapping_source)
        touch = InputDevice(device['event'])
        switch = InputDevice(device['switch']['event'])
        slots = Slots(touch, c)
        x, y = touch.absinfo(c.ABS_MT_POSITION_X), touch.absinfo(c.ABS_MT_POSITION_Y)
        output = UInput({c.EV_KEY: sorted({code for keys in mapping.values() for code in keys})},
                        name='ASUS NumberPad numbers only')
        engine = Engine(send, grab, lambda on: illuminate(device['adapter'], on),
                        (x.min, x.max, y.min, y.max), hold)
        engine.blocked = bool(slots.contacts())
        # A delay inhibitor prevents sleep until cleanup completes. The monitor
        # sees PrepareForSleep, and exiting releases the inherited inhibitor FD.
        monitor = subprocess.Popen(['dbus-monitor', '--system',
            "type='signal',sender='org.freedesktop.login1',interface='org.freedesktop.login1.Manager',member='PrepareForSleep'"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        os.set_blocking(monitor.stdout.fileno(), False)
        # BecomeMonitor causes NameLost after subscription. NameAcquired alone
        # precedes subscription and cannot serve as the readiness barrier.
        sleep_buffer = b''
        deadline = time.monotonic() + 3
        with selectors.DefaultSelector() as ready:
            ready.register(monitor.stdout, selectors.EVENT_READ)
            while b'member=NameLost' not in sleep_buffer:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not ready.select(remaining):
                    raise ValueError('Suspend monitor did not become ready')
                message = os.read(monitor.stdout.fileno(), 8192)
                if not message:
                    raise ValueError('Suspend monitor exited during subscription')
                sleep_buffer += message
        if b'boolean true' in sleep_buffer:
            return
        inhibitor = DelayInhibitor()
        selector = selectors.DefaultSelector()
        selector.register(touch, selectors.EVENT_READ, 'touch')
        selector.register(switch, selectors.EVENT_READ, 'switch')
        selector.register(monitor.stdout, selectors.EVENT_READ, 'sleep')
        for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            old_handlers[sig] = signal.signal(sig, stop)
        tablet = tablet_mode(switch)
        dropped = False
        last_check = 0
        while not stopping:
            for key, _ in selector.select(.05):
                if key.data == 'sleep':
                    message = os.read(monitor.stdout.fileno(), 8192)
                    if not message:
                        raise ValueError('Lost suspend monitor')
                    sleep_buffer += message
                    lines = sleep_buffer.split(b'\n')
                    sleep_buffer = lines.pop()
                    if any(line.strip() == b'boolean true' for line in lines):
                        return
                    continue
                source = touch if key.data == 'touch' else switch
                for event in list(source.read()):
                    if event.type == c.EV_SYN and event.code == c.SYN_DROPPED:
                        engine.reset()
                        if source is touch:
                            dropped = True
                        else:
                            tablet = True
                        continue
                    if source is switch:
                        if event.type == c.EV_SW and event.code == c.SW_TABLET_MODE:
                            tablet = bool(event.value)
                            if tablet:
                                engine.reset()
                        elif event.type == c.EV_SYN and event.code == c.SYN_REPORT:
                            tablet = tablet_mode(switch)
                        continue
                    if event.type == c.EV_SYN and event.code == c.SYN_REPORT:
                        if dropped:
                            # Kernel state is ahead of buffered events. Discard
                            # the entire queue before reading its current state.
                            while True:
                                try:
                                    list(touch.read())
                                except BlockingIOError:
                                    break
                            slots.resync()
                            dropped = False
                            engine.blocked = True
                            break  # discard the remainder of this old batch
                        engine.frame(slots.contacts(), time.monotonic(), not tablet)
                    elif not dropped and event.type == c.EV_ABS:
                        slots.event(event)
            now = time.monotonic()
            if now - last_check >= .5:
                if not session_active(session):
                    return
                new_source = session_map()
                if new_source != mapping_source:
                    # Stop rather than inject keys using a stale map. Supervision
                    # can restart in the new map, always with NumberPad off.
                    return
                if monitor.poll() is not None:
                    raise ValueError('Suspend monitor exited')
                last_check = now
            if not dropped:
                engine.frame(slots.contacts(), now, not tablet)
    finally:
        try:
            if engine:
                engine.off()
            else:
                illuminate(device['adapter'], False)
        finally:
            for opened in (output, switch, touch):
                if opened:
                    opened.close()
            if inhibitor:
                inhibitor.close()
            if monitor:
                monitor.terminate()
                monitor.wait(timeout=3)
            for sig, handler in old_handlers.items():
                signal.signal(sig, handler)
            lock.close()
