#!/system/bin/sh
# Prefer common core when installed (step 5). Markers also make activator no-op.
VPNHIDE_CORE_MOD="/data/adb/modules/vpnhide_core"
if [ -d "$VPNHIDE_CORE_MOD" ] && [ ! -f "$VPNHIDE_CORE_MOD/disable" ]; then
  if [ -x "$VPNHIDE_CORE_MOD/vpnhide-core" ] || [ -f /data/adb/vpnhide/core/config_owned ]; then
    log -t vpnhide "$0: skipped — vpnhide_core owns boot apply"
    exit 0
  fi
fi
# Late (post-boot) retry of config delivery, in case /proc/vpnhide_ctl or the
# config store was not ready yet in post-fs-data. Idempotent: boot-service just
# re-verifies the backend id, re-delivers the config, and refreshes load_status.
MODDIR="${0%/*}"
"$MODDIR/activator" boot-service &
exit 0
