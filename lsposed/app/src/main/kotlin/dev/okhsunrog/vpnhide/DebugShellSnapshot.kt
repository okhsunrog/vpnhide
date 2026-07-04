package dev.okhsunrog.vpnhide

internal data class DebugShellSnapshot(
    val sections: Map<String, String>,
    val exitCode: Int,
)

private const val DEBUG_SNAPSHOT_BEGIN_PREFIX = "__VPNHIDE_DEBUG_SECTION_BEGIN__:"
private const val DEBUG_SNAPSHOT_END_PREFIX = "__VPNHIDE_DEBUG_SECTION_END__:"

// The batch runs many heavy root commands (pm list --user all, dumpsys
// connectivity, ip route show table all, fib_trie, several sha256sum). On a busy
// device 20s was easy to overrun, which truncated the output mid-section and
// silently dropped that section and every later one. Give it real headroom.
private const val DEBUG_SNAPSHOT_TIMEOUT_SEC: Long = 60
private const val COUNTER_SNAPSHOT_TIMEOUT_SEC: Long = 8

internal fun collectDebugShellSnapshot(): DebugShellSnapshot {
    val (exit, raw) =
        suExec(
            buildDebugShellSnapshotCommand(),
            timeoutSec = DEBUG_SNAPSHOT_TIMEOUT_SEC,
        )
    val sections = parseDebugShellSnapshot(raw).toMutableMap()
    if (exit != 0) {
        sections["debug_snapshot_error"] = "root debug snapshot command failed with exit=$exit"
    }
    return DebugShellSnapshot(sections = sections, exitCode = exit)
}

internal fun collectHookCounterSnapshot(): DebugShellSnapshot {
    val (exit, raw) =
        suExec(
            buildHookCounterSnapshotCommand(),
            timeoutSec = COUNTER_SNAPSHOT_TIMEOUT_SEC,
        )
    val sections = parseDebugShellSnapshot(raw).toMutableMap()
    if (exit != 0) {
        sections["debug_snapshot_error"] = "root counter snapshot command failed with exit=$exit"
    }
    return DebugShellSnapshot(sections = sections, exitCode = exit)
}

internal fun parseDebugShellSnapshot(raw: String): Map<String, String> {
    val sections = linkedMapOf<String, String>()
    var currentName: String? = null
    val currentBody = StringBuilder()

    fun finishSection(name: String) {
        sections[name] = currentBody.toString().trimEnd()
        currentBody.clear()
        currentName = null
    }

    raw.lineSequence().forEach { line ->
        when {
            line.startsWith(DEBUG_SNAPSHOT_BEGIN_PREFIX) -> {
                currentName = line.removePrefix(DEBUG_SNAPSHOT_BEGIN_PREFIX)
                currentBody.clear()
            }

            line.startsWith(DEBUG_SNAPSHOT_END_PREFIX) -> {
                val endName = line.removePrefix(DEBUG_SNAPSHOT_END_PREFIX)
                if (currentName == endName) finishSection(endName)
            }

            currentName != null -> {
                currentBody.appendLine(line)
            }
        }
    }
    // A command that overran the su timeout leaves the last section open (no END
    // marker) and every later section absent. Keep the partial body but flag it,
    // so a bug report shows "cut off here" instead of a silently-missing section.
    currentName?.let { name ->
        sections[name] =
            (currentBody.toString().trimEnd() + "\n(TRUNCATED: snapshot cut off before this section completed)")
                .trim()
        sections["debug_snapshot_truncated"] = name
    }
    return sections
}

// Long because it is one batched root-side diagnostic script, not Kotlin logic.
@Suppress("LongMethod")
internal fun buildDebugShellSnapshotCommand(): String =
    """
    emit_cmd() {
      NAME="${'$'}1"
      shift
      echo "$DEBUG_SNAPSHOT_BEGIN_PREFIX${'$'}NAME"
      "$@" 2>&1 || true
      echo "$DEBUG_SNAPSHOT_END_PREFIX${'$'}NAME"
    }
    emit_file() {
      NAME="${'$'}1"
      PATH_TO_READ="${'$'}2"
      echo "$DEBUG_SNAPSHOT_BEGIN_PREFIX${'$'}NAME"
      if [ -f "${'$'}PATH_TO_READ" ]; then
        cat "${'$'}PATH_TO_READ" 2>&1 || true
      else
        echo "(missing: ${'$'}PATH_TO_READ)"
      fi
      echo "$DEBUG_SNAPSHOT_END_PREFIX${'$'}NAME"
    }
    emit_eval() {
      NAME="${'$'}1"
      shift
      echo "$DEBUG_SNAPSHOT_BEGIN_PREFIX${'$'}NAME"
      eval "${'$'}*" 2>&1 || true
      echo "$DEBUG_SNAPSHOT_END_PREFIX${'$'}NAME"
    }
    prop() {
      VALUE="${'$'}(getprop "${'$'}1" 2>/dev/null)"
      [ -n "${'$'}VALUE" ] && printf '%s=%s\n' "${'$'}1" "${'$'}VALUE"
    }
    file_flags() {
      DIR="${'$'}1"
      echo "path=${'$'}DIR"
      if [ -d "${'$'}DIR" ]; then
        echo "installed=1"
        [ -f "${'$'}DIR/disable" ] && echo "disabled=1" || echo "disabled=0"
        [ -f "${'$'}DIR/remove" ] && echo "remove=1" || echo "remove=0"
        [ -f "${'$'}DIR/update" ] && echo "update=1" || echo "update=0"
        ls -ldZ "${'$'}DIR" 2>&1 || ls -ld "${'$'}DIR" 2>&1 || true
      else
        echo "installed=0"
      fi
    }
    hash_file() {
      PATH_TO_HASH="${'$'}1"
      if [ -f "${'$'}PATH_TO_HASH" ]; then
        ls -lZ "${'$'}PATH_TO_HASH" 2>&1 || ls -l "${'$'}PATH_TO_HASH" 2>&1 || true
        sha256sum "${'$'}PATH_TO_HASH" 2>&1 || true
      else
        echo "(missing: ${'$'}PATH_TO_HASH)"
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
        if command -v "${'$'}CANDIDATE" >/dev/null 2>&1; then
          command -v "${'$'}CANDIDATE"
          return 0
        fi
        if [ -x "${'$'}CANDIDATE" ]; then
          echo "${'$'}CANDIDATE"
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
        prop "${'$'}P"
      done
    '
    emit_eval root_manager '
      echo "id=${'$'}(id 2>/dev/null)"
      echo "su=${'$'}(command -v su 2>/dev/null || true)"
      echo "magisk_version_name=${'$'}(magisk -v 2>/dev/null || true)"
      echo "magisk_version_code=${'$'}(magisk -V 2>/dev/null || true)"
      echo "ksu_version=${'$'}(cat /data/adb/ksu/version 2>/dev/null || true)"
      echo "ksud_version=${'$'}(ksud --version 2>/dev/null || true)"
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
        printf "%s: " "${'$'}SYM"
        grep -w "${'$'}SYM" /proc/kallsyms 2>/dev/null | head -3 || true
      done
    '
    emit_eval module_inventory '
      for BASE in /data/adb/modules /data/adb/modules_update; do
        [ -d "${'$'}BASE" ] || continue
        for ID in \
          vpnhide_kmod vpnhide_kpm vpnhide_zygisk vpnhide_ports \
          KPatch-Next kpatch-next zygisk_vector zygisk_lsposed lsposed \
          zygisk_next zygisknext zygisksu NeoZygisk
        do
          DIR="${'$'}BASE/${'$'}ID"
          [ -e "${'$'}DIR" ] || continue
          echo "### ${'$'}DIR"
          file_flags "${'$'}DIR"
          if [ -f "${'$'}DIR/module.prop" ]; then
            sed "s/^/module.prop: /" "${'$'}DIR/module.prop" 2>&1 || true
          fi
          ls -la "${'$'}DIR" 2>&1 | sed -n "1,80p" || true
          echo
        done
      done
    '

    emit_file kmod_prop $KMOD_MODULE_DIR/module.prop
    emit_eval kmod_module_state 'file_flags $KMOD_MODULE_DIR; hash_file $KMOD_MODULE_DIR/vpnhide_kmod.ko; hash_file $KMOD_ACTIVATOR'
    emit_file kmod_targets $KMOD_TARGETS
    emit_file kmod_load_status $KMOD_LOAD_STATUS_FILE
    emit_file kmod_load_dmesg $KMOD_LOAD_DMESG_FILE
    emit_eval kmod_state '[ -e $PROC_CTL ] && cat $PROC_CTL 2>&1 || echo "(missing: $PROC_CTL)"'

    emit_file kpm_prop $KPM_MODULE_DIR/module.prop
    emit_eval kpm_module_state 'file_flags $KPM_MODULE_DIR; hash_file $KPM_MODULE_DIR/vpnhide.kpm; hash_file $KPM_ACTIVATOR'
    emit_file kpm_targets $KPM_TARGETS
    emit_file kpm_load_status $KPM_LOAD_STATUS_FILE
    emit_eval kpm_state '
      if [ -x $KPM_ACTIVATOR ] && [ ! -f $KPM_MODULE_DIR/disable ]; then
        $KPM_ACTIVATOR state 2>&1
        echo "[exit=${'$'}?]"
      else
        echo "(KPM activator missing or disabled)"
      fi
    '
    emit_eval kpatch_runtime '
      [ -d /data/adb/ap ] && echo "apatch_dir=1" || echo "apatch_dir=0"
      [ -x /data/adb/apd ] && echo "apd=1" || echo "apd=0"
      [ -s $SUPERKEY_FILE ] && echo "superkey_saved=1" || echo "superkey_saved=0"
      if [ -f /data/adb/fp/kpms/kpm_autoload_config.json ]; then
        echo "folkpatch_kpm_autoload=1"
        sed -n "1,80p" /data/adb/fp/kpms/kpm_autoload_config.json 2>&1 || true
      else
        echo "folkpatch_kpm_autoload=0"
      fi
      BIN="${'$'}(kpatch_bin 2>/dev/null || true)"
      if [ -n "${'$'}BIN" ]; then
        echo "kpatch_bin=${'$'}BIN"
        "${'$'}BIN" hello 2>&1
        echo "hello_exit=${'$'}?"
        "${'$'}BIN" kpm list 2>&1
        echo "list_exit=${'$'}?"
      else
        echo "kpatch_bin=(not found)"
      fi
    '

    emit_file zygisk_prop $ZYGISK_MODULE_DIR/module.prop
    emit_eval zygisk_module_state 'file_flags $ZYGISK_MODULE_DIR; hash_file $ZYGISK_MODULE_DIR/zygisk/arm64-v8a.so; hash_file $ZYGISK_ACTIVATOR'
    emit_file zygisk_targets $ZYGISK_TARGETS
    emit_file zygisk_status $ZYGISK_STATUS_FILE
    emit_eval zygisk_runtime '
      for BASE in /data/adb/modules /data/adb/modules_update; do
        [ -d "${'$'}BASE" ] || continue
        find "${'$'}BASE" -maxdepth 1 \( -iname "*zygisk*" -o -iname "*lsposed*" -o -iname "*vector*" \) -type d -print 2>/dev/null
      done
    '

    emit_file ports_prop $PORTS_MODULE_DIR/module.prop
    emit_eval ports_module_state 'file_flags $PORTS_MODULE_DIR; hash_file $PORTS_ACTIVATOR'
    emit_file ports_load_status $PORTS_LOAD_STATUS_FILE
    emit_file ports_load_log $PORTS_LOAD_LOG_FILE
    emit_file ports_observers $PORTS_OBSERVERS_FILE
    emit_eval ports_state '
      iptables -S OUTPUT 2>&1 | grep vpnhide || true
      iptables -S vpnhide_out 2>&1 || true
      iptables -L vpnhide_out -n -v --line-numbers 2>&1 || true
      ip6tables -S OUTPUT 2>&1 | grep vpnhide || true
      ip6tables -S vpnhide_out6 2>&1 || true
      ip6tables -L vpnhide_out6 -n -v --line-numbers 2>&1 || true
    '

    emit_file lsposed_state $LSPOSED_STATE_FILE
    emit_eval lsposed_framework '
      for BASE in /data/adb/modules /data/adb/modules_update; do
        [ -d "${'$'}BASE" ] || continue
        for ID in zygisk_vector zygisk_lsposed lsposed; do
          DIR="${'$'}BASE/${'$'}ID"
          [ -f "${'$'}DIR/module.prop" ] || continue
          echo "### ${'$'}DIR"
          file_flags "${'$'}DIR"
          sed "s/^/module.prop: /" "${'$'}DIR/module.prop" 2>&1 || true
        done
      done
    '
    emit_eval lsposed_files '
      ls -laZ /data/adb/lspd/config 2>&1 || ls -la /data/adb/lspd/config 2>&1 || true
      ls -lZ /data/adb/lspd/config/modules_config.db* 2>&1 ||
        ls -l /data/adb/lspd/config/modules_config.db* 2>&1 || true
    '

    emit_file canonical_config $CANONICAL_CONFIG_FILE
    emit_file hidden_pkgs $SS_HIDDEN_PKGS_FILE
    emit_file observer_uids $SS_OBSERVER_UIDS_FILE
    emit_cmd pm_packages pm list packages -U --user all

    emit_cmd network_addr ip -d addr
    emit_eval network_operstate 'for IFACE in /sys/class/net/*; do echo "${'$'}(basename "${'$'}IFACE"): ${'$'}(cat "${'$'}IFACE/operstate" 2>/dev/null)"; done'
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
    """.trimIndent()

internal fun buildHookCounterSnapshotCommand(): String =
    """
    emit_file() {
      NAME="${'$'}1"
      PATH_TO_READ="${'$'}2"
      echo "$DEBUG_SNAPSHOT_BEGIN_PREFIX${'$'}NAME"
      if [ -f "${'$'}PATH_TO_READ" ]; then
        cat "${'$'}PATH_TO_READ" 2>&1 || true
      else
        echo "(missing: ${'$'}PATH_TO_READ)"
      fi
      echo "$DEBUG_SNAPSHOT_END_PREFIX${'$'}NAME"
    }
    emit_cmd() {
      NAME="${'$'}1"
      shift
      echo "$DEBUG_SNAPSHOT_BEGIN_PREFIX${'$'}NAME"
      "$@" 2>&1 || true
      echo "$DEBUG_SNAPSHOT_END_PREFIX${'$'}NAME"
    }
    emit_eval() {
      NAME="${'$'}1"
      shift
      echo "$DEBUG_SNAPSHOT_BEGIN_PREFIX${'$'}NAME"
      eval "${'$'}*" 2>&1 || true
      echo "$DEBUG_SNAPSHOT_END_PREFIX${'$'}NAME"
    }

    emit_cmd current_boot_id cat /proc/sys/kernel/random/boot_id
    emit_eval kmod_state '[ -e $PROC_CTL ] && cat $PROC_CTL 2>&1 || echo "(missing: $PROC_CTL)"'
    emit_eval kpm_state '
      if [ -x $KPM_ACTIVATOR ] && [ ! -f $KPM_MODULE_DIR/disable ]; then
        $KPM_ACTIVATOR state 2>&1
        echo "[exit=${'$'}?]"
      else
        echo "(KPM activator missing or disabled)"
      fi
    '
    emit_file lsposed_state $LSPOSED_STATE_FILE
    emit_file zygisk_status $ZYGISK_STATUS_FILE
    """.trimIndent()
