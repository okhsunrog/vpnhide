#!/usr/bin/env bash
# Print the effective build version for vpnhide artifacts.
#
#   - HEAD on a tag vX.Y.Z        -> "X.Y.Z"          (release build)
#   - N commits after tag vX.Y.Z  -> "X.Y.Z-N-gSHA"   (dev build)
#   - working tree dirty          -> additional "-dirty" suffix
#   - no git / no matching tag    -> falls back to VERSION file
#
# Used by every packaging step (module.prop, APK versionName, CI artifact
# names) so dev builds are unambiguously identifiable at a glance.

set -euo pipefail
cd "$(dirname "$0")/.."

if git rev-parse --git-dir >/dev/null 2>&1 \
    && raw=$(git describe --tags --match 'v*' --dirty 2>/dev/null); then
    echo "${raw#v}"
else
    cat VERSION
fi
