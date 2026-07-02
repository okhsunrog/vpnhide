#!/usr/bin/env bash
# Wipe all VPN Hide data from a connected device for clean install testing.
# Usage: ./scripts/clean-device.sh
#
# The path lists below mirror FULL_RESET_FILES / FULL_RESET_DIRS in
# lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/FullReset.kt — keep them in
# sync. They cover the canonical config, the derived runtime wires, the root-
# owned APatch superkey under /data/adb/vpnhide, and the KPM/ports state, none of
# which an APK uninstall touches.
set -euo pipefail

# Files under /data/system (rm -f).
RESET_FILES=(
  /data/system/vpnhide_config.json
  /data/system/vpnhide_uids.txt
  /data/system/vpnhide_hidden_pkgs.txt
  /data/system/vpnhide_observer_uids.txt
  /data/system/vpnhide_lsposed_state
  /data/system/vpnhide_hook_active
)

# Directories under /data/adb (rm -rf — covers targets.txt, load_status,
# superkey, etc. inside). /data/adb/vpnhide holds the superkey, so it goes first.
RESET_DIRS=(
  /data/adb/vpnhide
  /data/adb/vpnhide_kmod
  /data/adb/vpnhide_kpm
  /data/adb/vpnhide_zygisk
  /data/adb/vpnhide_lsposed
  /data/adb/vpnhide_ports
)

echo "Uninstalling app..."
adb shell pm uninstall dev.okhsunrog.vpnhide 2>/dev/null || true

echo "Removing persistent data (config, superkey, KPM/ports/runtime state)..."
adb shell su -c "rm -f ${RESET_FILES[*]}" 2>/dev/null || true
adb shell su -c "rm -rf ${RESET_DIRS[*]}" 2>/dev/null || true

echo "Removing app data dir..."
adb shell su -c "rm -rf /data/user/0/dev.okhsunrog.vpnhide" 2>/dev/null || true

echo "Done. Reboot recommended if modules were installed."
