# Copy LSPosed's module scope database (+ wal/shm) into $VPNHIDE_OUT_DIR for the app's
# read-only inspection. Core hands ownership of the copies to the app.
SRC=/data/adb/lspd/config/modules_config.db
[ -n "$VPNHIDE_OUT_DIR" ] || exit 1
mkdir -p "$VPNHIDE_OUT_DIR" || exit 1
cat "$SRC" > "$VPNHIDE_OUT_DIR/modules_config.db" || exit 1
cat "$SRC-wal" > "$VPNHIDE_OUT_DIR/modules_config.db-wal" 2>/dev/null || rm -f "$VPNHIDE_OUT_DIR/modules_config.db-wal"
cat "$SRC-shm" > "$VPNHIDE_OUT_DIR/modules_config.db-shm" 2>/dev/null || rm -f "$VPNHIDE_OUT_DIR/modules_config.db-shm"
ls -l "$VPNHIDE_OUT_DIR" 2>/dev/null || true
