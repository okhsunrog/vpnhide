# The cheap counter-only probe: hook hit counters from whichever backend is
# live, without the full snapshot's cost. Used to diff counters across a forced
# check run.
#
# Inputs: VPNHIDE_SECTION_* and the backend paths (see ShellScripts.kt).
# shellcheck disable=SC2016
emit_file() {
  NAME="$1"
  PATH_TO_READ="$2"
  echo "${VPNHIDE_SECTION_BEGIN}$NAME"
  if [ -f "$PATH_TO_READ" ]; then
    cat "$PATH_TO_READ" 2>&1 || true
  else
    echo "(missing: $PATH_TO_READ)"
  fi
  echo "${VPNHIDE_SECTION_END}$NAME"
}
emit_cmd() {
  NAME="$1"
  shift
  echo "${VPNHIDE_SECTION_BEGIN}$NAME"
  "$@" 2>&1 || true
  echo "${VPNHIDE_SECTION_END}$NAME"
}
emit_eval() {
  NAME="$1"
  shift
  echo "${VPNHIDE_SECTION_BEGIN}$NAME"
  eval "$*" 2>&1 || true
  echo "${VPNHIDE_SECTION_END}$NAME"
}

emit_cmd current_boot_id cat /proc/sys/kernel/random/boot_id
emit_eval kmod_state '[ -e $VPNHIDE_PROC_CTL ] && cat $VPNHIDE_PROC_CTL 2>&1 || echo "(missing: $VPNHIDE_PROC_CTL)"'
emit_eval kpm_state '
  if [ -x $VPNHIDE_KPM_ACTIVATOR ] && [ ! -f $VPNHIDE_KPM_DIR/disable ]; then
    $VPNHIDE_KPM_ACTIVATOR state 2>&1
    echo "[exit=$?]"
  else
    echo "(KPM activator missing or disabled)"
  fi
'
emit_file lsposed_state "$VPNHIDE_LSPOSED_STATE"
emit_file zygisk_status "$VPNHIDE_ZYGISK_STATUS"