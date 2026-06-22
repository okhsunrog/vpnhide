#!/system/bin/sh
# Copies zygisk targets to module dir and resolves lsposed targets at boot.
# zygisk targets → module dir (read by Zygisk via get_module_dir() fd)
# lsposed targets → /data/system/vpnhide_uids.txt

ZYGISK_TARGETS="/data/adb/vpnhide_zygisk/targets.txt"
LSPOSED_TARGETS="/data/adb/vpnhide_lsposed/targets.txt"
MODULE_DIR="${0%/*}"
SS_UIDS_FILE="/data/system/vpnhide_uids.txt"

# Copy zygisk targets to module dir so Zygisk can read via get_module_dir() fd.
if [ -f "$ZYGISK_TARGETS" ]; then
    cp "$ZYGISK_TARGETS" "$MODULE_DIR/targets.txt" 2>/dev/null
fi

# Wait until PackageManager has actually indexed user-installed apps.
# `pm list packages` starts responding very early in boot but returns
# only system packages for several more seconds — if we resolve during
# that window, `dev.okhsunrog.vpnhide` (and any other user-installed
# target) silently drops from the UID file and the LSPosed hook caches
# an empty target set for the rest of the session. Gate on our own
# package being visible, with a 60s budget.
for i in $(seq 1 60); do
    if pm list packages -U 2>/dev/null | grep -q "^package:dev.okhsunrog.vpnhide "; then
        break
    fi
    sleep 1
done

# Migration: if lsposed targets don't exist yet, seed from zygisk targets
if [ ! -f "$LSPOSED_TARGETS" ] && [ -f "$ZYGISK_TARGETS" ]; then
    mkdir -p /data/adb/vpnhide_lsposed 2>/dev/null
    cp "$ZYGISK_TARGETS" "$LSPOSED_TARGETS"
    log -t vpnhide "migrated zygisk targets to lsposed targets"
fi

# Get all packages with UIDs across every profile in one call.
# `--user all` emits comma-separated UIDs per package for apps present
# in multiple profiles (e.g. work profile) so each profile's copy ends
# up in vpnhide_uids.txt.
ALL_PACKAGES="$(pm list packages -U --user all 2>/dev/null)"

# resolve_uids <targets_file> — prints one UID per line to stdout,
# splitting comma-separated UIDs so every profile gets its own entry.
resolve_uids() {
    local targets_file="$1"
    [ -f "$targets_file" ] || return
    local uids=""
    while IFS= read -r line || [ -n "$line" ]; do
        pkg="$(echo "$line" | tr -d '[:space:]')"
        [ -z "$pkg" ] && continue
        case "$pkg" in \#*) continue ;; esac
        uid_csv="$(echo "$ALL_PACKAGES" | awk -v p="package:${pkg}" '
            $1 == p {
                sub(/uid:/, "", $2)
                n = split($2, ids, ",")
                for (i = 1; i <= n; i++) print ids[i]
            }')"
        if [ -n "$uid_csv" ]; then
            expanded="$(echo "$uid_csv" | tr ',' '\n')"
            if [ -z "$uids" ]; then uids="$expanded"; else uids="${uids}
${expanded}"; fi
        else
            log -t vpnhide "package not found: $pkg"
        fi
    done < "$targets_file"
    [ -n "$uids" ] && echo "$uids"
}

# Resolve lsposed targets → /data/system/vpnhide_uids.txt
# Create persist dir if needed (for first-time installs)
mkdir -p /data/adb/vpnhide_lsposed 2>/dev/null
if [ -f "$LSPOSED_TARGETS" ]; then
    LSPOSED_UIDS="$(resolve_uids "$LSPOSED_TARGETS")"
    if [ -n "$LSPOSED_UIDS" ]; then
        echo "$LSPOSED_UIDS" > "$SS_UIDS_FILE"
        chmod 644 "$SS_UIDS_FILE"
        chcon u:object_r:system_data_file:s0 "$SS_UIDS_FILE" 2>/dev/null
        count="$(echo "$LSPOSED_UIDS" | wc -l)"
        log -t vpnhide "zygisk: wrote $count lsposed UIDs to $SS_UIDS_FILE"
    else
        echo > "$SS_UIDS_FILE"
        chmod 644 "$SS_UIDS_FILE"
        log -t vpnhide "zygisk: no lsposed UIDs resolved"
    fi
fi
