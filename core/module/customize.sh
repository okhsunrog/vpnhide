#!/system/bin/sh
mv -f "$MODPATH/bin/$ARCH/vpnhide-core" "$MODPATH/vpnhide-core"
rm -rf "${MODPATH:?}/bin"
set_perm "$MODPATH/vpnhide-core" 0 0 0755
