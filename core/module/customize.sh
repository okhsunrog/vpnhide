#!/system/bin/sh
# shellcheck disable=SC2034
SKIPUNZIP=0
MOD_VER="$(grep '^version=' "$MODPATH/module.prop" | cut -d= -f2)"
ui_print "- VPN Hide (Core) ${MOD_VER:-unknown}"
ui_print "- Common Magisk-format module (Magisk / KSU / APatch)"

PERSIST_DIR="/data/adb/vpnhide/core"
mkdir -p "$PERSIST_DIR"
set_perm "$PERSIST_DIR" 0 0 0700

# Keep only the binary matching this device's CPU (zip may ship arm64 + armv7).
VH_ARCH="$ARCH"
if [ -z "$VH_ARCH" ]; then
  case "$(uname -m)" in
    aarch64 | arm64) VH_ARCH=arm64 ;;
    arm*) VH_ARCH=arm ;;
    *) VH_ARCH=arm64 ;;
  esac
fi
if [ -f "$MODPATH/vpnhide-core.arm64" ] || [ -f "$MODPATH/vpnhide-core.armv7" ]; then
  if [ "$VH_ARCH" = "arm" ]; then
    mv -f "$MODPATH/vpnhide-core.armv7" "$MODPATH/vpnhide-core"
  else
    mv -f "$MODPATH/vpnhide-core.arm64" "$MODPATH/vpnhide-core"
  fi
  rm -f "$MODPATH/vpnhide-core.arm64" "$MODPATH/vpnhide-core.armv7"
fi

if [ ! -f "$MODPATH/vpnhide-core" ]; then
  ui_print "! vpnhide-core binary missing — flash a release zip, not an empty scaffold"
fi

set_perm "$MODPATH/vpnhide-core" 0 0 0755
set_perm "$MODPATH/post-fs-data.sh" 0 0 0755
set_perm "$MODPATH/service.sh" 0 0 0755
set_perm "$MODPATH/uninstall.sh" 0 0 0755
[ -f "$MODPATH/legacy-prefer-core.sh" ] && set_perm "$MODPATH/legacy-prefer-core.sh" 0 0 0644

ui_print "- Persist: $PERSIST_DIR (journal, markers, boot_status, socket)"
ui_print "- Config:  /data/system/vpnhide_config.json (LSPosed-compatible schema)"
ui_print "- Reboot required. Core applies policy without the APK."
ui_print "- APatch SuperKey / secrets are not touched by this module."
