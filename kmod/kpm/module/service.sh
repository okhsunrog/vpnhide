#!/system/bin/sh
# Boot-time config delivery for the KPM backend. The activator owns JSON ->
# protocol projection and the KernelPatch ctl0 transport.

MODDIR="${0%/*}"
ACTIVATOR="$MODDIR/activator"
KPM="$MODDIR/vpnhide.kpm"
STATUS_DIR="/data/adb/vpnhide_kpm"
STATUS_FILE="$STATUS_DIR/load_status"
SUPERKEY_FILE="/data/adb/vpnhide/superkey"

sanitize() {
    printf '%s' "$1" | tr '\n' ' ' | cut -c 1-240
}

write_status() {
    mkdir -p "$STATUS_DIR"
    {
        printf 'timestamp=%s\n' "$(date +%s 2>/dev/null)"
        printf 'boot_id=%s\n' "$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)"
        printf 'uname_r=%s\n' "$(uname -r 2>/dev/null)"
        printf 'runtime=%s\n' "$1"
        printf 'loaded=%s\n' "$2"
        printf 'detail=%s\n' "$(sanitize "$3")"
    } > "$STATUS_FILE.tmp" && mv "$STATUS_FILE.tmp" "$STATUS_FILE"
    chmod 0644 "$STATUS_FILE" 2>/dev/null
}

apply_at_boot() {
    # Single-active guard (§1.5), checked before the APatch superkey branch so
    # a co-installed .ko isn't masked as `awaiting_superkey`. Cheap, fail-safe,
    # ordering-independent floor; the activator re-checks (a superset, incl. a
    # live /proc/vpnhide_ctl) below and exits 3 if it still sees the .ko.
    if [ -d /data/adb/modules/vpnhide_kmod ] && \
       [ ! -f /data/adb/modules/vpnhide_kmod/disable ]; then
        log -t vpnhide "kpm: .ko backend present — not configuring KPM (single-active)"
        write_status conflict 0 "vpnhide_kmod present"
        return 0
    fi
    if [ ! -x "$ACTIVATOR" ]; then
        log -t vpnhide "kpm: activator missing at $ACTIVATOR"
        write_status activator 0 "activator missing at $ACTIVATOR"
        return 1
    fi
    if [ ! -f "$KPM" ]; then
        log -t vpnhide "kpm: KPM missing at $KPM"
        write_status activator 0 "vpnhide.kpm missing at $KPM"
        return 1
    fi

    out="$("$ACTIVATOR" --boot-wait 2>&1)"
    rc=$?
    case "$rc" in
        0)
            log -t vpnhide "kpm: activator finished boot config"
            write_status activator 1 configured
            ;;
        3)
            # EXIT_DEFERRED_CONFLICT: the activator found the .ko present (e.g.
            # it loaded during the PackageManager wait) and stood down. Record
            # the truthful conflict status rather than a false `configured`.
            log -t vpnhide "kpm: activator deferred to .ko (single-active)"
            write_status conflict 0 "vpnhide_kmod present"
            ;;
        *)
            log -t vpnhide "kpm: activator failed rc=$rc"
            if [ -d /data/adb/ap ] && [ ! -s "$SUPERKEY_FILE" ] && \
               printf '%s' "$out" | grep -q "APatch/FolkPatch KPM requires"; then
                write_status apatch 0 awaiting_superkey
                return 0
            fi
            write_status activator 0 "rc=$rc $out"
            return "$rc"
            ;;
    esac
}

apply_at_boot &
exit 0
