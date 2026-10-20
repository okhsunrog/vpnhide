#!/usr/bin/env bash
# Wipe all VPN Hide data from a connected device for clean install testing.
# Usage: ./scripts/clean-device.sh
#
# A host-side developer tool (it uses `adb shell su`; the app itself never does).
# The path lists below are a superset of core's FullReset effect (RESET_FILES /
# RESET_DIRS / LEGACY_FILES in crates/core-daemon/src/effects.rs) — keep them in
# sync. They cover the canonical config, the derived runtime wires, the root-
# owned APatch superkey and core's journal under /data/adb/vpnhide, and the
# backend state dirs, none of which an APK uninstall touches. Root-manager
# modules (vpnhide_core and the backends) are not removed: uninstall them in the
# manager.
set -euo pipefail

# Files under /data/system (rm -f).
RESET_FILES=(
  /data/system/vpnhide_config.json
  /data/system/vpnhide_lsposed_state
  /data/system/vpnhide_hook_active
  /data/system/vpnhide_hidden_pkgs.txt
  /data/system/vpnhide_observer_uids.txt
  /data/system/vpnhide_uids.txt
)

# Directories under /data/adb (rm -rf — covers load status, logs, the ctl lock,
# and the superkey). /data/adb/vpnhide holds the superkey, so it goes first.
RESET_DIRS=(
  /data/adb/vpnhide
  /data/adb/vpnhide_kmod
  /data/adb/vpnhide_builtin
  /data/adb/vpnhide_kpm
  /data/adb/vpnhide_ports
  /data/adb/vpnhide_zygisk
  /data/adb/vpnhide_lsposed
)

echo "Uninstalling app..."
adb shell pm uninstall dev.okhsunrog.vpnhide 2>/dev/null || true

echo "Removing persistent data (config, superkey, KPM/ports/runtime state)..."
adb shell su -c "rm -f ${RESET_FILES[*]}" 2>/dev/null || true
adb shell su -c "rm -rf ${RESET_DIRS[*]}" 2>/dev/null || true

echo "Removing app data dirs (CE and DE, where core's socket lives)..."
adb shell su -c "rm -rf /data/user/0/dev.okhsunrog.vpnhide /data/user_de/0/dev.okhsunrog.vpnhide" 2>/dev/null || true

echo "Done. Reboot recommended if modules were installed."
