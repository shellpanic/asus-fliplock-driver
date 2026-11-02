"""Hold a logind sleep-delay inhibitor only for the lifetime of its parent pipe.

The supervised process is the driver itself. SIGKILL closes stdin as well, so
neither an orphaned driver nor a persistent sleep inhibitor is left behind.
"""
import selectors
import shutil
import subprocess


class DelayInhibitor:
    def __init__(self):
        utility = shutil.which('systemd-inhibit') or shutil.which('elogind-inhibit')
        if not utility:
            raise ValueError('A logind/elogind delay inhibitor is required')
        self.child = subprocess.Popen([utility, '--what=sleep', '--mode=delay',
            '--who=ASUS NumberPad', '--why=Release input before sleep',
            '/bin/sh', '-c', 'printf "ready\\n"; exec cat >/dev/null'],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(self.child.stdout, selectors.EVENT_READ)
                if not selector.select(5) or self.child.stdout.readline() != b'ready\n':
                    raise ValueError('Cannot acquire sleep-delay inhibitor')
        except BaseException:
            self.close()
            raise

    def close(self):
        self.child.stdin.close()
        try:
            self.child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.child.terminate()
            self.child.wait(timeout=3)
        self.child.stdout.close()
