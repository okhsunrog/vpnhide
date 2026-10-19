#!/system/bin/sh
# Restricted maintenance uninstall (core-2.0 §11).
# - Stop the daemon and remove the module's runtime socket/log.
# - Do NOT unload kernel hooks, wipe Ports chains, delete SuperKey, or restore
#   any other writer (there is none in 2.0). Journal and markers stay so a
#   reinstalled core recovers; an admin clears /data/adb/vpnhide/core to reset.
PERSIST="/data/adb/vpnhide/core"
SOCKET="$PERSIST/vpnhide-core.sock"

rm -f "$SOCKET"
rm -f "$PERSIST/core.log"
rm -f "$PERSIST/boot_phase"
# Leave: operations.json, boot_status, *_owned markers, Ports rules, ctl state.
log -t vpnhide_core "uninstall: module removed; markers and applied policy retained for recovery"
exit 0
