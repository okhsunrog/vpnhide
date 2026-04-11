#!/system/bin/sh
# Resolves package names → UIDs and writes to /data/system/vpnhide_uids.txt
# for the LSPosed system_server hooks. Same contract as kmod's service.sh.

PERSIST_DIR="/data/adb/vpnhide_zygisk"
TARGETS_FILE="$PERSIST_DIR/targets.txt"
SS_UIDS_FILE="/data/system/vpnhide_uids.txt"

# Wait for PackageManager to be ready
for i in $(seq 1 30); do
    pm list packages >/dev/null 2>&1 && break
    sleep 1
done

if [ ! -f "$TARGETS_FILE" ]; then
    exit 0
fi

# Get all packages with UIDs in one call
ALL_PACKAGES="$(pm list packages -U 2>/dev/null)"

# Resolve each target package name to its UID
UIDS=""
while IFS= read -r line || [ -n "$line" ]; do
    pkg="$(echo "$line" | tr -d '[:space:]')"
    [ -z "$pkg" ] && continue
    case "$pkg" in \#*) continue ;; esac

    uid="$(echo "$ALL_PACKAGES" | grep "^package:${pkg} " | sed 's/.*uid://')"
    if [ -n "$uid" ]; then
        if [ -z "$UIDS" ]; then
            UIDS="$uid"
        else
            UIDS="${UIDS}
${uid}"
        fi
    else
        log -t vpnhide "package not found: $pkg"
    fi
done < "$TARGETS_FILE"

if [ -n "$UIDS" ]; then
    echo "$UIDS" > "$SS_UIDS_FILE"
    chmod 644 "$SS_UIDS_FILE"
    chcon u:object_r:system_data_file:s0 "$SS_UIDS_FILE" 2>/dev/null
    count="$(echo "$UIDS" | wc -l)"
    log -t vpnhide "zygisk: wrote $count UIDs to $SS_UIDS_FILE for system_server"
else
    log -t vpnhide "zygisk: no UIDs resolved from targets.txt"
fi
