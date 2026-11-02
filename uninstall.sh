#!/bin/sh
cat >&2 <<'MESSAGE'
Automatic legacy uninstallation is retired. For NumberPad, run
asus-numberpad disable plan and disable apply as the enrolled user before
removing its distribution package. User configuration is retained.
Review any historical fliplock installation separately; no broad files,
permissions or services will be removed by this entrypoint.
MESSAGE
exit 2
