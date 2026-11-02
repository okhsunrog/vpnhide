#!/bin/sh
# Host developer entrypoint. Default is an offline plan, not a device wipe.
# Execute only with an explicit serial and private resumable operation receipt.
# All reset/recovery policy belongs to the typed Rust executor and Core writer.
set -eu
reset_repo=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
exec "$reset_repo/tools/vpnhide-tooling" device-reset "$@"
