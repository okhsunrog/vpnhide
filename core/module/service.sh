#!/system/bin/sh
# Late-start: load intent, recover journal, select backend, apply, then serve.
# Magisk disable: if $MODDIR/disable exists, managers skip this script — applied
# kernel/Ports policy is left intact (no unload). See docs/notes/core-2.0-module-lifecycle.md.
MODDIR="${0%/*}"
PERSIST="/data/adb/vpnhide/core"
CORE="$MODDIR/vpnhide-core"
APP_DATA_DIR="/data/user_de/0/dev.okhsunrog.vpnhide"
SOCKET="$APP_DATA_DIR/vpnhide-core.sock"
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

# App IPC: core binds its socket inside the app's own device-encrypted data dir,
# owned by the app uid (0600) with the dir's SELinux label, and admits only root and
# that uid (SO_PEERCRED). Re-bound on reinstall / clear data. The app runs no root
# process. See crates/core-daemon/src/app_socket.rs.
export VPNHIDE_APP_DATA_DIR="$APP_DATA_DIR"
# Root work the app asks for runs here, as typed RPCs: fixed diagnostics probes
# (collect_diagnostics) and fixed effects (root_effect: secret key, legacy cleanup,
# full reset). Large probe output lands in $APP_DATA_DIR/vpnhide-core-export.
export VPNHIDE_ROOT_EFFECTS=1

# Background: boot-apply runs inside the daemon when VPNHIDE_BOOT_APPLY=1, then serve.
(
  {
    echo "phase=starting $(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date)"
    "$CORE" "$SOCKET" 2>&1
    echo "phase=exited rc=$? $(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date)"
  } >>"$LOG"
) &

exit 0
