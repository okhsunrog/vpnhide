#!/system/bin/sh
# Resolves package names → UIDs for kmod and lsposed at boot.
# kmod targets → /proc/vpnhide_targets
# lsposed targets → /data/system/vpnhide_uids.txt

KMOD_TARGETS="/data/adb/vpnhide_kmod/targets.txt"
LSPOSED_TARGETS="/data/adb/vpnhide_lsposed/targets.txt"
PROC_TARGETS="/proc/vpnhide_targets"
SS_UIDS_FILE="/data/system/vpnhide_uids.txt"

# Wait for the proc entry (kernel module must be loaded)
for i in 1 2 3 4 5 6 7 8 9 10; do
    [ -f "$PROC_TARGETS" ] && break
    sleep 1
done

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

if [ ! -f "$PROC_TARGETS" ]; then
    log -t vpnhide "kernel module not loaded, skipping kmod UID resolution"
fi

# Migration: if lsposed targets don't exist yet, seed from kmod targets
if [ ! -f "$LSPOSED_TARGETS" ] && [ -f "$KMOD_TARGETS" ]; then
    cp "$KMOD_TARGETS" "$LSPOSED_TARGETS"
    log -t vpnhide "migrated kmod targets to lsposed targets"
fi

# Get all packages with UIDs across every profile in one call.
# `--user all` emits comma-separated UIDs per package line for apps
# present in multiple profiles, e.g.
#   package:com.android.chrome uid:10187,1010187
# so work-profile / secondary-user installs get targeted too.
ALL_PACKAGES="$(pm list packages -U --user all 2>/dev/null)"

# resolve_uids <targets_file> — prints one UID per line to stdout.
# Splits the comma-separated UID list so every profile's copy of the
# target package ends up individually in /proc/vpnhide_targets.
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

# Resolve kmod targets → /proc/vpnhide_targets
if [ -f "$PROC_TARGETS" ] && [ -f "$KMOD_TARGETS" ]; then
    KMOD_UIDS="$(resolve_uids "$KMOD_TARGETS")"
    if [ -n "$KMOD_UIDS" ]; then
        echo "$KMOD_UIDS" > "$PROC_TARGETS"
        count="$(echo "$KMOD_UIDS" | wc -l)"
        log -t vpnhide "kmod: loaded $count target UIDs"
    else
        log -t vpnhide "kmod: no UIDs resolved"
    fi
fi

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
        log -t vpnhide "lsposed: wrote $count UIDs to $SS_UIDS_FILE"
    else
        echo > "$SS_UIDS_FILE"
        chmod 644 "$SS_UIDS_FILE"
        log -t vpnhide "lsposed: no UIDs resolved"
    fi
fi
