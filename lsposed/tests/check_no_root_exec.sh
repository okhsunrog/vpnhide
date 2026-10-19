#!/bin/sh
# The 2.0 app runs no root process: core-daemon is the only one, and the app reaches it
# over the typed core-ffi client (collect_diagnostics / root_effect / config edits). Fail
# if app source spawns `su` or any other process.
set -eu
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SRC="$ROOT/lsposed/app/src/main"
fail=0
check() {
  if grep -rnE --include='*.kt' --include='*.java' -e "$1" "$SRC"; then
    echo "app source must not $2" >&2
    fail=1
  fi
}
check '"su"' 'name the su binary'
check '"su[[:space:]]' 'run a su command line'
check '/s?bin/su\b' 'reference a su path'
check 'Runtime\.getRuntime\(\)\.exec' 'exec a process'
check 'ProcessBuilder\(' 'spawn a process'
check 'suExec' 'keep a su helper'
if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "OK app runs no root process"
