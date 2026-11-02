#!/bin/sh
# Host bootstrap only; plans, execution and validation live in the shared CLI.
set -eu
qemu_repo=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
exec "$qemu_repo/tools/vpnhide-tooling" qemu run --repo "$qemu_repo" --backend kmod "${1:-android12-5.10}"
