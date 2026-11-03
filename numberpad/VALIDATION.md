# NumberPad 1.0.0 validation

## Automated evidence

27 Rust component tests and 6 private-process IPC tests pass locally without
skips. The production runtime uses the tested event decoder and hardware guard;
these tests do not take the fixture shortcut that bypasses hardware startup.

Covered behavior includes:

- real US/German/French XKB maps and multigroup refusal;
- double-tap/hold cancellation and every mapped grid cell;
- held-key folding, unfolding without reactivation, pending-gesture cancellation,
  touch/switch SYN_DROPPED, current-state resynchronization, all-fingers-up gating,
  invalid slots and bounded draining of a continuously producing input stream;
- failed grab refusing illumination, normal release ordering, errors during key
  release/ungrab/light-off, RAII cleanup on unwinding and initially-off startup;
- private D-Bus sender authentication, inhibitor FD lifetime, the sleep transition
  during acquisition, login-manager loss/replacement during startup/operation and
  bus disconnect while the bus or old owner may otherwise stay alive;
- process/token/kernel-lock identity, runtime SIGKILL with stale endpoint recovery,
  completed teardown requirements, unknown-file preservation and survivor refusal.

The disposable Void suite adds real authenticated private Xvfb, real runit
pause/resume, SIGKILL runtime respawn with a fresh control token, repeat install,
partial enrollment, failed readiness and checked-recovery cleanup. Device nodes,
I2C, modules and udev actions are synthetic. Debian's older XKB CLI is refused;
component/staging/control/bus tests there do not establish graphical compatibility.
Lazyvoid additionally exercises native XBPS candidate3.0.0_1-to-release1.0.0_1
version migration without force flags, settings preservation and full one-action
Install/Remove in a disposable container. Exact final CI evidence is recorded in
the consuming Lazyvoid PR. All driver CI uses Umatix self-hosted runners.

## User-observed physical evidence

On the exact B5402FVA / 04F3:3230 ExpertBook under Void/elogind/local startx X11,
the preceding Rust candidate installed through one Lazyvoid action and reported
current-session readiness. The user entered 0123456789/*-+%.= and Enter and
confirmed double-tap activation. Removal through Lazyvoid completed; read-back
found no package, managed processes, root enrollment/module persistence, user
startup or udev owner markers. Settings remained. The user confirmed normal
pointer movement and tapping after removal. This is evidence from one laptop;
it does not establish every physical grid cell or modifier combination.

## Remaining physical boundaries

Actual fold/unfold, suspend/resume, kernel-input crash behavior, modifier keys,
all printed cells, live Wayland and a real systemd user manager remain unverified.
Synthetic tests cannot establish I2C timing, firmware behavior or real compositor
state during system sleep. Only the known ExpertBook profile is selected.

The runtime can be restarted off after SIGKILL under an intact supervisor.
SIGKILL of the session supervisor itself is different: unknown remaining
restarters cannot be safely adopted or killed. Cleanup deliberately retains the
package/grants and refuses unverified teardown. A reboot
clears the private session state; no broad pkill or arbitrary PID cleanup is used.

Production builds exclude test-fixtures. Public source release/tag does not
publish signed XBPS artifacts or enable the storage-gated production channel.
