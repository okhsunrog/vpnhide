#!/system/bin/sh
VPNHIDE_CORE_MOD="/data/adb/modules/vpnhide_core"
if [ -d "$VPNHIDE_CORE_MOD" ] && [ ! -f "$VPNHIDE_CORE_MOD/disable" ]; then
  if [ -x "$VPNHIDE_CORE_MOD/vpnhide-core" ] || [ -f /data/adb/vpnhide/core/config_owned ]; then
    exit 0
  fi
fi
MODDIR="${0%/*}"
exec "$MODDIR/activator" boot-load
