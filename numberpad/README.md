# ASUS NumberPad component (2.0.0 candidate)

Optional session driver for the illuminated keypad inside an ASUS touchpad.
The initial exact profile is **ASUS EXPERTBOOK B5402FVA_B5402FVA + 04F3:3230 →
b7402**, from the imported upstream database. **Physical acceptance pending.**
Unknown or ambiguous hardware, missing tablet-switch isolation, multiple XKB
layout groups and missing session dependencies are refused. Do not infer support
for another ExpertBook from a similar product name.

The selected user runs the driver in an active unlocked local logind/elogind
X11 or Wayland session. It starts off. Hold the upper-right icon for one second
to activate/deactivate; lift after toggling. While active it exclusively grabs
only the selected touchpad. Touchscreen, external mouse and physical keyboards
are never grabbed or opened. It releases generated keys, disables illumination
and releases the grab on stop/error, tablet entry, session loss or suspend.
Unfolding/resume does not activate it. No compositor tapping settings are edited.

## Dependencies and installation

Distribution packages only: Python 3.10+, python-evdev, libxkbcommon (including
xkbcli dump-keymap-x11 and dump-keymap-wayland tools, version >=1.8), udev, dbus-monitor,
logind/elogind (loginctl and systemd-inhibit/elogind-inhibit), and either systemd
user services or runit (runsvdir/sv). Void currently packages the Python binding
as python3-evdev; libxkbcommon-tools supplies xkbcli. Debian with sufficiently recent xkbcommon tools uses
python3-evdev, libxkbcommon-tools, dbus, libpam-systemd and systemd. Debian 12
xkbcommon 1.5 lacks the required commands and is refused; staged installation
and unit tests there do not prove runtime compatibility.

`numberpad/install /absolute/staging/root` stages files for a native package.
It refuses `/` and does not install dependencies, enroll permissions, start a
service, modify a session or run either upstream installer. Legacy fliplock is
not installed by this entrypoint. For removal, run `disable apply` as the enrolled
user before removing the package. Package upgrades retain user configuration;
there are no package lifecycle hooks or automatic service restarts.

## Commands (as the intended regular user)

```
asus-numberpad probe --json
asus-numberpad setup plan
asus-numberpad setup apply
asus-numberpad session
asus-numberpad run
asus-numberpad disable plan
asus-numberpad disable apply
asus-numberpad recover
```

Probe is read-only. Plan shows exact current nodes, profile and proposed files.
Apply repeats discovery, asks for `apply`, then uses native sudo/doas authentication
for only the fixed root-owned permission rule. The root helper independently
rediscovers hardware. No session is restarted. Autostart begins on next graphical
login; Sway and other sessions without XDG autostart must invoke `asus-numberpad
session` in their own session startup. Runit supervises in the current session
runtime directory, never a root /var/service daemon. Systemd starts a user unit
using an explicit allowlist imported from the current session environment.
Concurrent sessions are unsupported; one per-user runtime lock prevents two
runtimes in one session user runtime directory.

Permissions grant the selected account read/write access to exactly the matched
touchpad and isolated tablet switch, **the entire matched I2C adapter** (including
other devices on it), and **uinput**, which permits arbitrary input injection.
This is a meaningful local trust grant. It adds no input group membership and
no access to physical keyboard event devices. Rules resolve nodes dynamically
on every udev event. Event and bus numbers are not saved in user configuration.
Only one enrolled user is supported. Disable stops the owned supervisor and
removes owned startup and rules, resets device permissions and reruns distribution
udev rules. Modified/unowned files are refused, never removed. Configuration at
`$XDG_CONFIG_HOME/asus-numberpad/config.json` (default ~/.config) is retained;
`hold_seconds` defaults to 1.0 (range .2–5).

`recover` requires the runtime to have stopped, acquires its lock and turns off
illumination. The kernel releases grabs and uinput devices on process death.
SIGKILL/power loss cannot run cleanup; restart/recover resets illumination before
any input grab. Stop a foreground `run` process before disabling. An unplugged
unknown device cannot be illuminated by recovery; permission/startup removal can
still proceed. Never use a stale bus number for recovery.

## Acceptance and limits

The adapter uses the actual X11/Wayland XKB map via xkbcli, refusing multiple
groups until group tracking is implemented. Keyboard layout changes stop it;
restart explicitly with the new layout. Holding physical modifiers while using
the keypad needs acceptance testing. It does not read physical keyboards to
cancel those modifiers. This restriction must be resolved before general support
claims. No keyboard backlight or privacy-indicator behavior is included.

Unit/fault-injection and disposable staging evidence are separate from runtime
service testing, graphical X11/Sway/GNOME/Plasma integration and physical laptop
acceptance. All graphical and physical acceptance remains pending until recorded
in VALIDATION.md. No automatic desktop switch, hardware install or release
publication is performed by tests. See ORIGINS.md for provenance and licensing.
