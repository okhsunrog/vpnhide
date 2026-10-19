#!/system/bin/sh
# shellcheck disable=SC2034
SKIPUNZIP=0
MOD_VER="$(grep '^version=' "$MODPATH/module.prop" | cut -d= -f2)"
ui_print "- VPN Hide (kernel) ${MOD_VER:-unknown}"
ui_print "- Installing kernel module artifact only"
ui_print "- Activation is owned exclusively by the vpnhide_core module"

PERSIST_DIR="/data/adb/vpnhide_kmod"
mkdir -p "$PERSIST_DIR"
set_perm "$PERSIST_DIR" 0 0 0755
set_perm "$MODPATH/vpnhide_kmod.ko" 0 0 0644

ui_print "- Install vpnhide_core (Magisk/KSU/APatch) for boot apply"
ui_print "- Config: /data/system/vpnhide_config.json (via core)"
