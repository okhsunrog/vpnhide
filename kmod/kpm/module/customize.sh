#!/system/bin/sh
SKIPUNZIP=0
MOD_VER="$(grep '^version=' "$MODPATH/module.prop" | cut -d= -f2)"
ui_print "- VPN Hide (KPM) ${MOD_VER:-unknown}"
ui_print "- Installing KernelPatch module to $MODPATH"

# Status directory (survives module updates — KSU/Magisk wipe the module dir on
# update, but never touch /data/adb/vpnhide_kpm/).
PERSIST_DIR="/data/adb/vpnhide_kpm"

mkdir -p "$PERSIST_DIR"
set_perm "$PERSIST_DIR" 0 0 0755

set_perm "$MODPATH/activator" 0 0 0755
set_perm "$MODPATH/post-fs-data.sh" 0 0 0755
set_perm "$MODPATH/service.sh" 0 0 0755
set_perm "$MODPATH/vpnhide.kpm" 0 0 0644
set_perm "$MODPATH/uninstall.sh" 0 0 0755

# Single-active warning (protocol §1.5): the .ko and KPM wrap the SAME kernel
# functions; running both freezes the device. The boot loader refuses to load
# the KPM while the .ko module is present AND enabled (post-fs-data.sh /
# service.sh check the same `disable` flag), so warn here in exactly that case —
# a vpnhide_kmod that is merely disabled is fine and lets the KPM load.
if [ -d /data/adb/modules/vpnhide_kmod ] &&
    [ ! -f /data/adb/modules/vpnhide_kmod/disable ]; then
    ui_print "! WARNING: the .ko backend (vpnhide_kmod) is installed and enabled."
    ui_print "! Only one native backend may be active — the KPM will NOT load"
    ui_print "! until you disable (or uninstall) vpnhide_kmod."
fi

ui_print "- Config: /data/system/vpnhide_config.json (managed by the app)"
ui_print "- Pick target apps via the VPN Hide app."
