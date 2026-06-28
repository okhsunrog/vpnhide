package dev.okhsunrog.vpnhide

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

internal data class RootSnapshot(
    val sections: Map<String, String>,
)

internal class RootSnapshotException(
    message: String,
) : RuntimeException(message)

private const val ROOT_SNAPSHOT_BEGIN_PREFIX = "__VPNHIDE_ROOT_SECTION_BEGIN__:"
private const val ROOT_SNAPSHOT_END_PREFIX = "__VPNHIDE_ROOT_SECTION_END__:"
private const val ROOT_TIMING_PREFIX = "__VPNHIDE_ROOT_TIMING__:"
private const val ROOT_SNAPSHOT_TIMEOUT_SEC: Long = 5

internal val REQUIRED_ROOT_SNAPSHOT_SECTIONS =
    setOf(
        "kmod_prop",
        "zygisk_prop",
        "kpm_prop",
        "ports_prop",
        "kmod_module_dir",
        "zygisk_module_dir",
        "kpm_module_dir",
        "canonical_config",
        "kmod_targets",
        "zygisk_targets",
        "kpm_targets",
        "kpm_load_status",
        "lsposed_targets",
        "hidden_pkgs",
        "observer_uids",
        "ports_observers",
        "superkey_saved",
        "current_boot_id",
        "kmod_load_status",
        "kmod_load_dmesg",
        "kernel_release",
        "lsposed_state",
        "debug_logging",
        "getenforce",
        "pm_packages",
        "proc_exists",
        "ports_chain",
        "lsposed_framework",
        "vpn_ifaces",
    )

/**
 * Single in-process source for root-owned/system state. Dashboard and
 * Protection derive different UI models from the same cached snapshot, so
 * their counts/statuses cannot drift because two independent shell snapshots
 * raced.
 */
internal object RootSnapshotCache {
    private val _snapshot = MutableStateFlow<RootSnapshot?>(null)
    val snapshot: StateFlow<RootSnapshot?> = _snapshot.asStateFlow()

    private val _loading = MutableStateFlow(false)
    val loading: StateFlow<Boolean> = _loading.asStateFlow()

    private val mutex = Mutex()
    private var preloadedPmPackages: String? = null

    suspend fun getOrLoad(): RootSnapshot =
        withContext(Dispatchers.IO) {
            _snapshot.value?.let { return@withContext it }
            mutex.withLock {
                _snapshot.value ?: loadLocked()
            }
        }

    suspend fun refresh(): RootSnapshot =
        withContext(Dispatchers.IO) {
            mutex.withLock {
                preloadedPmPackages = null
                loadLocked()
            }
        }

    fun invalidate() {
        _snapshot.value = null
        preloadedPmPackages = null
    }

    fun seedPmPackages(pmPackages: String?) {
        preloadedPmPackages = pmPackages?.trimEnd()?.takeIf { it.isNotBlank() }
    }

    private fun loadLocked(): RootSnapshot {
        _loading.value = true
        return try {
            StartupTrace.mark("root_snapshot_start")
            val pmPackages = preloadedPmPackages
            preloadedPmPackages = null
            val sections = loadRootShellSnapshot(pmPackagesOverride = pmPackages)
            val snapshot = RootSnapshot(sections)
            _snapshot.value = snapshot
            StartupTrace.mark("root_snapshot_done")
            snapshot
        } catch (e: Exception) {
            StartupTrace.mark("root_snapshot_failed")
            throw e
        } finally {
            _loading.value = false
        }
    }
}

private fun loadRootShellSnapshot(pmPackagesOverride: String?): Map<String, String> {
    val (exitCode, raw) =
        suExec(
            buildRootShellSnapshotCommand(includePmPackages = pmPackagesOverride == null),
            timeoutSec = ROOT_SNAPSHOT_TIMEOUT_SEC,
        )
    if (exitCode != 0) {
        throw RootSnapshotException("root snapshot command failed with exit=$exitCode")
    }
    val sections = parseRootShellSnapshot(raw).toMutableMap()
    if (pmPackagesOverride != null) {
        sections["pm_packages"] = pmPackagesOverride
    }
    validateRootSnapshotSections(sections)
    return sections
}

internal fun validateRootSnapshotSections(sections: Map<String, String>) {
    val missing = REQUIRED_ROOT_SNAPSHOT_SECTIONS.filterNot(sections::containsKey)
    if (missing.isNotEmpty()) {
        throw RootSnapshotException("root snapshot incomplete, missing sections: ${missing.joinToString()}")
    }
}

// Long because it's a single embedded shell script (the batched root probe),
// not Kotlin control flow.
@Suppress("LongMethod")
internal fun buildRootShellSnapshotCommand(includePmPackages: Boolean = true): String =
    """
    emit_cmd() {
      NAME="${'$'}1"
      shift
      echo "$ROOT_SNAPSHOT_BEGIN_PREFIX${'$'}NAME"
      "$@" 2>/dev/null || true
      echo "$ROOT_SNAPSHOT_END_PREFIX${'$'}NAME"
    }
    emit_file() {
      NAME="${'$'}1"
      PATH_TO_READ="${'$'}2"
      echo "$ROOT_SNAPSHOT_BEGIN_PREFIX${'$'}NAME"
      if [ -f "${'$'}PATH_TO_READ" ]; then
        cat "${'$'}PATH_TO_READ" 2>/dev/null || true
      fi
      echo "$ROOT_SNAPSHOT_END_PREFIX${'$'}NAME"
    }
    emit_eval() {
      NAME="${'$'}1"
      shift
      echo "$ROOT_SNAPSHOT_BEGIN_PREFIX${'$'}NAME"
      eval "${'$'}*" 2>/dev/null || true
      echo "$ROOT_SNAPSHOT_END_PREFIX${'$'}NAME"
    }
    now_ms() {
      if [ -n "${'$'}{EPOCHREALTIME:-}" ]; then
        SEC="${'$'}{EPOCHREALTIME%.*}"
        FRAC="${'$'}{EPOCHREALTIME#*.}"
      else
        IFS=' .' read -r SEC FRAC _ < /proc/uptime
      fi
      FRAC="${'$'}{FRAC}000"
      FRAC="${'$'}{FRAC%${'$'}{FRAC#???}}"
      while [ -n "${'$'}FRAC" ] && [ "${'$'}{FRAC#0}" != "${'$'}FRAC" ]; do
        FRAC="${'$'}{FRAC#0}"
      done
      [ -n "${'$'}FRAC" ] || FRAC=0
      case "${'$'}SEC${'$'}FRAC" in
        ''|*[!0-9]*) echo 0 ;;
        *) echo ${'$'}((SEC * 1000 + FRAC)) ;;
      esac
    }
    phase_start() {
      PHASE_NAME="${'$'}1"
      PHASE_START="${'$'}(now_ms)"
    }
    phase_end() {
      END="${'$'}(now_ms)"
      echo "$ROOT_TIMING_PREFIX${'$'}PHASE_NAME=${'$'}((END - PHASE_START))"
    }
    phase_module_props() {
      phase_start module_props
      emit_file kmod_prop $KMOD_MODULE_DIR/module.prop
      emit_file zygisk_prop $ZYGISK_MODULE_DIR/module.prop
      emit_file kpm_prop $KPM_MODULE_DIR/module.prop
      emit_file ports_prop $PORTS_MODULE_DIR/module.prop
      emit_eval kmod_module_dir '[ -d $KMOD_MODULE_DIR ] && echo 1 || echo 0'
      emit_eval zygisk_module_dir '[ -d $ZYGISK_MODULE_DIR ] && echo 1 || echo 0'
      emit_eval kpm_module_dir '[ -d $KPM_MODULE_DIR ] && echo 1 || echo 0'
      phase_end
    }
    phase_target_files() {
      phase_start target_files
      emit_file canonical_config $CANONICAL_CONFIG_FILE
      # Migration shim: the legacy files below are read only when canonical JSON
      # is absent. Remove after a few public releases with the ShellUtils consts.
      emit_file kmod_targets $KMOD_TARGETS
      emit_file zygisk_targets $ZYGISK_TARGETS
      emit_file kpm_targets $KPM_TARGETS
      emit_file lsposed_targets $LSPOSED_TARGETS
      emit_file hidden_pkgs $SS_HIDDEN_PKGS_FILE
      emit_file observer_uids $SS_OBSERVER_UIDS_FILE
      emit_file ports_observers $PORTS_OBSERVERS_FILE
      emit_eval superkey_saved '[ -s $SUPERKEY_FILE ] && echo 1 || echo 0'
      phase_end
    }
    phase_kmod_status_files() {
      phase_start kmod_status_files
      emit_file current_boot_id /proc/sys/kernel/random/boot_id
      emit_file kmod_load_status $KMOD_LOAD_STATUS_FILE
      emit_file kmod_load_dmesg $KMOD_LOAD_DMESG_FILE
      emit_file kpm_load_status $KPM_LOAD_STATUS_FILE
      phase_end
    }
    phase_runtime_status_files() {
      phase_start runtime_status_files
      emit_cmd kernel_release uname -r
      emit_file lsposed_state $LSPOSED_STATE_FILE
      emit_file debug_logging /data/system/vpnhide_debug_logging
      emit_cmd getenforce getenforce
      phase_end
    }
    __VPNHIDE_PM_PACKAGES_FUNCTION__
    phase_proc_exists() {
      phase_start shell_probe_proc_exists
      emit_eval proc_exists '[ -e $PROC_CTL ] && echo 1 || echo 0'
      phase_end
    }
    phase_ports_chain() {
      phase_start shell_probe_ports_chain
      emit_eval ports_chain '
        iptables -L vpnhide_out -n >/dev/null 2>&1 &&
        iptables -C OUTPUT -j vpnhide_out >/dev/null 2>&1 &&
        ip6tables -L vpnhide_out6 -n >/dev/null 2>&1 &&
        ip6tables -C OUTPUT -j vpnhide_out6 >/dev/null 2>&1 &&
        echo 1 || echo 0
      '
      phase_end
    }
    phase_lsposed_framework() {
      phase_start shell_probe_lsposed_framework
      emit_eval lsposed_framework 'FOUND=0; for id in zygisk_vector zygisk_lsposed lsposed; do for base in /data/adb/modules /data/adb/modules_update; do dir="${'$'}base/${'$'}id"; if [ -f "${'$'}dir/module.prop" ]; then echo installed=1; if [ -f "${'$'}dir/disable" ]; then echo disabled=1; else echo disabled=0; fi; FOUND=1; break 2; fi; done; done; [ "${'$'}FOUND" = 1 ] || echo installed=0; echo probe_ok=1'
      phase_end
    }
    phase_vpn_ifaces() {
      phase_start shell_probe_vpn_ifaces
      emit_cmd vpn_ifaces grep -H . /sys/class/net/*/operstate
      phase_end
    }
    run_all_phases_sequential() {
      phase_module_props
      phase_target_files
      phase_kmod_status_files
      phase_runtime_status_files
      __VPNHIDE_PM_PACKAGES_PHASE__
      phase_proc_exists
      phase_ports_chain
      phase_lsposed_framework
      phase_vpn_ifaces
    }
    run_all_phases_sequential
    """.trimIndent()
        .replace(
            "__VPNHIDE_PM_PACKAGES_FUNCTION__",
            if (includePmPackages) {
                """
                phase_pm_packages() {
                  phase_start pm_packages
                  emit_cmd pm_packages pm list packages -U --user all
                  phase_end
                }
                """.trimIndent()
            } else {
                ""
            },
        ).replace(
            "__VPNHIDE_PM_PACKAGES_PHASE__",
            if (includePmPackages) "phase_pm_packages" else ":",
        )

internal fun parseRootShellSnapshot(
    raw: String,
    recordMetric: (String, Long) -> Unit = StartupTrace::metric,
): Map<String, String> {
    val sections = linkedMapOf<String, String>()
    var currentName: String? = null
    val currentBody = StringBuilder()

    fun finishSection(name: String) {
        sections[name] = currentBody.toString()
        currentBody.clear()
        currentName = null
    }

    raw.lineSequence().forEach { line ->
        if (line.startsWith(ROOT_TIMING_PREFIX)) {
            val body = line.removePrefix(ROOT_TIMING_PREFIX)
            val parts = body.split("=", limit = 2)
            val name = parts.getOrNull(0)?.trim().orEmpty()
            val durationMs = parts.getOrNull(1)?.trim()?.toLongOrNull()
            if (name.isNotEmpty() && durationMs != null) {
                recordMetric("root_shell_$name", durationMs)
            }
            return@forEach
        }
        if (line.startsWith(ROOT_SNAPSHOT_BEGIN_PREFIX)) {
            currentName = line.removePrefix(ROOT_SNAPSHOT_BEGIN_PREFIX)
            currentBody.clear()
            return@forEach
        }
        if (line.startsWith(ROOT_SNAPSHOT_END_PREFIX)) {
            val endName = line.removePrefix(ROOT_SNAPSHOT_END_PREFIX)
            val name =
                currentName?.takeIf { it == endName } ?: run {
                    currentBody.clear()
                    currentName = null
                    return@forEach
                }
            finishSection(name)
            return@forEach
        }
        if (currentName != null) {
            if (currentBody.isNotEmpty()) currentBody.append('\n')
            currentBody.append(line)
        }
    }
    return sections
}
