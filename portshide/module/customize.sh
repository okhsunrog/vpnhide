#!/system/bin/sh
SKIPUNZIP=0
MOD_VER="$(grep '^version=' "$MODPATH/module.prop" | cut -d= -f2)"
ui_print "- VPN Hide (Ports) ${MOD_VER:-unknown}"
ui_print "- Installing to $MODPATH"

set_perm "$MODPATH/activator" 0 0 0755
set_perm "$MODPATH/service.sh" 0 0 0755
set_perm "$MODPATH/uninstall.sh" 0 0 0755

ui_print "- Config: /data/system/vpnhide_config.json (managed by the app)"
ui_print "- Pick apps via the VPN Hide app → Hiding → Ports."
