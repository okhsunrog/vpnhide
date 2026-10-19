#!/system/bin/sh
# shellcheck disable=SC2034
SKIPUNZIP=0
MOD_VER="$(grep '^version=' "$MODPATH/module.prop" | cut -d= -f2)"
ui_print "- VPN Hide (KPM) ${MOD_VER:-unknown}"
ui_print "- Installing KPM artifact only"
ui_print "- Activation is owned exclusively by the vpnhide_core module"

PERSIST_DIR="/data/adb/vpnhide_kpm"
mkdir -p "$PERSIST_DIR"
set_perm "$PERSIST_DIR" 0 0 0755
[ -f "$MODPATH/vpnhide.kpm" ] && set_perm "$MODPATH/vpnhide.kpm" 0 0 0644

ui_print "- Install vpnhide_core for boot apply / KPM delivery"
ui_print "- Config: /data/system/vpnhide_config.json (via core)"
