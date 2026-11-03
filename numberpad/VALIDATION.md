# Rust validation boundary

The Rust migration replaces the maintained Python runtime and setup. It does
not inherit physical acceptance from the earlier Python build.

Local checks: 14 component tests and 5 private-process control tests pass,
including real US/German/French XKB maps and multigroup refusal, double-tap/hold
state handling, exact discovery and owned-file rules, private D-Bus sender
authentication, delay-inhibitor FD lifetime and the sleep transition race,
control-token/socket/kernel-lock identity and teardown completion records.

The disposable Void suite validates staged production feature exclusion and
runs a fixture-only binary through real authenticated Xvfb and runit supervision,
root file operations and restart/cleanup failures. Input/I2C/module/udev behavior
is synthetic. Debian's older native XKB CLI is refused and only component,
staging/control/bus checks count there. Actual systemd user manager, live
Wayland, suspend/fold and physical ExpertBook checks remain pending.

CI evidence belongs to the consuming Lazyvoid PR for the exact source pin.
Only Umatix self-hosted runners execute driver checks. Tests do not install on
the developer laptop or claim live hardware results.
