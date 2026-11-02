"""Numbers-only state machine; coordinates/padding derived from upstream b7402.

GPL-2.0. Original NumberPad authors and upstream revision: ../ORIGINS.md.
Dropped frames release output and discard state through the next SYN_REPORT;
only a fresh complete contact can activate again. No recovered contact is a tap.
"""
KEYS = (('7', '8', '9', '/', 'backspace'),
        ('4', '5', '6', '*', 'backspace'),
        ('1', '2', '3', '-', '%'),
        ('0', '.', 'enter', '+', '='))


class Engine:
    def __init__(self, output, grab, light, bounds, hold=1.0):
        self.output, self.grab, self.light = output, grab, light
        self.bounds, self.hold = bounds, hold
        xmin, xmax, ymin, ymax = bounds
        if xmax - xmin <= 400 or ymax - ymin <= 500 or not 0.2 <= hold <= 5:
            raise ValueError('Invalid profile bounds or hold time')
        self.active = False
        self.blocked = False
        self.contact = None
        self.held = False
        self.down = None

    def off(self):
        # Attempt every cleanup action even if one device has disappeared.
        errors = []
        for action in (lambda: self.output(None), lambda: self.light(False), lambda: self.grab(False)):
            try:
                action()
            except OSError as error:
                errors.append(error)
        self.active = False
        self.contact, self.down, self.held = None, None, False
        if errors:
            raise errors[0]

    def reset(self):
        self.off()
        self.blocked = True

    def frame(self, contacts, now, allowed=True):
        if not allowed:
            if self.active or self.contact is not None:
                self.off()
            self.blocked = bool(contacts)
            return
        if self.blocked:
            if not contacts:
                self.blocked = False
                self.contact, self.down, self.held = None, None, False
            return
        if len(contacts) != 1:
            self.output(None)
            self.down = None
            self.contact, self.held = None, False
            return
        identity, x, y = contacts[0]
        xmin, xmax, ymin, ymax = self.bounds
        icon = x >= xmax - 250 and y <= ymin + 250
        if self.contact is None or self.contact[0] != identity:
            self.contact = (identity, now, icon)
            self.held = False
        if icon and self.contact[2] and not self.held:
            if now - self.contact[1] >= self.hold:
                if self.active:
                    self.off()
                else:
                    # Grab before illumination; rollback handled by runtime finally.
                    self.grab(True)
                    self.active = True
                    self.light(True)
                self.held = True
                self.blocked = True  # require lift after either toggle
            return
        if self.contact[2]:
            self.contact = (identity, now, False)
            self.blocked = True
            return  # sliding out of the icon is never a key or activation
        key = None
        if self.active and xmin + 200 <= x < xmax - 200 and ymin + 200 <= y < ymax - 80:
            col = min(4, int((x - xmin - 200) * 5 / (xmax - xmin - 400)))
            row = min(3, int((y - ymin - 200) * 4 / (ymax - ymin - 280)))
            key = KEYS[row][col]
        if key != self.down:
            self.output(key)
            self.down = key
