#!/system/bin/sh
# Early boot: create ownership/persist dirs and claim config adoption markers.
# Do NOT wait on PackageManager or netd here — that blocks boot. Apply runs in service.sh.
MODDIR="${0%/*}"
PERSIST="/data/adb/vpnhide/core"

mkdir -p "$PERSIST"
chmod 700 "$PERSIST" 2>/dev/null

# Record that core module reached post-fs-data (diagnosable if service never runs).
echo "phase=post-fs-data" >"$PERSIST/boot_phase"
# Prefer core for later legacy activators even before apply completes.
# Claiming config ownership here fences APK writers that honor the marker;
# Native/Ports markers are claimed by core on first successful apply.
if [ ! -f "$PERSIST/config_owned" ]; then
  echo "owned-by-core" >"$PERSIST/config_owned"
fi

exit 0
