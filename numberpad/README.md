# ASUS NumberPad 1.0.0 (Rust)

Optional GPL-2.0 native session driver for exactly ASUS EXPERTBOOK
B5402FVA_B5402FVA, touchpad 04F3:3230, b7402 layout and an isolated tablet switch.
Unknown/ambiguous hardware and multiple XKB groups are refused. The maintained
runtime and setup are Rust. The upstream Python snapshot is retained only for
attribution and protocol/layout reference; it is never installed or executed.

The user tested the preceding Rust candidate on this exact ExpertBook under local
X11/Void/elogind: installation with automatic setup/startup, digits 0–9, the
printed / * - + % . = symbols, Enter and double-tap activation worked. One-action
removal was read back to confirm package/enrollment/startup/process absence and
retained settings; normal pointer movement/tapping worked after removal. These
observations do not establish physical fold/suspend, crash, all key cells,
modifier combinations, other desktops or other hardware acceptance.

1.0.0 names the first stable Rust series. The earlier 3.0.0_1 package was an
experimental migration candidate; the consuming XBPS package explicitly reverts
that version so a normal update can adopt this series without force flags.
Python 2.x installations must be disabled/removed with their old version before
installing this Rust series; they are not silently adopted.

## User flow

Use Lazyvoid Install / enable ASUS NumberPad from the intended active local
unlocked graphical account. Its review includes the package transaction and
permission/module/startup scope; native sudo/doas handles privileged steps.
Install loads uinput and i2c-dev, persists those module requests, enrolls exact
selected devices and starts supervision immediately, with the keypad off.
Remove stops authenticated owned processes, resets illumination before revoking
permissions, removes owned startup/module configuration and removes the package.
Settings and shared dependencies are retained. No module is unloaded.

```
asus-numberpad status
asus-numberpad install plan
asus-numberpad install apply
asus-numberpad gesture double-tap plan
asus-numberpad gesture double-tap apply
asus-numberpad gesture hold apply
asus-numberpad disable plan
asus-numberpad disable apply
```

Setup is an alias for install; run starts an attached diagnostic runtime;
session starts supervision; recover resets illumination under the runtime lock.
Normal Lazyvoid use needs none of these manual lifecycle commands. Applying a
standalone plan requires typing apply and native authentication. --yes is used
by the already reviewed Lazyvoid action and never bypasses native authentication.

Double-tap the upper-right icon to toggle, lifting between taps (each <=250ms,
gap <=400ms). Movement/multiple contacts cancel it. Hold remains selectable.
Double-tap while off passes ordinary taps to the desktop, so use the icon over
a harmless desktop area during acceptance. While on only the chosen touchpad
is grabbed. Cleanup releases generated keys and pointer control before resetting illumination on
stop, errors, tablet mode, sleep and session loss; resume starts off. Physical
keyboard, external mouse and touchscreen are not opened. No tapping setting is
rewritten. Mutable physical modifiers and every printed symbol need live testing.

Permissions cover the selected touchpad, isolated tablet switch, entire resolved
I2C adapter (every address), and uinput (arbitrary input injection), without
physical keyboard or input-group grants. Discovery resolves current event/bus
identities each time. Only exact owned enrollment/startup files can be changed;
modified or foreign files cause refusal. An incomplete teardown blocks removal
rather than abandoning a surviving supervisor. If the session supervisor itself
is forcibly killed, reboot to clear its private
runtime state; automatic removal refuses unverified surviving descendants. Failed setup leaves the package
and owned state available for repeat Install or Remove; no rollback is promised.

## Dependencies and sessions

Target: native Linux glibc, libxkbcommon with xkbcli dump-keymap-x11 and
dump-keymap-wayland >=1.8, logind/elogind with loginctl, D-Bus, udev, kmod,
and runit or a live systemd user manager. No target Python, evdev binding,
GObject/Gio, cairo, pip or Rust compiler. Input/uinput/I2C use native Linux FDs;
D-Bus sleep monitoring uses zbus with a real delay-inhibitor FD.

Console-started X11 with tty session metadata is accepted only for a local :N
display, authenticated real XKB connection, and active matching elogind account;
SSH/Wayland ambiguity is refused. Environment variables are not rewritten.
Runit supervision uses the current user session; XDG autostart resumes it on
later graphical logins. Sessions without XDG autostart need their native startup
to invoke asus-numberpad session. No desktop is restarted. Debian 12's xkbcommon
1.5 lacks the required CLI and is refused for live operation. Systemd and live
Wayland are separate pending acceptance targets.

## Building and disposable validation

Rust >=1.88, cargo and a native compiler are build dependencies. Cargo.lock pins
all crates. Python is used only by the build license-inventory utility.

```
cargo test --locked --all-features
cargo build --locked --release
NUMBERPAD_BINARY=target/release/asus-numberpad ./install /absolute/stage
```

install stages an already-built native binary and dependency notices, refuses
the host root and never enrolls/starts hardware. Production builds use no
features. build-info must report test_fixtures:false. The optional test-fixtures
feature is restricted to marked disposable Docker fixtures; staging refuses it
outside explicit container QA. It must never be distributed to real machines.

Umatix CI in Lazyvoid checks the exact source pin using test-void.Dockerfile,
test-debian.Dockerfile and test-native.sh. Private D-Bus/X servers exercise real
protocols and runit processes; synthetic device nodes and mocked module/udev
commands exercise filesystem lifecycle. They do not test real kernel input,
illumination or actual suspend. No host input/I2C devices are exposed. See
VALIDATION.md and ORIGINS.md. No GitHub-hosted driver runners are configured.
