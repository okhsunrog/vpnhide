#!/usr/bin/env bash
# Host check: common module scaffold is complete and legacy scripts prefer core.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MOD="$ROOT/core/module"
fail=0
need() {
  if [ ! -e "$1" ]; then
    echo "MISSING $1" >&2
    fail=1
  fi
}
need "$MOD/module.prop"
need "$MOD/customize.sh"
need "$MOD/post-fs-data.sh"
need "$MOD/service.sh"
need "$MOD/uninstall.sh"
need "$MOD/sepolicy.rule"
need "$MOD/legacy-prefer-core.sh"
need "$MOD/META-INF/com/google/android/update-binary"
grep -q '^id=vpnhide_core$' "$MOD/module.prop" || { echo "bad id"; fail=1; }
grep -q 'VPNHIDE_BOOT_APPLY=1' "$MOD/service.sh" || { echo "service missing BOOT_APPLY"; fail=1; }
grep -q 'VPNHIDE_CORE_ADOPT=1' "$MOD/service.sh" || { echo "service missing ADOPT"; fail=1; }
grep -q 'config_owned' "$MOD/post-fs-data.sh" || { echo "post-fs missing claim"; fail=1; }
grep -q 'markers and applied policy retained' "$MOD/uninstall.sh" || { echo "uninstall not restricted"; fail=1; }
# SuperKey must not be referenced for deletion
if grep -E 'superkey|SUPERKEY|/data/adb/ap' "$MOD/uninstall.sh" "$MOD/service.sh" "$MOD/post-fs-data.sh"; then
  echo "scripts must not touch APatch secrets" >&2
  fail=1
fi
for f in \
  "$ROOT/builtin/module/service.sh" \
  "$ROOT/kmod/module/service.sh" \
  "$ROOT/kmod/module/post-fs-data.sh" \
  "$ROOT/kmod/kpm/module/service.sh" \
  "$ROOT/kmod/kpm/module/post-fs-data.sh" \
  "$ROOT/zygisk/module/service.sh" \
  "$ROOT/portshide/module/service.sh"
do
  grep -q 'vpnhide_core' "$f" || { echo "legacy missing prefer-core: $f"; fail=1; }
done
if command -v shellcheck >/dev/null 2>&1; then
  shellcheck -s sh "$MOD/service.sh" "$MOD/post-fs-data.sh" "$MOD/uninstall.sh" "$MOD/customize.sh" \
    || fail=1
fi
if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "OK core module scaffold"
