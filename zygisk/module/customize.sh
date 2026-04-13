#!/system/bin/sh
# Installer hook run by KernelSU/Magisk. Called after the module files
# have been extracted to $MODPATH. Anything we write to $MODPATH persists
# into /data/adb/modules/<id>/ after the installer finishes.

SKIPUNZIP=0
ui_print "- VPN Hide (Zygisk native) v0.1.0"
ui_print "- Installing to $MODPATH"

# Make the native library readable/executable by zygote
set_perm "$MODPATH/zygisk/arm64-v8a.so" 0 0 0755

# ----------------------------------------------------------------------
#  Persistent state directory
# ----------------------------------------------------------------------
# We deliberately store the user's targets list OUTSIDE the module
# directory. KSU/Magisk wipes the entire module dir on update, so anything
# under /data/adb/modules/vpnhide_zygisk/ would be lost every time the
# user installs a new build of the module. /data/adb/vpnhide_zygisk/ is
# never touched by the installer and persists across module updates and
# reboots.
PERSIST_DIR="/data/adb/vpnhide_zygisk"
PERSIST_TARGETS="$PERSIST_DIR/targets.txt"
LEGACY_TARGETS="/data/adb/modules/vpnhide_zygisk/targets.txt"

mkdir -p "$PERSIST_DIR"
set_perm "$PERSIST_DIR" 0 0 0755

# One-shot migration: when the new persistent file does not exist yet
# but the legacy in-module file does, copy the user's existing list over
# before the staged install dir replaces /data/adb/modules/vpnhide_zygisk/.
# This works because customize.sh runs while the OLD module directory is
# still on disk — KSU/Magisk only swaps the staged dir into place after
# this script returns successfully.
if [ ! -f "$PERSIST_TARGETS" ] && [ -f "$LEGACY_TARGETS" ]; then
    cp "$LEGACY_TARGETS" "$PERSIST_TARGETS"
    ui_print "- Migrated existing targets list from previous install"
fi

# Seed an empty file on a truly fresh install.
if [ ! -f "$PERSIST_TARGETS" ]; then
    cat > "$PERSIST_TARGETS" <<'EOF'
# vpnhide-zygisk target allowlist
# One package name per line. Lines starting with '#' are comments.
# Managed via the VPN Hide app.
EOF
fi
set_perm "$PERSIST_TARGETS" 0 0 0644

ui_print "- Targets list: $PERSIST_TARGETS (preserved across updates)"
ui_print "- Pick target apps via the VPN Hide app."
