#!/system/bin/sh
# Applies iptables rules from canonical JSON at boot time. iptables rules are
# in-memory, so this restores them after reboot.

MODDIR="${0%/*}"
ACTIVATOR="$MODDIR/activator"
STATUS_DIR="/data/adb/vpnhide_ports"
STATUS_FILE="$STATUS_DIR/load_status"
LOG_FILE="$STATUS_DIR/load_log"

sanitize() {
    printf '%s' "$1" | tr '\n\r\t' '   ' | sed 's/  */ /g' | cut -c 1-240
}

write_status() {
    mkdir -p "$STATUS_DIR"
    {
        printf 'timestamp=%s\n' "$(date +%s 2>/dev/null)"
        printf 'boot_id=%s\n' "$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)"
        printf 'uname_r=%s\n' "$(uname -r 2>/dev/null)"
        printf 'runtime=ports\n'
        printf 'source=%s\n' "$1"
        printf 'loaded=%s\n' "$2"
        printf 'target_count=%s\n' "$3"
        printf 'detail=%s\n' "$(sanitize "$4")"
    } > "$STATUS_FILE.tmp" && mv "$STATUS_FILE.tmp" "$STATUS_FILE"
    chmod 0644 "$STATUS_FILE" 2>/dev/null
}

write_log() {
    mkdir -p "$STATUS_DIR"
    printf '%s\n' "$1" > "$LOG_FILE.tmp" && mv "$LOG_FILE.tmp" "$LOG_FILE"
    chmod 0644 "$LOG_FILE" 2>/dev/null
}

run_activator() {
    LABEL="$1"
    OUT="$("$ACTIVATOR" --boot-wait 2>&1)"
    RC=$?
    if [ "$RC" = 0 ]; then
        log -t vpnhide_ports "$LABEL"
        return 0
    fi
    write_status boot 0 unknown "rc=$RC $OUT"
    write_log "$OUT"
    log -t vpnhide_ports "activator failed rc=$RC"
    return "$RC"
}

apply_when_ready() {
    if [ ! -x "$ACTIVATOR" ]; then
        log -t vpnhide_ports "activator missing at $ACTIVATOR"
        write_status boot 0 unknown "activator missing at $ACTIVATOR"
        write_log "activator missing at $ACTIVATOR"
        return 1
    fi

    # Wait for netd to finish its own iptables setup so our rules survive
    # netd's initial chain rebuild. Check for the bw_OUTPUT chain as a signal
    # that netd has populated its baseline rules.
    for i in $(seq 1 60); do
        iptables -L bw_OUTPUT -n >/dev/null 2>&1 && break
        sleep 1
    done

    run_activator "applied iptables rules at boot"

    # Re-apply once more 30 s later. On slow boots netd has been observed to
    # flush/rebuild its own chains AFTER bw_OUTPUT first appears, which
    # would wipe ours. The activator is idempotent — chains are created
    # with `-N ... 2>/dev/null || true` and rebuilt atomically via
    # `iptables-restore --noflush` — so a second pass is harmless when
    # nothing was wiped and self-healing when it was.
    sleep 30
    run_activator "re-applied iptables rules (T+30s safety pass)"
}

apply_when_ready &
exit 0
