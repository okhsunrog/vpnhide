#!/system/bin/sh
# shellcheck disable=SC2034
SKIPUNZIP=0
MOD_VER="$(grep '^version=' "$MODPATH/module.prop" | cut -d= -f2)"
ui_print "- VPN Hide (Zygisk) ${MOD_VER:-unknown}"
ui_print "- Installing Zygisk native libraries only"
ui_print "- targets.txt publication is owned exclusively by vpnhide_core"

set_perm "$MODPATH/zygisk/arm64-v8a.so" 0 0 0755
[ -f "$MODPATH/zygisk/armeabi-v7a.so" ] && set_perm "$MODPATH/zygisk/armeabi-v7a.so" 0 0 0755

ui_print "- Install vpnhide_core for boot apply / Zygisk policy publish"
ui_print "- Config: /data/system/vpnhide_config.json (via core)"
