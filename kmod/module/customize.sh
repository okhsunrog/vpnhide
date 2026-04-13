#!/system/bin/sh
SKIPUNZIP=0
ui_print "- VPN Hide (kernel) v0.1.0"
ui_print "- Installing kernel module to $MODPATH"

# Persistent config directory (survives module updates)
PERSIST_DIR="/data/adb/vpnhide_kmod"
PERSIST_TARGETS="$PERSIST_DIR/targets.txt"

mkdir -p "$PERSIST_DIR"
set_perm "$PERSIST_DIR" 0 0 0755

# Seed empty targets on fresh install
if [ ! -f "$PERSIST_TARGETS" ]; then
    cat > "$PERSIST_TARGETS" <<'EOF'
# vpnhide-kmod target apps
# One package name per line. Lines starting with '#' are comments.
# Managed via the VPN Hide app.
EOF
fi
set_perm "$PERSIST_TARGETS" 0 0 0644

set_perm "$MODPATH/vpnhide_kmod.ko" 0 0 0644

ui_print "- Targets: $PERSIST_TARGETS (preserved across updates)"
ui_print "- Pick target apps via the VPN Hide app."
