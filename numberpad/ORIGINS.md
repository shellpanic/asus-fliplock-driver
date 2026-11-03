# Source origins and attribution

This component is GPL-2.0, retaining the licenses and contributor notices of:

* Fliplock: https://github.com/asus-linux-drivers/asus-fliplock-driver through
  d57274f5dfa076867ee23b4b15005350ec0a9854, merged preserving fork history.
  Original work by Lukas Drahnik and contributors. Legacy code remains separate.
* NumberPad: https://github.com/asus-linux-drivers/asus-numberpad-driver through
  ae5b3293681abec518b728a88da17e5e118bd4ce, comprising v7.3.0 (f458376) plus
  b87ae8d and ae5b329 dropped-input fixes. Original work by Mohamed Badaoui,
  Lukas Drahnik and contributors; recovery fixes credit Dmytro Gajewski and
  contributors listed in the upstream history and source. The complete source
  snapshot, contributor notices and GPL license are retained in vendor/upstream.

Fork-specific adaptation: separate evdev session adapter, exact discovery,
permission enrollment, explicit staged install, systemd/runit user supervision,
numbers-only exclusive touchpad access, off-at-start and fail-closed recovery.
The b7402 layout/padding, I2C packet protocol and physical-state recovery rules
come from NumberPad upstream. Runtime and setup are now maintained as Rust in src/;
top-level legacy install/uninstall entrypoints are retired; historical versions
remain in Git. No vendored installer, telemetry or package upgrade code is executed/installed.

The maintained fork version is independent of the imported upstream versions.
3.0.0 is a release target, not a claim of hardware acceptance. See README.

Rust dependency versions and declared licenses are recorded in Cargo.lock and
dependencies.json. Native package staging includes the dependency license files.
The Rust binary is a separate GPL executable, not linked into Lazyvoid.
