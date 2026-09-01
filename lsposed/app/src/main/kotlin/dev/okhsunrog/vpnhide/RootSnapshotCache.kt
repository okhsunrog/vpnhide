package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.PM_USERS_STATUS_PREFIX
import dev.okhsunrog.vpnhide.picker.PM_USER_BEGIN_PREFIX
import dev.okhsunrog.vpnhide.picker.PM_USER_END_PREFIX
import dev.okhsunrog.vpnhide.startup.StartupTrace
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

internal data class PackageInventorySeed(
    val packages: String,
    val users: String,
)

internal class RootSnapshotException(
    message: String,
) : RuntimeException(message)

private const val ROOT_SNAPSHOT_BEGIN_PREFIX = "__VPNHIDE_ROOT_SECTION_BEGIN__:"
private const val ROOT_SNAPSHOT_END_PREFIX = "__VPNHIDE_ROOT_SECTION_END__:"
private const val ROOT_TIMING_PREFIX = "__VPNHIDE_ROOT_TIMING__:"
private const val ROOT_SNAPSHOT_TIMEOUT_SEC: Long = 10

internal val REQUIRED_ROOT_SNAPSHOT_SECTIONS =
    setOf(
        "kmod_prop",
        "builtin_prop",
        "zygisk_prop",
        "kpm_prop",
        "ports_prop",
        "kmod_module_dir",
        "builtin_module_dir",
        "zygisk_module_dir",
        "kpm_module_dir",
        "kmod_activator_state",
        "builtin_activator_state",
        "kpm_activator_state",
        "zygisk_activator_state",
        "ports_activator_state",
        "kmod_disabled",
        "builtin_disabled",
        "kpm_disabled",
        "zygisk_disabled",
        "ports_disabled",
        "canonical_config",
        "kpm_load_status",
        "ports_load_status",
        "superkey_saved",
        "current_boot_id",
        "kmod_load_status",
        "kmod_load_dmesg",
        "builtin_load_status",
        "zygisk_status",
        "kernel_release",
        "kmod_state",
        "kpm_state",
        "kpm_runtime_modules",
        "lsposed_state",
        "getenforce",
        "kpatch_runtime",
        "pm_packages",
        "pm_users",
        "snapshot_shell_uid",
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
    private var preloadedPackageInventory: PackageInventorySeed? = null
    private var runtimeProbeSource: String? = null

    fun setRuntimeProbeSource(path: String?) {
        runtimeProbeSource = path?.takeIf { it.matches(Regex("/[A-Za-z0-9_./-]+")) }
    }

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
                preloadedPackageInventory = null
                loadLocked()
            }
        }

    fun invalidate() {
        _snapshot.value = null
        preloadedPackageInventory = null
    }

    fun seedPackageInventory(seed: PackageInventorySeed?) {
        preloadedPackageInventory = seed
    }

    private fun loadLocked(): RootSnapshot {
        _loading.value = true
        return try {
            StartupTrace.mark("root_snapshot_start")
            val inventory = preloadedPackageInventory
            preloadedPackageInventory = null
            val sections = loadRootShellSnapshot(inventoryOverride = inventory, runtimeProbeSource = runtimeProbeSource)
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

private fun loadRootShellSnapshot(
    inventoryOverride: PackageInventorySeed?,
    runtimeProbeSource: String?,
): Map<String, String> {
    val (exitCode, raw) =
        suExec(
            buildRootShellSnapshotCommand(
                includePmPackages = inventoryOverride == null,
                runtimeProbeSource = runtimeProbeSource,
            ),
            timeoutSec = ROOT_SNAPSHOT_TIMEOUT_SEC,
        )
    if (exitCode != 0) {
        throw RootSnapshotException("root snapshot command failed with exit=$exitCode")
    }
    val sections = parseRootShellSnapshot(raw).toMutableMap()
    if (inventoryOverride != null) {
        sections["pm_packages"] = inventoryOverride.packages
        sections["pm_users"] = inventoryOverride.users
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

/**
 * The batched root probe: one `su` round-trip that answers every question the
 * dashboard and the Hiding tab ask. The script itself lives in
 * `resources/shell/root_snapshot.sh` (with the shared package inventory
 * concatenated ahead of it); this only supplies the paths and prefixes it reads,
 * so a path stays defined once, in Kotlin.
 */
internal fun buildRootShellSnapshotCommand(
    includePmPackages: Boolean = true,
    runtimeProbeSource: String? = null,
): String =
    shellVariables(
        mapOf(
            "VPNHIDE_SECTION_BEGIN" to ROOT_SNAPSHOT_BEGIN_PREFIX,
            "VPNHIDE_SECTION_END" to ROOT_SNAPSHOT_END_PREFIX,
            "VPNHIDE_TIMING" to ROOT_TIMING_PREFIX,
            "VPNHIDE_PM_USERS_STATUS" to PM_USERS_STATUS_PREFIX,
            "VPNHIDE_PM_USER_BEGIN" to PM_USER_BEGIN_PREFIX,
            "VPNHIDE_PM_USER_END" to PM_USER_END_PREFIX,
            "VPNHIDE_PM_STDERR_TO_STDOUT" to "0",
            "VPNHIDE_WITH_PM" to if (includePmPackages) "1" else "0",
            "VPNHIDE_KPM_PROBE_SOURCE" to runtimeProbeSource.orEmpty(),
            "VPNHIDE_KMOD_DIR" to KMOD_MODULE_DIR,
            "VPNHIDE_BUILTIN_DIR" to BUILTIN_MODULE_DIR,
            "VPNHIDE_KPM_DIR" to KPM_MODULE_DIR,
            "VPNHIDE_ZYGISK_DIR" to ZYGISK_MODULE_DIR,
            "VPNHIDE_PORTS_DIR" to PORTS_MODULE_DIR,
            "VPNHIDE_KMOD_ACTIVATOR" to KMOD_ACTIVATOR,
            "VPNHIDE_BUILTIN_ACTIVATOR" to BUILTIN_ACTIVATOR,
            "VPNHIDE_KPM_ACTIVATOR" to KPM_ACTIVATOR,
            "VPNHIDE_ZYGISK_ACTIVATOR" to ZYGISK_ACTIVATOR,
            "VPNHIDE_PORTS_ACTIVATOR" to PORTS_ACTIVATOR,
            "VPNHIDE_CONFIG_FILE" to CANONICAL_CONFIG_FILE,
            "VPNHIDE_SUPERKEY_FILE" to SUPERKEY_FILE,
            "VPNHIDE_KMOD_LOAD_STATUS" to KMOD_LOAD_STATUS_FILE,
            "VPNHIDE_KMOD_LOAD_DMESG" to KMOD_LOAD_DMESG_FILE,
            "VPNHIDE_BUILTIN_LOAD_STATUS" to BUILTIN_LOAD_STATUS_FILE,
            "VPNHIDE_ZYGISK_STATUS" to ZYGISK_STATUS_FILE,
            "VPNHIDE_KPM_LOAD_STATUS" to KPM_LOAD_STATUS_FILE,
            "VPNHIDE_PORTS_LOAD_STATUS" to PORTS_LOAD_STATUS_FILE,
            "VPNHIDE_PROC_CTL" to PROC_CTL,
            "VPNHIDE_LSPOSED_STATE" to LSPOSED_STATE_FILE,
            "VPNHIDE_LEGACY_SECTIONS" to
                LEGACY_CONFIG_SECTIONS.entries.joinToString(" ") { (section, path) -> "$section=$path" },
        ),
    ) + ShellScripts.load("package_inventory.sh") + "\n" + ShellScripts.load("root_snapshot.sh")

internal fun parseRootShellSnapshot(
    raw: String,
    recordMetric: (String, Long) -> Unit = StartupTrace::metric,
): Map<String, String> =
    parseFramedSections(
        raw = raw,
        beginPrefix = ROOT_SNAPSHOT_BEGIN_PREFIX,
        endPrefix = ROOT_SNAPSHOT_END_PREFIX,
        policy =
            FramedSectionParsePolicy(
                preserveIncomplete = false,
                discardOnMismatchedEnd = true,
                trimSectionEnd = false,
            ),
        consumeLine = { line ->
            if (!line.startsWith(ROOT_TIMING_PREFIX)) {
                false
            } else {
                val (name, duration) =
                    line.removePrefix(ROOT_TIMING_PREFIX).split("=", limit = 2).let {
                        it.firstOrNull()?.trim().orEmpty() to it.getOrNull(1)?.trim()?.toLongOrNull()
                    }
                if (name.isNotEmpty() && duration != null) recordMetric("root_shell_$name", duration)
                true
            }
        },
    ).complete
