#!/system/bin/sh
# Early boot: create ownership/persist dirs and claim config adoption markers.
# Do NOT wait on PackageManager or netd here — that blocks boot. Apply runs in service.sh.
PERSIST="/data/adb/vpnhide/core"

mkdir -p "$PERSIST"
chmod 700 "$PERSIST" 2>/dev/null

# Record that core module reached post-fs-data (diagnosable if service never runs).
echo "phase=post-fs-data" >"$PERSIST/boot_phase"
# Core-internal adoption marker (diagnostics only; core is the sole writer by
# construction). Native/Ports markers are claimed by core on first successful apply.
if [ ! -f "$PERSIST/config_owned" ]; then
  echo "owned-by-core" >"$PERSIST/config_owned"
fi

exit 0
