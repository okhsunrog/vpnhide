#!/bin/sh
# Post-release entrypoint. The shared Rust tool owns update-feed policy.
set -eu
update_repo=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
exec "$update_repo/tools/vpnhide-tooling" distribution update-feed --repo "$update_repo"
