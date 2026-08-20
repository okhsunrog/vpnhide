# Everything a bug report needs from the device, framed into named sections:
# module state, backend status, kernel/network forensics and the per-user app
# scan. One su round-trip; the app parses the sections back out.
#
# Inputs (assigned by the Kotlin caller, see ShellScripts.kt): VPNHIDE_SECTION_*,
# the module/file paths, and VPNHIDE_LEGACY_SECTIONS ("name=path name=path").
# vpnhide_package_inventory() comes from package_inventory.sh, concatenated
# ahead of this file.
#
# emit_eval bodies are single-quoted on purpose: emit_eval re-parses them with
# `eval`, so their $VAR references must survive to that point — which is what
# SC2016 warns about.
# shellcheck disable=SC2016
emit_cmd() {
  NAME="$1"
  shift
  echo "${VPNHIDE_SECTION_BEGIN}$NAME"
  "$@" 2>&1 || true
  echo "${VPNHIDE_SECTION_END}$NAME"
}
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
emit_eval() {
  NAME="$1"
  shift
  echo "${VPNHIDE_SECTION_BEGIN}$NAME"
  eval "$*" 2>&1 || true
  echo "${VPNHIDE_SECTION_END}$NAME"
}
prop() {
  VALUE="$(getprop "$1" 2>/dev/null)"
  [ -n "$VALUE" ] && printf '%s=%s\n' "$1" "$VALUE"
}
file_flags() {
  DIR="$1"
  echo "path=$DIR"
  if [ -d "$DIR" ]; then
    echo "installed=1"
    [ -f "$DIR/disable" ] && echo "disabled=1" || echo "disabled=0"
    [ -f "$DIR/remove" ] && echo "remove=1" || echo "remove=0"
    [ -f "$DIR/update" ] && echo "update=1" || echo "update=0"
    ls -ldZ "$DIR" 2>&1 || ls -ld "$DIR" 2>&1 || true
  else
    echo "installed=0"
  fi
}
hash_file() {
  PATH_TO_HASH="$1"
  if [ -f "$PATH_TO_HASH" ]; then
    ls -lZ "$PATH_TO_HASH" 2>&1 || ls -l "$PATH_TO_HASH" 2>&1 || true
    sha256sum "$PATH_TO_HASH" 2>&1 || true
  else
    echo "(missing: $PATH_TO_HASH)"
  fi
}
# Reports the copy staged in modules_update/ awaiting the next reboot. A
# complete staged install with the active activator absent means "installed,
# reboot needed" rather than a corrupt install (matches pending_update in the
# runtime snapshot).
staged_state() {
  STAGED=/data/adb/modules_update/${1##*/}
  if [ ! -d "$STAGED" ]; then
    echo "staged=0"
  elif [ -x "$STAGED/activator" ]; then
    echo "staged=1 staged_activator=executable"
  elif [ -e "$STAGED/activator" ]; then
    echo "staged=1 staged_activator=not_executable"
  else
    echo "staged=1 staged_activator=missing"
  fi
}
redact_cmdline() {
  sed -E \
    -e 's/(androidboot[.]serialno=)[^ ]+/\1<redacted>/g' \
    -e 's/(androidboot[.]wifi_macaddr=)[^ ]+/\1<redacted>/g' \
    -e 's/(androidboot[.]btmacaddr=)[^ ]+/\1<redacted>/g'
}
kpatch_bin() {
  for CANDIDATE in \
    kpatch \
    /data/adb/modules/KPatch-Next/bin/kpatch \
    /data/adb/modules/kpatch-next/bin/kpatch
  do
    if command -v "$CANDIDATE" >/dev/null 2>&1; then
      command -v "$CANDIDATE"
      return 0
    fi
    if [ -x "$CANDIDATE" ]; then
      echo "$CANDIDATE"
      return 0
    fi
  done
  return 1
}

emit_cmd current_boot_id cat /proc/sys/kernel/random/boot_id
emit_cmd uname uname -a
emit_file proc_version /proc/version
emit_eval proc_cmdline 'cat /proc/cmdline 2>/dev/null | redact_cmdline'
emit_eval getprop_selected '
  for P in \
    ro.product.manufacturer ro.product.brand ro.product.model \
    ro.product.device ro.product.name ro.product.board \
    ro.product.cpu.abilist ro.build.fingerprint ro.build.version.release \
    ro.build.version.sdk ro.build.version.security_patch \
    ro.vendor.build.security_patch ro.bootimage.build.fingerprint \
    ro.vendor.build.fingerprint ro.odm.build.fingerprint \
    ro.boot.slot_suffix ro.boot.bootloader ro.boot.hardware \
    ro.boot.hardware.sku ro.boot.verifiedbootstate \
    ro.boot.vbmeta.device_state ro.boot.flash.locked \
    ro.boot.dynamic_partitions ro.kernel.qemu sys.boot_completed
  do
    prop "$P"
  done
'
emit_eval root_manager '
  echo "id=$(id 2>/dev/null)"
  echo "su=$(command -v su 2>/dev/null || true)"
  echo "magisk_version_name=$(magisk -v 2>/dev/null || true)"
  echo "magisk_version_code=$(magisk -V 2>/dev/null || true)"
  echo "ksu_version=$(cat /data/adb/ksu/version 2>/dev/null || true)"
  echo "ksud_version=$(ksud --version 2>/dev/null || true)"
  [ -d /data/adb/ap ] && echo "apatch_dir=1" || echo "apatch_dir=0"
  [ -x /data/adb/apd ] && echo "apd=1" || echo "apd=0"
  [ -d /data/adb/magisk ] && echo "magisk_dir=1" || echo "magisk_dir=0"
  [ -d /data/adb/ksu ] && echo "ksu_dir=1" || echo "ksu_dir=0"
  ls -ldZ /data/adb /data/adb/modules /data/adb/modules_update 2>&1 ||
    ls -ld /data/adb /data/adb/modules /data/adb/modules_update 2>&1 || true
'
emit_eval selinux '
  getenforce 2>/dev/null || true
  cat /sys/fs/selinux/enforce 2>/dev/null || true
  ls -Zd /data /data/adb /data/system /proc/net /sys/class/net 2>&1 ||
    ls -ld /data /data/adb /data/system /proc/net /sys/class/net 2>&1 || true
'
emit_eval kernel_config '
  if [ -r /proc/config.gz ]; then
    zcat /proc/config.gz 2>/dev/null |
      grep -E "CONFIG_(KPROBES|KRETPROBES|MODULES|MODULE_SIG|SECURITY_SELINUX|NETFILTER|IP_NF|IP6_NF|CGROUP_BPF)=" || true
  else
    echo "(missing: /proc/config.gz)"
  fi
'
emit_eval proc_modules 'cat /proc/modules 2>/dev/null | grep -iE "vpnhide|kpatch|ksu|magisk|zygisk" || true'
emit_eval kprobes 'cat /sys/kernel/debug/kprobes/list 2>/dev/null | grep -iE "vpnhide|dev_ioctl|sock_ioctl|rtnl|inet.*fill|route" || true'
emit_eval kernel_symbols '
  for SYM in dev_ioctl dev_ifconf sock_ioctl rtnl_fill_ifinfo inet6_fill_ifaddr inet_fill_ifaddr fib_route_seq_show ipv6_route_seq_show fib_dump_info rt6_fill_node fib_nl_fill_rule; do
    printf "%s: " "$SYM"
    grep -w "$SYM" /proc/kallsyms 2>/dev/null | head -3 || true
  done
'
emit_eval module_inventory '
  for BASE in /data/adb/modules /data/adb/modules_update; do
    [ -d "$BASE" ] || continue
    for ID in \
      vpnhide_kmod vpnhide_kpm vpnhide_zygisk vpnhide_ports \
      KPatch-Next kpatch-next zygisk_vector zygisk_lsposed lsposed \
      zygisk_next zygisknext zygisksu NeoZygisk
    do
      DIR="$BASE/$ID"
      [ -e "$DIR" ] || continue
      echo "### $DIR"
      file_flags "$DIR"
      if [ -f "$DIR/module.prop" ]; then
        sed "s/^/module.prop: /" "$DIR/module.prop" 2>&1 || true
      fi
      ls -la "$DIR" 2>&1 | sed -n "1,80p" || true
      echo
    done
  done
'

emit_file kmod_prop "$VPNHIDE_KMOD_DIR"/module.prop
emit_eval kmod_module_state 'file_flags $VPNHIDE_KMOD_DIR; hash_file $VPNHIDE_KMOD_DIR/vpnhide_kmod.ko; hash_file $VPNHIDE_KMOD_ACTIVATOR; staged_state $VPNHIDE_KMOD_DIR'
emit_file kmod_load_status "$VPNHIDE_KMOD_LOAD_STATUS"
emit_file kmod_load_dmesg "$VPNHIDE_KMOD_LOAD_DMESG"
emit_eval kmod_state '[ -e $VPNHIDE_PROC_CTL ] && cat $VPNHIDE_PROC_CTL 2>&1 || echo "(missing: $VPNHIDE_PROC_CTL)"'
# Diagnostic-only companion to the control node: hook masks, kretprobe nmissed
# counters and the live is_vpn_ifname() verdict per netdev. Never parsed — it is
# catted verbatim for a human reading a field report.
emit_eval kmod_diag '[ -e $VPNHIDE_PROC_DIAG ] && cat $VPNHIDE_PROC_DIAG 2>&1 || echo "(missing: $VPNHIDE_PROC_DIAG)"'

emit_file kpm_prop "$VPNHIDE_KPM_DIR"/module.prop
emit_eval kpm_module_state 'file_flags $VPNHIDE_KPM_DIR; hash_file $VPNHIDE_KPM_DIR/vpnhide.kpm; hash_file $VPNHIDE_KPM_ACTIVATOR; staged_state $VPNHIDE_KPM_DIR'
emit_file kpm_load_status "$VPNHIDE_KPM_LOAD_STATUS"
emit_eval kpm_state '
  if [ -x $VPNHIDE_KPM_ACTIVATOR ] && [ ! -f $VPNHIDE_KPM_DIR/disable ]; then
    $VPNHIDE_KPM_ACTIVATOR state 2>&1
    echo "[exit=$?]"
  else
    echo "(KPM activator missing or disabled)"
  fi
'
emit_eval kpatch_runtime '
  [ -d /data/adb/ap ] && echo "apatch_dir=1" || echo "apatch_dir=0"
  [ -x /data/adb/apd ] && echo "apd=1" || echo "apd=0"
  [ -s $VPNHIDE_SUPERKEY_FILE ] && echo "superkey_saved=1" || echo "superkey_saved=0"
  if [ -f /data/adb/fp/kpms/kpm_autoload_config.json ]; then
    echo "folkpatch_kpm_autoload=1"
    sed -n "1,80p" /data/adb/fp/kpms/kpm_autoload_config.json 2>&1 || true
  else
    echo "folkpatch_kpm_autoload=0"
  fi
  BIN="$(kpatch_bin 2>/dev/null || true)"
  if [ -n "$BIN" ]; then
    echo "kpatch_bin=$BIN"
    "$BIN" hello 2>&1
    echo "hello_exit=$?"
    "$BIN" kpm list 2>&1
    echo "list_exit=$?"
  else
    echo "kpatch_bin=(not found)"
  fi
'

emit_file zygisk_prop "$VPNHIDE_ZYGISK_DIR"/module.prop
emit_eval zygisk_module_state 'file_flags $VPNHIDE_ZYGISK_DIR; hash_file $VPNHIDE_ZYGISK_DIR/zygisk/arm64-v8a.so; hash_file $VPNHIDE_ZYGISK_ACTIVATOR; staged_state $VPNHIDE_ZYGISK_DIR'
emit_file zygisk_status "$VPNHIDE_ZYGISK_STATUS"
emit_eval zygisk_runtime '
  for BASE in /data/adb/modules /data/adb/modules_update; do
    [ -d "$BASE" ] || continue
    find "$BASE" -maxdepth 1 \( -iname "*zygisk*" -o -iname "*lsposed*" -o -iname "*vector*" \) -type d -print 2>/dev/null
  done
'

emit_file ports_prop "$VPNHIDE_PORTS_DIR"/module.prop
emit_eval ports_module_state 'file_flags $VPNHIDE_PORTS_DIR; hash_file $VPNHIDE_PORTS_ACTIVATOR; staged_state $VPNHIDE_PORTS_DIR'
emit_file ports_load_status "$VPNHIDE_PORTS_LOAD_STATUS"
emit_file ports_load_log "$VPNHIDE_PORTS_LOAD_LOG"
emit_eval ports_state '
  iptables -S OUTPUT 2>&1 | grep vpnhide || true
  iptables -S vpnhide_out 2>&1 || true
  iptables -L vpnhide_out -n -v --line-numbers 2>&1 || true
  ip6tables -S OUTPUT 2>&1 | grep vpnhide || true
  ip6tables -S vpnhide_out6 2>&1 || true
  ip6tables -L vpnhide_out6 -n -v --line-numbers 2>&1 || true
'

emit_file lsposed_state "$VPNHIDE_LSPOSED_STATE"
emit_eval lsposed_framework '
  for BASE in /data/adb/modules /data/adb/modules_update; do
    [ -d "$BASE" ] || continue
    for ID in zygisk_vector zygisk_lsposed lsposed; do
      DIR="$BASE/$ID"
      [ -f "$DIR/module.prop" ] || continue
      echo "### $DIR"
      file_flags "$DIR"
      sed "s/^/module.prop: /" "$DIR/module.prop" 2>&1 || true
    done
  done
'
emit_eval lsposed_files '
  ls -laZ /data/adb/lspd/config 2>&1 || ls -la /data/adb/lspd/config 2>&1 || true
  ls -lZ /data/adb/lspd/config/modules_config.db* 2>&1 ||
    ls -l /data/adb/lspd/config/modules_config.db* 2>&1 || true
'

# Network state first: the per-user package scan further down can eat most of
# the su timeout on a bloatware-heavy ROM, and bundles came back truncated
# with exactly these sections missing — the ones a routing bug needs.
emit_cmd network_addr ip -d addr
emit_eval network_operstate 'for IFACE in /sys/class/net/*; do echo "$(basename "$IFACE"): $(cat "$IFACE/operstate" 2>/dev/null)"; done'
emit_cmd network_routes ip route show table all
emit_cmd network_rules ip rule
emit_eval network_sockets 'ss -H -ltnup 2>/dev/null | grep -E "127[.]|::1|LISTEN|udp" | head -300 || true'
emit_eval connectivity_dump 'dumpsys connectivity 2>/dev/null | grep -iE "vpn|tun|NetworkAgentInfo|NetworkCapabilities|LinkProperties|rmnet|wlan|dummy" | head -400 || true'

emit_file proc_net_route /proc/net/route
emit_file proc_net_ipv6_route /proc/net/ipv6_route
emit_file proc_net_if_inet6 /proc/net/if_inet6
emit_file proc_net_tcp /proc/net/tcp
emit_file proc_net_tcp6 /proc/net/tcp6
emit_file proc_net_udp /proc/net/udp
emit_file proc_net_udp6 /proc/net/udp6
emit_file proc_net_dev /proc/net/dev
emit_eval proc_net_fib_trie 'cat /proc/net/fib_trie 2>&1 | sed -n "1,1200p" || true'

emit_file canonical_config "$VPNHIDE_CONFIG_FILE"
# Pre-1.0 lists. Present only on an install that upgraded across the 1.2.0
# gap without importing yet, so a bundle still shows what the import would
# have folded in (it deletes them once it runs).
# "name=path name=path" from the Kotlin constants; no spaces in the paths, so
# word splitting is the iteration.
# shellcheck disable=SC2086
for VPNHIDE_LEGACY in $VPNHIDE_LEGACY_SECTIONS; do
  emit_file "${VPNHIDE_LEGACY%%=*}" "${VPNHIDE_LEGACY#*=}"
done
vpnhide_package_inventory

# Privacy-safe summary of the per-user app scan: profile list with names
# redacted, then per-user exit code / package counts / format flags / first
# stderr line — NO package names or paths. This is what diagnoses "couldn't
# read all profiles" cases (which user fails, and whether it is a non-zero
# exit, an empty list, or a `-U`/`-f` format the parser misses). The full
# package inventory is intentionally never written to the bundle.
emit_eval app_scan_diagnostics '
  PLAIN=$(pm list users 2>/dev/null)
  echo "--- pm list users (names redacted) ---"
  printf "%s\n" "$PLAIN" | sed -E "s/(UserInfo\{[0-9]+:)[^:]*(:[0-9a-fA-F]*\})/\1<name>\2/"
  IDS=$(printf "%s\n" "$PLAIN" | sed -n "s/.*UserInfo{\([0-9][0-9]*\):.*/\1/p")
  [ -z "$IDS" ] && IDS=0
  echo "--- per-user pm list packages -U -f (counts only, no names) ---"
  for U in $IDS; do
    RUN=0
    printf "%s\n" "$PLAIN" | grep "UserInfo{$U:" | grep -qw running && RUN=1
    ERR=$(pm list packages -U -f --user "$U" 2>&1 1>/dev/null | head -2 | tr "\n" " ")
    # NOTE: never write an apostrophe inside this single-quoted block. It is
    # not a comment to the shell (# only starts one outside quotes), so the
    # apostrophe closes the quoting and the whole snapshot command stops
    # parsing.
    # Stage the pm output in a temp file rather than a shell variable: a very
    # large app list (bloatware-heavy MIUI/HyperOS) exceeds the kernel
    # single-argument limit, so `printf "%s\n" "$OUT"` would fail with
    # "Argument list too long" and count zero packages. A file never hits
    # ARG_MAX. pm is the only command on its line, so $? is its status.
    OUT_FILE=/data/local/tmp/vpnhide_app_scan.$$.$U
    pm list packages -U -f --user "$U" >"$OUT_FILE" 2>/dev/null
    EX=$?
    TOTAL=$(grep -c "^package:" "$OUT_FILE")
    WUID=$(grep -c "^package:.* uid:" "$OUT_FILE")
    WPATH=$(grep -c "^package:[^ ]*=" "$OUT_FILE")
    rm -f "$OUT_FILE"
    echo "user=$U running=$RUN exit=$EX package_lines=$TOTAL with_uid=$WUID with_path=$WPATH stderr=[$ERR]"
  done
  echo "inprocess_backstop=app also unions getInstalledApplications(0) into user 0"
'

