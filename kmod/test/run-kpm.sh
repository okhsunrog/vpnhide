#!/bin/sh
# Host bootstrap only; the typed executor owns both immutable KPM boot phases.
set -eu
qemu_repo=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
exec "$qemu_repo/tools/vpnhide-tooling" qemu run --repo "$qemu_repo" --backend kpm "${1:-android12-5.10}"
