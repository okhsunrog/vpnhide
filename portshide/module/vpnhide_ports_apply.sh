#!/system/bin/sh
# Reads /data/adb/vpnhide_ports/observers.txt (one package name per line)
# and installs iptables REJECT rules that block each observer's UID from
# reaching any port on 127.0.0.1 / ::1. Used for hiding locally-bound
# VPN/proxy daemons from apps that probe via connect(127.0.0.1, PORT).
#
# Package names are resolved to UIDs at apply time so reinstalls (which
# rotate an app's UID) are picked up on the next boot or Save — a stale
# UID never sticks. Same pattern as kmod's service.sh.
#
# Callable from service.sh at boot and from the VPN Hide app via su.
# Idempotent: flushes our chain and rebuilds atomically via
# iptables-restore --noflush. Jump from OUTPUT is added only if missing.
#
# v4 and v6 restores are independent operations — if v6 fails mid-way,
# v4 rules are already live. The exit code reflects both so the caller
# can surface partial-apply, but the chain state is not transactional
# across families.

OBSERVERS_FILE="/data/adb/vpnhide_ports/observers.txt"
CHAIN4="vpnhide_out"
CHAIN6="vpnhide_out6"

# Wait for PackageManager so pm list packages -U works. Relevant at boot
# (service.sh); at Save-time pm is already up, the first iteration wins.
pm_ready=0
for i in $(seq 1 30); do
    if pm list packages >/dev/null 2>&1; then
        pm_ready=1
        break
    fi
    sleep 1
done

if [ "$pm_ready" != 1 ]; then
    log -t vpnhide_ports "pm never became ready after 30s; skipping apply"
    exit 1
fi

ALL_PACKAGES="$(pm list packages -U 2>/dev/null)"

# Read pkg names from observers.txt, resolve to UIDs, build newline-
# separated UID list. Skip comments, blanks, unknown packages.
UIDS=""
if [ -f "$OBSERVERS_FILE" ]; then
    while IFS= read -r line || [ -n "$line" ]; do
        pkg="$(echo "$line" | tr -d '[:space:]')"
        [ -z "$pkg" ] && continue
        case "$pkg" in \#*) continue ;; esac
        # Exact match on $1 — grep would treat pkg dots as regex wildcards
        # and could mis-resolve e.g. "com.x.y" to "comXxXy" if such a package
        # existed. awk compares fields literally.
        uid="$(echo "$ALL_PACKAGES" | awk -v p="package:${pkg}" '$1 == p { sub(/uid:/, "", $2); print $2; exit }')"
        [ -z "$uid" ] && continue
        case "$uid" in *[!0-9]*) continue ;; esac
        # System UID guard — don't let user accidentally block localhost
        # for installd / system_server / bluetooth / etc.
        [ "$uid" -lt 10000 ] && continue
        if [ -z "$UIDS" ]; then UIDS="$uid"; else UIDS="${UIDS}
${uid}"; fi
    done < "$OBSERVERS_FILE"
fi

# Build an iptables-restore ruleset for a given chain + loopback destination.
# UDP reject differs by family: `icmp-port-unreachable` on IPv4,
# `icmp6-port-unreachable` on IPv6.
build_ruleset() {
    chain="$1"
    loopback="$2"
    udp_reject="$3"
    echo "*filter"
    echo ":${chain} - [0:0]"
    if [ -n "$UIDS" ]; then
        echo "$UIDS" | while IFS= read -r uid; do
            [ -z "$uid" ] && continue
            echo "-A ${chain} -m owner --uid-owner ${uid} -d ${loopback} -p tcp -j REJECT --reject-with tcp-reset"
            echo "-A ${chain} -m owner --uid-owner ${uid} -d ${loopback} -p udp -j REJECT --reject-with ${udp_reject}"
        done
    fi
    echo "-A ${chain} -j RETURN"
    echo "COMMIT"
}

# Ensure our chains exist before restore tries to replace them.
iptables  -N "$CHAIN4" 2>/dev/null || true
ip6tables -N "$CHAIN6" 2>/dev/null || true

build_ruleset "$CHAIN4" "127.0.0.1" "icmp-port-unreachable" | iptables-restore --noflush
rc4=$?
build_ruleset "$CHAIN6" "::1" "icmp6-port-unreachable" | ip6tables-restore --noflush
rc6=$?

# Ensure OUTPUT jumps into our chain (exactly once).
iptables  -C OUTPUT -j "$CHAIN4" >/dev/null 2>&1 || iptables  -I OUTPUT -j "$CHAIN4"
ip6tables -C OUTPUT -j "$CHAIN6" >/dev/null 2>&1 || ip6tables -I OUTPUT -j "$CHAIN6"

count=0
[ -n "$UIDS" ] && count=$(echo "$UIDS" | wc -l)
log -t vpnhide_ports "applied rules: ${count} observer(s), rc4=${rc4} rc6=${rc6}"

[ "$rc4" = 0 ] && [ "$rc6" = 0 ]
