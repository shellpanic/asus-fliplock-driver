#!/bin/sh
# The historical installer is preserved in Git history, never executed here.
cat >&2 <<'MESSAGE'
The legacy automatic installer is retired in this maintained fork.
For NumberPad, build a distribution package with numberpad/install STAGING_ROOT,
then review asus-numberpad setup plan/apply as the intended graphical user.
Legacy fliplock source remains separate and inactive; review its service and
configuration manually. See numberpad/README.md and LEGACY-FLIPLOCK.md.
MESSAGE
exit 2
