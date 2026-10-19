#!/usr/bin/env bash
# Host check: the device runs no vpnhide shell logic. Every module script is the
# irreducible manager stub, and core spawns no shell. Everything else is Rust in core.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MOD="$ROOT/core/module"
fail=0
bad() {
  echo "FAIL: $*" >&2
  fail=1
}
need() { [ -e "$1" ] || bad "missing $1"; }

need "$MOD/module.prop"
need "$MOD/service.sh"
need "$MOD/customize.sh"
need "$MOD/sepolicy.rule"
grep -q '^id=vpnhide_core$' "$MOD/module.prop" || bad "core module.prop id"

# 1. service.sh is exactly the exec stub: no env, paths, conditionals or backgrounding.
expected_service='#!/system/bin/sh
exec "${0%/*}/vpnhide-core" service'
[ "$(cat "$MOD/service.sh")" = "$expected_service" ] ||
  bad "core/module/service.sh must be exactly the exec stub:
$expected_service"

# 2. customize.sh only keeps this device's binary ($ARCH from the manager) executable.
expected_customize='#!/system/bin/sh
mv -f "$MODPATH/bin/$ARCH/vpnhide-core" "$MODPATH/vpnhide-core"
rm -rf "${MODPATH:?}/bin"
set_perm "$MODPATH/vpnhide-core" 0 0 0755'
[ "$(cat "$MOD/customize.sh")" = "$expected_customize" ] ||
  bad "core/module/customize.sh must be exactly the binary-placement hook:
$expected_customize"

# 3. No other script in any module tree: backend modules ship artifacts only (default
#    extraction installs them) and core needs no post-fs-data / uninstall hook.
INSTALLER="META-INF/com/google/android/update-binary"
for tree in core/module kmod/module kmod/kpm/module zygisk/module; do
  dir="$ROOT/$tree"
  [ -d "$dir" ] || {
    bad "missing module tree $tree"
    continue
  }
  while IFS= read -r -d '' file; do
    rel="${file#"$dir"/}"
    case "$tree/$rel" in
      core/module/service.sh | core/module/customize.sh) continue ;;
      */"$INSTALLER") continue ;;
    esac
    case "$rel" in
      *.sh) bad "$tree/$rel: module trees carry no scripts beyond the core stub" ;;
    esac
    if head -c 2 "$file" | grep -q '^#!'; then
      bad "$tree/$rel: executable script in a module tree"
    fi
  done < <(find "$dir" -type f -print0)
  # The recovery-flash shim is the one standard installer, identical everywhere.
  if [ -f "$dir/$INSTALLER" ]; then
    cmp -s "$dir/$INSTALLER" "$MOD/$INSTALLER" || bad "$tree/$INSTALLER differs from core's"
  fi
done
[ -d "$ROOT/portshide" ] && bad "portshide/ must be deleted"
[ -d "$ROOT/builtin/module" ] && bad "builtin/module companion must be deleted"
[ -d "$ROOT/crates/core-daemon/probes" ] && bad "core probes are Rust (src/device_probes), not scripts"

# 4. Core spawns no shell (and no su): logic is Rust; external binaries are invoked
#    directly. Applies to every crate linked into core, tests included.
if grep -rnE --include='*.rs' \
  -e 'Command::new\("(/system)?(/bin/)?(sh|bash|mksh|su)"\)' \
  -e '"/system/bin/sh"|"/bin/sh"' \
  -e '\.args?\(\[?"-c"' \
  "$ROOT/crates"; then
  bad "core must not spawn a shell"
fi
include_str_hits="$(grep -rnE --include='*.rs' 'include_str!\("[^"]*\.sh"\)' "$ROOT/crates" || true)"
[ -z "$include_str_hits" ] || bad "core must not embed shell scripts: $include_str_hits"

# Core is the only root process: the app must spawn none.
sh "$ROOT/lsposed/tests/check_no_root_exec.sh" || fail=1
if command -v shellcheck >/dev/null 2>&1; then
  shellcheck -s sh -e SC2034 "$MOD/service.sh" "$MOD/customize.sh" || fail=1
fi
if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "OK module scaffold: exec stub only, no shell logic on device"
