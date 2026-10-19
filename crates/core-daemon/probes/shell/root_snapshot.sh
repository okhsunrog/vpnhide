# One batched root probe: every piece of system state the dashboard and the
# Hiding tab derive from, framed into named sections so one
# request answers everything. Core runs it for the `root_snapshot` probe.
#
# Inputs (assigned by core, see diagnostics.rs snapshot_vars): VPNHIDE_SECTION_*,
# VPNHIDE_TIMING, the module/file paths, VPNHIDE_LEGACY_SECTIONS
# ("name=path name=path"), VPNHIDE_WITH_PM (1 = include the package inventory)
# and VPNHIDE_CORE_EXE (core itself, for its read-only `observe` children).
#
# vpnhide_package_inventory() comes from package_inventory.sh, which the caller
# concatenates ahead of this file.
#
# emit_eval bodies are single-quoted on purpose: they are re-parsed by `eval`
# inside emit_eval, so their $VAR references must survive to that point. That is
# what SC2016 warns about, so it is off for this file.
# shellcheck disable=SC2016
emit_cmd() {
  NAME="$1"
  shift
  echo "${VPNHIDE_SECTION_BEGIN}$NAME"
  "$@" 2>/dev/null || true
  echo "${VPNHIDE_SECTION_END}$NAME"
}
emit_file() {
  NAME="$1"
  PATH_TO_READ="$2"
  echo "${VPNHIDE_SECTION_BEGIN}$NAME"
  if [ -f "$PATH_TO_READ" ]; then
    cat "$PATH_TO_READ" 2>/dev/null || true
  fi
  echo "${VPNHIDE_SECTION_END}$NAME"
}
emit_eval() {
  NAME="$1"
  shift
  echo "${VPNHIDE_SECTION_BEGIN}$NAME"
  eval "$*" 2>/dev/null || true
  echo "${VPNHIDE_SECTION_END}$NAME"
}
# An installed module's payload: "present" when every listed file is a regular
# file, otherwise "missing". Core loads exactly these artifacts.
artifact_state() {
  for ARTIFACT in "$@"; do
    [ -f "$ARTIFACT" ] || {
      echo missing
      return
    }
  done
  echo present
}
# 1 when a module update is staged in modules_update/ awaiting the next reboot:
# the root manager keeps the freshly-installed files there and only swaps them
# into modules/ on boot. Distinguishes "just installed, reboot needed" from a
# genuinely corrupt install.
pending_update() {
  STAGED=/data/adb/modules_update/${1##*/}
  if [ -f "$STAGED/module.prop" ]; then
    echo 1
  elif [ -f "$1/update" ] && [ -d "$STAGED" ]; then
    echo 1
  else
    echo 0
  fi
}
now_ms() {
  if [ -n "${EPOCHREALTIME:-}" ]; then
    SEC="${EPOCHREALTIME%.*}"
    FRAC="${EPOCHREALTIME#*.}"
  else
    IFS=' .' read -r SEC FRAC _ < /proc/uptime
  fi
  FRAC="${FRAC}000"
  FRAC="${FRAC%"${FRAC#???}"}"
  while [ -n "$FRAC" ] && [ "${FRAC#0}" != "$FRAC" ]; do
    FRAC="${FRAC#0}"
  done
  [ -n "$FRAC" ] || FRAC=0
  case "$SEC$FRAC" in
    ''|*[!0-9]*) echo 0 ;;
    *) echo $((SEC * 1000 + FRAC)) ;;
  esac
}
phase_start() {
  PHASE_NAME="$1"
  PHASE_START="$(now_ms)"
}
phase_end() {
  END="$(now_ms)"
  echo "${VPNHIDE_TIMING}$PHASE_NAME=$((END - PHASE_START))"
}
phase_module_props() {
  phase_start module_props
  emit_file kmod_prop "$VPNHIDE_KMOD_DIR"/module.prop
  emit_file zygisk_prop "$VPNHIDE_ZYGISK_DIR"/module.prop
  emit_file kpm_prop "$VPNHIDE_KPM_DIR"/module.prop
  emit_file core_prop "$VPNHIDE_CORE_DIR"/module.prop
  emit_eval kmod_module_dir '[ -d $VPNHIDE_KMOD_DIR ] && echo 1 || echo 0'
  emit_eval zygisk_module_dir '[ -d $VPNHIDE_ZYGISK_DIR ] && echo 1 || echo 0'
  emit_eval kpm_module_dir '[ -d $VPNHIDE_KPM_DIR ] && echo 1 || echo 0'
  emit_eval core_module_dir '[ -d $VPNHIDE_CORE_DIR ] && echo 1 || echo 0'
  emit_eval kmod_artifact_state 'artifact_state $VPNHIDE_KMOD_DIR/vpnhide_kmod.ko'
  emit_eval kpm_artifact_state 'artifact_state $VPNHIDE_KPM_DIR/vpnhide.kpm'
  emit_eval zygisk_artifact_state '
    if [ -f $VPNHIDE_ZYGISK_DIR/zygisk/arm64-v8a.so ] || [ -f $VPNHIDE_ZYGISK_DIR/zygisk/armeabi-v7a.so ]; then
      echo present
    else
      echo missing
    fi'
  emit_eval kmod_disabled '[ -f $VPNHIDE_KMOD_DIR/disable ] && echo 1 || echo 0'
  emit_eval kpm_disabled '[ -f $VPNHIDE_KPM_DIR/disable ] && echo 1 || echo 0'
  emit_eval zygisk_disabled '[ -f $VPNHIDE_ZYGISK_DIR/disable ] && echo 1 || echo 0'
  emit_eval core_disabled '[ -f $VPNHIDE_CORE_DIR/disable ] && echo 1 || echo 0'
  emit_eval kmod_pending_update 'pending_update $VPNHIDE_KMOD_DIR'
  emit_eval kpm_pending_update 'pending_update $VPNHIDE_KPM_DIR'
  emit_eval zygisk_pending_update 'pending_update $VPNHIDE_ZYGISK_DIR'
  emit_eval core_pending_update 'pending_update $VPNHIDE_CORE_DIR'
  phase_end
}
phase_target_files() {
  phase_start target_files
  emit_file canonical_config "$VPNHIDE_CONFIG_FILE"
  emit_eval superkey_saved '[ -s $VPNHIDE_SUPERKEY_FILE ] && echo 1 || echo 0'
  phase_end
}
# Pre-1.0 per-component lists. Nothing writes them today, so a non-empty
# section means an install that skipped the 1.0.x migration window and can
# still have its choices recovered (LegacyConfigImport).
phase_legacy_config() {
  phase_start legacy_config
  # "name=path name=path", built from the Kotlin constants so the paths stay
  # defined once. They carry no spaces, so word splitting is the iteration.
  # shellcheck disable=SC2086
  for VPNHIDE_LEGACY in $VPNHIDE_LEGACY_SECTIONS; do
    emit_file "${VPNHIDE_LEGACY%%=*}" "${VPNHIDE_LEGACY#*=}"
  done
  phase_end
}
phase_kmod_status_files() {
  phase_start kmod_status_files
  emit_file current_boot_id /proc/sys/kernel/random/boot_id
  emit_file kmod_load_status "$VPNHIDE_KMOD_LOAD_STATUS"
  emit_file kmod_load_dmesg "$VPNHIDE_KMOD_LOAD_DMESG"
  emit_file zygisk_status "$VPNHIDE_ZYGISK_STATUS"
  emit_file kpm_load_status "$VPNHIDE_KPM_LOAD_STATUS"
  emit_file core_boot_status "$VPNHIDE_CORE_BOOT_STATUS"
  phase_end
}
phase_runtime_status_files() {
  phase_start runtime_status_files
  emit_cmd kernel_release uname -r
  emit_eval kmod_state '[ -e $VPNHIDE_PROC_CTL ] && cat $VPNHIDE_PROC_CTL || true'
  emit_eval kpm_state '"$VPNHIDE_CORE_EXE" observe kpm-state'
  emit_eval kpm_runtime_modules '
    "$VPNHIDE_CORE_EXE" observe kpm-list 2>/dev/null'
  emit_file lsposed_state "$VPNHIDE_LSPOSED_STATE"
  emit_cmd getenforce getenforce
  # Is a *live* KPM runtime present (kernel actually patched, able to load
  # KPMs)? Two runtimes qualify:
  #   - APatch native KernelPatch: loads KPMs via supercall, detected by its
  #     /data/adb/ap dir (no kpatch CLI on disk — APatch keeps it in the
  #     manager app's private libs).
  #   - KPatch-Next-Module on any manager (Magisk / KSU / KSU-Next): ships
  #     the kpatch CLI at a fixed module path. Installing the module is not
  #     enough — the boot image must be patched from its UI first, so probe
  #     liveness the same way KPatch-Next's own status.sh does: `kpatch
  #     hello` succeeds only when the kernel is patched.
  # kpatchRuntimeAvailable() reads apatch_dir/hello_exit from this. Verified
  # on a Pixel 4a (APatch) and a Pixel 8 Pro (KSU-Next + KPatch-Next).
  emit_eval kpatch_runtime '
    [ -d /data/adb/ap ] && echo "apatch_dir=1" || echo "apatch_dir=0"
    KP=""
    for CAND in kpatch /data/adb/modules/KPatch-Next/bin/kpatch /data/adb/modules/kpatch-next/bin/kpatch; do
      if command -v "$CAND" >/dev/null 2>&1; then KP="$CAND"; break; fi
      [ -x "$CAND" ] && { KP="$CAND"; break; }
    done
    if [ -n "$KP" ]; then
      echo "kpatch_bin=$KP"
      "$KP" hello >/dev/null 2>&1
      echo "hello_exit=$?"
    else
      echo "kpatch_bin="
    fi'
  phase_end
}
phase_pm_packages() {
  phase_start pm_packages
  vpnhide_package_inventory
  phase_end
}
phase_shell_identity() {
  phase_start shell_probe_identity
  # Who is the snapshot shell, really? The liveness probes below read
  # root-only runtime resources (0600 /proc/vpnhide_ctl; iptables needs
  # CAP_NET_ADMIN). If this shell is not uid 0 — e.g. a KernelSU grant that
  # raced or degraded — those probes read a false negative, and a "0" from
  # proc_exists/ports_chain must NOT be rendered as "inactive". The detectors
  # gate runtimeCheckable on this uid; errno_ctl distinguishes EACCES (no
  # access) from ENOENT (truly absent) for the bundle.
  emit_eval snapshot_shell_uid '
    echo "uid=$(id -u 2>/dev/null)"
    echo "id=$(id 2>/dev/null)"
    echo "context=$(cat /proc/self/attr/current 2>/dev/null | tr -d "\0")"
    if [ -e $VPNHIDE_PROC_CTL ]; then
      echo "errno_ctl=ok"
    else
      ERR=$(ls $VPNHIDE_PROC_CTL 2>&1 1>/dev/null)
      case "$ERR" in
        *[Pp]ermission*) echo "errno_ctl=eacces" ;;
        *o\ such*) echo "errno_ctl=enoent" ;;
        *) echo "errno_ctl=other:$ERR" ;;
      esac
    fi
  '
  phase_end
}
phase_proc_exists() {
  phase_start shell_probe_proc_exists
  emit_eval proc_exists '[ -e $VPNHIDE_PROC_CTL ] && echo 1 || echo 0'
  phase_end
}
phase_ports_chain() {
  phase_start shell_probe_ports_chain
  # "1" when both chains exist and OUTPUT jumps to them, "0" when a chain or a
  # jump is absent (iptables exits 1 for a missing chain, 2 for a missing
  # target), "error=<rc>" for anything else: 4 when another process holds the
  # xtables lock (netd on a network change, the VPN client, core
  # applying Ports), 3 without CAP_NET_ADMIN. -w waits briefly for the lock instead
  # of failing at once; a probe that still could not run is reported as an
  # error and rendered as "not verified", never as an inactive module.
  emit_eval ports_chain '
    RESULT=1
    for CMD in "iptables -w 2 -L vpnhide_out -n" "iptables -w 2 -C OUTPUT -j vpnhide_out" \
               "ip6tables -w 2 -L vpnhide_out6 -n" "ip6tables -w 2 -C OUTPUT -j vpnhide_out6"; do
      $CMD >/dev/null 2>&1
      RC=$?
      case "$RC" in
        0) ;;
        1|2) RESULT=0; break ;;
        *) RESULT="error=$RC"; break ;;
      esac
    done
    echo "$RESULT"
  '
  phase_end
}
phase_lsposed_framework() {
  phase_start shell_probe_lsposed_framework
  emit_eval lsposed_framework 'FOUND=0; for id in zygisk_vector zygisk_lsposed lsposed; do for base in /data/adb/modules /data/adb/modules_update; do dir="$base/$id"; if [ -f "$dir/module.prop" ]; then echo installed=1; if [ -f "$dir/disable" ]; then echo disabled=1; else echo disabled=0; fi; FOUND=1; break 2; fi; done; done; [ "$FOUND" = 1 ] || echo installed=0; echo probe_ok=1'
  phase_end
}
run_all_phases_sequential() {
  phase_module_props
  phase_target_files
  phase_legacy_config
  phase_kmod_status_files
  phase_runtime_status_files
  if [ "$VPNHIDE_WITH_PM" = 1 ]; then
    phase_pm_packages
  fi
  phase_shell_identity
  phase_proc_exists
  phase_ports_chain
  phase_lsposed_framework
}
run_all_phases_sequential
