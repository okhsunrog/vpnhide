#!/usr/bin/env bash
# Host check: common core module scaffold (single-writer; no legacy activator prefer-core).
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
need "$MOD/META-INF/com/google/android/update-binary"
# Dual-writer shim must be gone
if [ -e "$MOD/legacy-prefer-core.sh" ]; then
  echo "legacy-prefer-core.sh must be deleted" >&2
  fail=1
fi
grep -q '^id=vpnhide_core$' "$MOD/module.prop" || { echo "bad id"; fail=1; }
grep -q 'VPNHIDE_BOOT_APPLY=1' "$MOD/service.sh" || { echo "service missing BOOT_APPLY"; fail=1; }
grep -q 'VPNHIDE_CORE_ADOPT=1' "$MOD/service.sh" || { echo "service missing ADOPT"; fail=1; }
grep -q 'config_owned' "$MOD/post-fs-data.sh" || { echo "post-fs missing claim"; fail=1; }
grep -q 'markers and applied policy retained' "$MOD/uninstall.sh" || { echo "uninstall not restricted"; fail=1; }
if grep -E 'superkey|SUPERKEY|/data/adb/ap' "$MOD/uninstall.sh" "$MOD/service.sh" "$MOD/post-fs-data.sh"; then
  echo "scripts must not touch APatch secrets" >&2
  fail=1
fi
# Artifact modules must NOT ship activator boot scripts
for f in   "$ROOT/kmod/module/service.sh"   "$ROOT/kmod/module/post-fs-data.sh"   "$ROOT/kmod/module/uninstall.sh"   "$ROOT/kmod/kpm/module/service.sh"   "$ROOT/kmod/kpm/module/post-fs-data.sh"   "$ROOT/kmod/kpm/module/uninstall.sh"   "$ROOT/zygisk/module/service.sh"   "$ROOT/zygisk/module/uninstall.sh"
do
  if [ -e "$f" ]; then
    echo "activator boot/uninstall must be deleted: $f" >&2
    fail=1
  fi
done
if [ -d "$ROOT/portshide" ]; then
  echo "portshide/ must be deleted" >&2
  fail=1
fi
if [ -d "$ROOT/builtin/module" ]; then
  echo "builtin/module companion must be deleted" >&2
  fail=1
fi
if command -v shellcheck >/dev/null 2>&1; then
  shellcheck -s sh "$MOD/service.sh" "$MOD/post-fs-data.sh" "$MOD/uninstall.sh" "$MOD/customize.sh"     || fail=1
fi
if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "OK core module scaffold (single-writer)"
