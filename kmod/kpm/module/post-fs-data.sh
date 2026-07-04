#!/system/bin/sh
# Loads the vpnhide KPM via the KernelPatch runtime, early in boot (before
# apps start), and records load_status so the app can explain *why* the module
# didn't come up without guessing. Targets are applied later, in service.sh,
# once PackageManager is up (mirrors the .ko: post-fs-data loads, service
# resolves UIDs). See docs/protocol.md §7.4.
#
# Runtime split (protocol §7.4):
#   - KPatch-Next (Magisk / KSU), keyless (d05): load here, fully automatic.
#   - APatch/FolkPatch: post-fs-data records a deferred status; service.sh
#     can load/configure later through the activator's direct supercall path
#     with a saved /data/adb/vpnhide/superkey or the runtime's trusted `su`
#     supercall grant.
#
# Single-active guard (protocol §1.5): if the .ko backend is installed, do NOT
# load the KPM. They wrap the same kernel functions and co-residence freezes
# the kernel. The guard is layered in userspace, fail-safe at every step:
#   1. here (post-fs-data): defer before loading the .kpm at all;
#   2. service.sh: re-checks before configuring, and honours the activator's
#      EXIT_DEFERRED_CONFLICT (3);
#   3. the activator's kmod_backend_present() — a superset that also catches a
#      live /proc/vpnhide_ctl — gates the config-delivery path.
# There is deliberately NO kernel-side mutual exclusion: the two modules load
# in the same post-fs-data window with no ordering guarantee, so an in-kernel
# check could itself race into the freeze it means to prevent. Detection-by-
# installation (a directory check, not a load check) is ordering-independent
# and keeps the decision in userspace where it can fail safe.

MODDIR="${0%/*}"
KPM="$MODDIR/vpnhide.kpm"
STATUS_DIR="/data/adb/vpnhide_kpm"
STATUS_FILE="$STATUS_DIR/load_status"

mkdir -p "$STATUS_DIR"

# Collapse newlines/tabs so the app's key=value parser stays line-based.
sanitize() {
    printf '%s' "$1" | tr '\n\r\t' '   ' | sed 's/  */ /g'
}

# runtime, loaded(0/1), detail
write_status() {
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

# Locate the KPatch-Next CLI (`kpatch`). It ships as the KPatch-Next-Module on
# any manager (Magisk / KSU / KSU-Next — bin path confirmed on a Pixel 8 Pro);
# also accept it on PATH. APatch is handled earlier (its own supercall branch),
# not here.
find_kpatch() {
    for c in \
        kpatch \
        /data/adb/modules/KPatch-Next/bin/kpatch \
        /data/adb/modules/kpatch-next/bin/kpatch
    do
        if command -v "$c" >/dev/null 2>&1; then echo "$c"; return 0; fi
        [ -x "$c" ] && { echo "$c"; return 0; }
    done
    return 1
}

# --- single-active guard (§1.5): defer to the .ko if it's installed+enabled --
if [ -d /data/adb/modules/vpnhide_kmod ] && \
   [ ! -f /data/adb/modules/vpnhide_kmod/disable ]; then
    log -t vpnhide "kpm: .ko backend present — not loading KPM (single-active)"
    write_status conflict 0 "vpnhide_kmod present"
    exit 0
fi

if [ ! -f "$KPM" ]; then
    write_status unknown 0 "vpnhide.kpm not found at $KPM"
    exit 1
fi

# --- APatch/FolkPatch: service activator owns load/config -------------------
if [ -d /data/adb/ap ]; then
    log -t vpnhide "kpm: APatch/FolkPatch runtime — deferring load to service activator"
    write_status apatch 0 awaiting_superkey
    exit 0
fi

KPATCH="$(find_kpatch)" || {
    log -t vpnhide "kpm: kpatch CLI not found — cannot load"
    write_status unknown 0 "kpatch CLI not found"
    exit 1
}

# --- KPatch-Next (Magisk / KSU): keyless, load now --------------------------
# Keyless: the superkey argument is omitted (protocol §7.3).
LOAD_OUT="$("$KPATCH" kpm load "$KPM" 2>&1)"
if "$KPATCH" kpm list 2>/dev/null | grep -q vpnhide; then
    log -t vpnhide "kpm: loaded (kpatch-next)"
    write_status kpatch-next 1 "$LOAD_OUT"
    exit 0
fi
log -t vpnhide "kpm: load failed: $LOAD_OUT"
write_status kpatch-next 0 "$LOAD_OUT"
exit 1
