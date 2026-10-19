# Sourced or copied into legacy backend service.sh scripts.
# If the common core module is present and not disabled, skip legacy activation.
# shellcheck disable=SC2034
VPNHIDE_CORE_MOD="/data/adb/modules/vpnhide_core"
if [ -d "$VPNHIDE_CORE_MOD" ] && [ ! -f "$VPNHIDE_CORE_MOD/disable" ]; then
  if [ -x "$VPNHIDE_CORE_MOD/vpnhide-core" ] || [ -f /data/adb/vpnhide/core/config_owned ]; then
    log -t vpnhide "legacy activator skipped: vpnhide_core owns boot apply"
    exit 0
  fi
fi
