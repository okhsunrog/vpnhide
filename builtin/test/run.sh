#!/bin/sh
# The built-in backend shares the exact same typed harness and native probes.
set -eu
qemu_repo=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
exec "$qemu_repo/tools/vpnhide-tooling" qemu run --repo "$qemu_repo" --backend builtin "${1:-android14-6.1}"
