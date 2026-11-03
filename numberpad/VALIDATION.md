# NumberPad validation evidence

Implementation candidate; no validated hardware support is claimed.

| Environment | Evidence | Limit |
|---|---|---|
| Local Linux/Python, libxkbcommon 1.13.2 | 21 local tests pass; private-bus test skipped locally without PyGObject; all printed b7402 cells, exact/unknown/ambiguous discovery, icon hold/lift, cleanup, actual US/DE/FR keymap parsing, configuration preservation, permissions revocation after ambiguous discovery, slot-state ioctl recovery, inhibitor pipe lifetime/SIGKILL | Input/kernel calls mocked except keymap library and process lifetime; no physical input |
| Disposable Void glibc container | All 22 tests (no skips), restricted private system-style D-Bus signal test and package staging pass with Python evdev 1.7.1_2, xkbcommon tools 1.13.2_1, elogind 252.39_1; installed CLI --help succeeds; xkbcli dump subcommands/options verified | No booted runit graphical login or physical devices; final expanded suite rerun recorded in delivery |
| Disposable Debian 12/systemd packages | All 22 tests (no skips), restricted private system-style D-Bus signal test and staging pass | xkbcommon 1.5.0 lacks dump-keymap commands: runtime must refuse; install current tools >=1.8 explicitly, never automatic package upgrades. This is not Debian runtime acceptance |
| Real runit supervision (Void and Debian containers) | Session supervisor starts/stops/restarts twice without orphaned runsv; canonical root aliases refused | Harmless fixture child, no input hardware; systemd unit syntax also verified in Debian |
| X11 live input | Pending | Keymap library tests alone do not establish compositor input |
| Sway Wayland live input | Pending | No Wayland support acceptance claim |
| GNOME Wayland live input | Pending | No Wayland support acceptance claim |
| Plasma Wayland live input | Pending | No Wayland support acceptance claim |
| ExpertBook B5402FVA/04F3:3230 | Pending | Need icon/light/all printed keys, numbers-only, deactivation/tapping, fold/unfold, suspend/resume, service stop and interrupted restart with user present |

The fork release target is v2.0.0, independent of imported upstream NumberPad
v7.3.0 + b87ae8d/ae5b329. Do not promote this candidate as validated hardware
support. No upstream installer, physical hardware write, desktop restart or
production repository publication was performed during these checks.

## Void acceptance preparation fixes (2026-11-03)

Read-only physical sysfs discovery exposed input event paths incorrectly rooted
at `/dev`. Touchpad and isolated switch paths now use `/dev/input`; adapter and
uinput remain under `/dev`. Regression checks assert all four path types with
an injected device root.

Disable now tolerates the runtime directory left by an exited runit supervisor.
If `sv down` fails, it must acquire runsv's supervisor lock before attempting
recovery. A live supervisor or foreground driver still prevents cleanup.
Regressions cover stale-supervisor disable/config preservation, failed live
supervisor stop, and a live foreground lock. Disposable session tests also
exercise stop_session after the real fixture supervisor exits.

Local suite: 24 pass, one private-bus dependency skip. New checks use disposable
files/process locks and command doubles; no permission enrollment, hardware
input, I2C writes or session changes were performed. Distribution CI evidence
is recorded in the delivery PR. All live graphical and physical acceptance
rows above remain pending. Systemd acceptance remains deferred.
