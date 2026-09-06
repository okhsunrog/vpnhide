#!/system/bin/sh
# shellcheck disable=SC2034
SKIPUNZIP=0
MOD_VER="$(grep '^version=' "$MODPATH/module.prop" | cut -d= -f2)"
ui_print "- VPN Hide (Built-in kernel) ${MOD_VER:-unknown}"
ui_print "- Userspace companion for the in-tree CONFIG_VPNHIDE driver"
ui_print "- No kernel module is flashed: the driver is compiled into the kernel"

# Status directory (survives module updates)
PERSIST_DIR="/data/adb/vpnhide_builtin"

mkdir -p "$PERSIST_DIR"
set_perm "$PERSIST_DIR" 0 0 0755

set_perm "$MODPATH/activator" 0 0 0755
set_perm "$MODPATH/post-fs-data.sh" 0 0 0755
set_perm "$MODPATH/service.sh" 0 0 0755
set_perm "$MODPATH/uninstall.sh" 0 0 0755

# The driver must actually be present. Warn (don't abort) if the control node is
# missing — the user may be flashing this before booting the CONFIG_VPNHIDE kernel.
if [ ! -e /proc/vpnhide_ctl ]; then
  ui_print "! /proc/vpnhide_ctl not found — this kernel may not be built with CONFIG_VPNHIDE"
  ui_print "! The backend stays inactive until you boot a kernel that has the driver built in"
fi

ui_print "- Config: /data/system/vpnhide_config.json (managed by the app)"
ui_print "- Pick target apps via the VPN Hide app."
