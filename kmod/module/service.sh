#!/system/bin/sh
# Resolves package names → UIDs for kmod and lsposed at boot.
# kmod targets → /proc/vpnhide_targets
# lsposed targets → /data/system/vpnhide_uids.txt

KMOD_TARGETS="/data/adb/vpnhide_kmod/targets.txt"
KMOD_DIRECT_TARGETS="/data/adb/vpnhide_kmod/direct_targets.txt"
LSPOSED_TARGETS="/data/adb/vpnhide_lsposed/targets.txt"
PROC_TARGETS="/proc/vpnhide_targets"
PROC_DIRECT_TARGETS="/proc/vpnhide_direct_targets"
SS_UIDS_FILE="/data/system/vpnhide_uids.txt"

# Wait for the proc entries (kernel module must be loaded)
for i in 1 2 3 4 5 6 7 8 9 10; do
    [ -f "$PROC_TARGETS" ] && [ -f "$PROC_DIRECT_TARGETS" ] && break
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
        # Literal match on $1 — grep would treat dots in `pkg` as regex
        # wildcards (e.g. `com.x.y` matching `comXxXy` if such a package
        # ever existed). awk's `$1 == p` compares fields literally.
        uid_csv="$(echo "$ALL_PACKAGES" | awk -v p="package:${pkg}" '$1 == p { sub(/uid:/, "", $2); print $2; exit }')"
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

# Resolve kmod direct bypass targets → /proc/vpnhide_direct_targets
if [ -f "$PROC_DIRECT_TARGETS" ] && [ -f "$KMOD_DIRECT_TARGETS" ]; then
    DIRECT_UIDS="$(resolve_uids "$KMOD_DIRECT_TARGETS")"
    if [ -n "$DIRECT_UIDS" ]; then
        echo "$DIRECT_UIDS" > "$PROC_DIRECT_TARGETS"
        count="$(echo "$DIRECT_UIDS" | wc -l)"
        log -t vpnhide "kmod-direct: loaded $count target UIDs"
    else
        log -t vpnhide "kmod-direct: no UIDs resolved"
    fi
fi

# Background monitoring: find active physical interface and tell the kernel
(
    TRIGGER="/tmp/vpnhide_route_trigger"
    rm -f "$TRIGGER"

    (
        while true; do
            ip monitor link address route | while read -r line; do
                touch "$TRIGGER"
            done
            sleep 1
        done
    ) &

    last_idx=""
    heartbeat=0

    while true; do
        if [ -f "$TRIGGER" ] || [ -z "$last_idx" ] || [ "$heartbeat" -ge 5 ]; then
            rm -f "$TRIGGER"
            heartbeat=0
            
            sleep 0.5
            
            phys_iface=$(ip route show table all | grep "default via" | grep -vE "tun|wg|dummy|p2p" | head -n1 | awk '{print $5}')
            
            if [ -n "$phys_iface" ]; then
                phys_ifindex=$(cat "/sys/class/net/$phys_iface/ifindex" 2>/dev/null)
                
                if [ -n "$phys_ifindex" ]; then
                    echo "$phys_ifindex" > /proc/vpnhide_phys_ifindex

                    if [ "$phys_ifindex" != "$last_idx" ]; then
                        log -t vpnhide "routing: active physical interface changed to $phys_iface ($phys_ifindex)"
                        last_idx="$phys_ifindex"
                    fi
                fi
            fi
        fi
        
        sleep 1
        heartbeat=$((heartbeat + 1))
    done
) &

log -t vpnhide "service.sh background monitoring started"

# Resolve lsposed targets → /data/system/vpnhide_uids.txt
# Create persist dir if needed (for first-time installs)
mkdir -p /data/adb/vpnhide_lsposed 2>/dev/null
# Mode 0640 + group=system: system_server (UID 1000, in group `system`)
# reads via the group bit; untrusted apps fall to "other" and get EACCES.
# Default 0644 was a fingerprint vector — `/data/system/` itself is mode
# 0775 traversable by untrusted, so any o+r file is enumerable + readable.
if [ -f "$LSPOSED_TARGETS" ]; then
    LSPOSED_UIDS="$(resolve_uids "$LSPOSED_TARGETS")"
    if [ -n "$LSPOSED_UIDS" ]; then
        echo "$LSPOSED_UIDS" > "$SS_UIDS_FILE"
        chmod 640 "$SS_UIDS_FILE"
        chown root:system "$SS_UIDS_FILE"
        chcon u:object_r:system_data_file:s0 "$SS_UIDS_FILE" 2>/dev/null
        count="$(echo "$LSPOSED_UIDS" | wc -l)"
        log -t vpnhide "lsposed: wrote $count UIDs to $SS_UIDS_FILE"
    else
        echo > "$SS_UIDS_FILE"
        chmod 640 "$SS_UIDS_FILE"
        chown root:system "$SS_UIDS_FILE"
        log -t vpnhide "lsposed: no UIDs resolved"
    fi
fi

# Migrate pre-PR files written by older versions with mode 0644: any
# vpnhide_*.txt the lsposed app may have left in /data/system/. Touch
# only files that already exist; don't create new ones here.
for f in "$SS_UIDS_FILE" \
         /data/system/vpnhide_hidden_pkgs.txt \
         /data/system/vpnhide_observer_uids.txt; do
    if [ -f "$f" ]; then
        chmod 640 "$f"
        chown root:system "$f"
        chcon u:object_r:system_data_file:s0 "$f" 2>/dev/null
    fi
done

# Re-seed /proc/vpnhide_debug from the persistent toggle flag the app
# maintains. /proc/vpnhide_debug is per-boot in-kernel state — without
# this re-seed, a user with "Debug logging" turned ON would silently
# get OFF after every reboot until they re-opened the app (which is
# what triggers the in-app re-propagation in MainActivity.onCreate).
# Same model as targets.txt → /proc/vpnhide_targets above: persistent
# file is canonical, /proc gets reseeded each boot.
SS_DEBUG_LOGGING="/data/system/vpnhide_debug_logging"
if [ -e /proc/vpnhide_debug ] && [ -f "$SS_DEBUG_LOGGING" ]; then
    cat "$SS_DEBUG_LOGGING" > /proc/vpnhide_debug 2>/dev/null
fi
