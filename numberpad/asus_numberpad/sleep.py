"""Ordinary logind D-Bus signal subscription, without monitoring privileges."""


class SleepSignals:
    def __init__(self):
        import gi
        gi.require_version('Gio', '2.0')
        from gi.repository import Gio, GLib
        self.gio, self.glib = Gio, GLib
        self.bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
        self.sleeping = False
        self.rule = ("type='signal',sender='org.freedesktop.login1',"
                     "path='/org/freedesktop/login1',"
                     "interface='org.freedesktop.login1.Manager',member='PrepareForSleep'")
        self.subscription = self.bus.signal_subscribe(
            'org.freedesktop.login1', 'org.freedesktop.login1.Manager',
            'PrepareForSleep', '/org/freedesktop/login1', None,
            Gio.DBusSignalFlags.NO_MATCH_RULE, self.changed)
        try:
            # Explicit successful AddMatch reply is our readiness barrier. This
            # is a normal signal subscription, not BecomeMonitor/eavesdropping.
            self.bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                'org.freedesktop.DBus', 'AddMatch', GLib.Variant('(s)', (self.rule,)),
                None, Gio.DBusCallFlags.NONE, 500, None)
        except BaseException:
            self.bus.signal_unsubscribe(self.subscription)
            raise

    def changed(self, connection, sender, path, interface, member, parameters):
        if parameters.unpack() == (True,):
            self.sleeping = True

    def poll(self):
        if self.bus.is_closed():
            raise ValueError('Lost logind signal connection')
        context = self.glib.MainContext.default()
        while context.pending():
            context.iteration(False)
        return self.sleeping

    def close(self):
        self.bus.signal_unsubscribe(self.subscription)
        if not self.bus.is_closed():
            self.bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                'org.freedesktop.DBus', 'RemoveMatch',
                self.glib.Variant('(s)', (self.rule,)), None,
                self.gio.DBusCallFlags.NONE, 500, None)
