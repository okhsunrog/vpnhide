#!/system/bin/sh
# Late-start: load intent, recover journal, select backend, apply, then serve.
# Magisk disable: if $MODDIR/disable exists, managers skip this script — applied
# kernel/Ports policy is left intact (no unload). See docs/notes/core-2.0-module-lifecycle.md.
MODDIR="${0%/*}"
PERSIST="/data/adb/vpnhide/core"
CORE="$MODDIR/vpnhide-core"
SOCKET="$PERSIST/vpnhide-core.sock"
LOG="$PERSIST/core.log"

mkdir -p "$PERSIST"
chmod 700 "$PERSIST" 2>/dev/null
echo "phase=service" >"$PERSIST/boot_phase"

if [ ! -x "$CORE" ]; then
  echo "status=Refused detail=vpnhide-core missing" >"$PERSIST/boot_status"
  log -t vpnhide_core "vpnhide-core binary missing under $MODDIR"
  exit 0
fi

# Production writers: enabled only because this common module is installed.
# Dev sandboxes leave these unset. Do not export from random shells.
export VPNHIDE_CONFIG_PATH="/data/system/vpnhide_config.json"
export VPNHIDE_JOURNAL_PATH="$PERSIST/operations.json"
export VPNHIDE_CORE_OWNER_DIR="$PERSIST"
export VPNHIDE_CORE_ADOPT=1
export VPNHIDE_BOOT_APPLY=1
export VPNHIDE_KMOD_CTL="/proc/vpnhide_ctl"
export VPNHIDE_PORTS=1
# Ports marker colocated with other ownership markers.
export VPNHIDE_PORTS_OWNER_MARKER="$PERSIST/ports_owned"

# IPC auth (step 7): optional uid allowlist. Empty/missing = open (dev). Production
# app writes $PERSIST/allowed_uids after a one-shot su grant (reinstall/UID change
# rewrites the file). Env VPNHIDE_CORE_ALLOW_UIDS overrides the file when set.
export VPNHIDE_CORE_ALLOW_UIDS_FILE="$PERSIST/allowed_uids"
# Socket DAC: leave owner root; mode 0666 so untrusted_app can connectto; peer
# auth is the real gate. Tighten with VPNHIDE_CORE_SOCKET_OWNER_UID + 0660 when
# the allowlisted app uid is known at boot.
if [ -f "$PERSIST/allowed_uids" ]; then
  # Prefer first uid as socket owner when present (safe reinstall: app rewrites file).
  first_uid=$(awk '/^[0-9]+/{print; exit}' "$PERSIST/allowed_uids" 2>/dev/null)
  if [ -n "$first_uid" ]; then
    export VPNHIDE_CORE_SOCKET_OWNER_UID="$first_uid"
    export VPNHIDE_CORE_SOCKET_MODE=0660
  fi
fi

# Stop a previous instance from an interrupted upgrade (best-effort).
if [ -S "$SOCKET" ]; then
  # Asking the old process to exit is preferred; kill by socket peer is not portable.
  rm -f "$SOCKET"
fi

# Background: boot-apply runs inside the daemon when VPNHIDE_BOOT_APPLY=1, then serve.
(
  echo "phase=starting $(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date)" >>"$LOG"
  "$CORE" "$SOCKET" >>"$LOG" 2>&1
  echo "phase=exited rc=$? $(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date)" >>"$LOG"
) &

exit 0
