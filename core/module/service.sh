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
