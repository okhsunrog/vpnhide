#!/system/bin/sh
# Restricted maintenance uninstall (core-2.0 §11).
# - Stop the daemon and remove the module's runtime socket/log.
# - Do NOT unload kernel hooks, wipe Ports chains, delete SuperKey, or restore
#   independent ordinary writers. Markers stay so legacy activators keep no-op
#   until an admin explicitly clears /data/adb/vpnhide/core (recovery path).
MODDIR="${0%/*}"
PERSIST="/data/adb/vpnhide/core"
SOCKET="$PERSIST/vpnhide-core.sock"

rm -f "$SOCKET"
rm -f "$PERSIST/core.log"
rm -f "$PERSIST/boot_phase"
# Leave: operations.json, boot_status, *_owned markers, Ports rules, ctl state.
log -t vpnhide_core "uninstall: module removed; markers and applied policy retained for recovery"
exit 0
