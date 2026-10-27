package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.startup.StartupTrace
import kotlinx.coroutines.flow.StateFlow
import java.util.concurrent.CopyOnWriteArrayList
import uniffi.vpnhide_core_ffi.CoreDiagnosticProbe as CoreProbe

internal data class RootSnapshot(
    val sections: Map<String, String>,
    val observationId: Long = 0,
    val generation: Long = 0,
    val core: CoreSessionUpdate = CoreSessionUpdate(),
) {
    val canonical: RootCanonicalRead get() = core.state?.canonical.takeIf { core.current } ?: RootCanonicalRead.Unavailable
    val nativeBoot: NativeBootStatus? get() = core.state?.nativeBoot.takeIf { core.current }
    val backend: CoreBackendView? get() = if (core.current) core.state?.backend else CoreBackendView(current = false)
}

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

internal val REQUIRED_ROOT_SNAPSHOT_SECTIONS =
    setOf(
        "kmod_prop",
        "zygisk_prop",
        "kpm_prop",
        "core_prop",
        "kmod_module_dir",
        "zygisk_module_dir",
        "kpm_module_dir",
        "core_module_dir",
        "kmod_artifact_state",
        "kpm_artifact_state",
        "zygisk_artifact_state",
        "kmod_disabled",
        "kpm_disabled",
        "zygisk_disabled",
        "core_disabled",
        "kmod_pending_update",
        "kpm_pending_update",
        "zygisk_pending_update",
        "core_pending_update",
        "canonical_config",
        "superkey_saved",
        "current_boot_id",
        "zygisk_status",
        "kernel_release",
        "kmod_state",
        "kpm_state",
        "lsposed_state",
        "getenforce",
        "kpatch_runtime",
        "pm_packages",
        "pm_users",
        "snapshot_shell_uid",
        "proc_exists",
        "ports_chain",
        "lsposed_framework",
    )

/**
 * Single in-process source for root-owned/system state. Dashboard and
 * Protection derive different UI models from the same cached snapshot, so
 * each projection carries the source observation identity. Retained screen values
 * may differ while a dependent refresh is still running.
 */
internal object RootSnapshotCache : StateCache<RootSnapshot>(
    traceName = "root_snapshot",
    logTag = LogTags.APP,
    timeoutMillis = 15_000,
) {
    val snapshot: StateFlow<RootSnapshot?> get() = value
    private val dependents = CopyOnWriteArrayList<() -> Unit>()
    private val inventoryDependents = CopyOnWriteArrayList<() -> Unit>()
    private val inputLock = Any()
    private var preloadedPackageInventory: PackageInventorySeed? = null

    // A complete validated snapshot read by startup with the same command; consumed
    // by the next load instead of a second shell. Any invalidation or explicit
    // refresh discards it, so a config write after the read can never be masked.
    private var preloadedSections: Map<String, String>? = null

    val dependency =
        object : ObservationDependency {
            override fun subscribe(invalidate: () -> Unit) {
                dependents.add(invalidate)
            }

            override fun refresh() {
                requestRefresh()
            }
        }
    val inventoryDependency =
        object : ObservationDependency {
            override fun subscribe(invalidate: () -> Unit) {
                inventoryDependents.add(invalidate)
            }

            override fun refresh() {
                requestRefresh()
            }
        }

    suspend fun getOrLoad(): RootSnapshot = awaitValue()

    suspend fun refresh(notBefore: Long = Long.MIN_VALUE): RootSnapshot {
        synchronized(inputLock) {
            preloadedPackageInventory = null
            preloadedSections = null
        }
        return awaitValue(refresh = true, notBefore = notBefore)
    }

    override fun invalidate() {
        synchronized(inputLock) {
            preloadedPackageInventory = null
            preloadedSections = null
        }
        super.invalidate()
    }

    fun seedPackageInventory(seed: PackageInventorySeed?) {
        synchronized(inputLock) { preloadedPackageInventory = seed }
    }

    /** Seed the next load with a complete startup read; a config write invalidates it like any other observation. */
    fun seedSnapshot(sections: Map<String, String>?) {
        synchronized(inputLock) { preloadedSections = sections?.toMap() }
    }

    override suspend fun load(request: ObservationRequest): RootSnapshot {
        val (seeded, inventory) =
            synchronized(inputLock) {
                Pair(preloadedSections, preloadedPackageInventory).also {
                    preloadedSections = null
                    preloadedPackageInventory = null
                }
            }
        val sections =
            if (seeded != null) {
                StartupTrace.mark("root_snapshot_seeded")
                seeded.also(::validateRootSnapshotSections)
            } else {
                loadRootShellSnapshot(inventory)
            }
        return joinRootSnapshot(sections, CoreSessions.updates.value, request.id, request.generation)
    }

    override fun observationChanged(
        previous: ObservationState<RootSnapshot>,
        next: ObservationState<RootSnapshot>,
    ) {
        if (rootObservationInvalidatesDependents(previous, next)) dependents.forEach { it() }
        if (rootObservationInvalidatesInventory(previous, next)) inventoryDependents.forEach { it() }
    }
}

/**
 * The batched root probe core runs for the dashboard and the Hiding tab
 * (`collect_diagnostics` `RootSnapshot`; core-daemon `device_probes/root_snapshot.rs`), plus
 * core's typed native boot decision from its published state.
 */
internal fun loadRootShellSnapshot(inventoryOverride: PackageInventorySeed?): Map<String, String> {
    val result = CoreRoot.collectOneBlocking(CoreProbe.RootSnapshot(packages = inventoryOverride == null))
    if (!result.ok || result.truncated) throw RootSnapshotException("root snapshot read failed (core exit=${result.exit})")
    val sections = parseRootShellSnapshot(result.text).toMutableMap()
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

/** Seeded startup reads and new probes join through this same typed provenance boundary. */
internal fun joinRootSnapshot(
    sections: Map<String, String>,
    core: CoreSessionUpdate,
    observationId: Long = 0,
    generation: Long = 0,
): RootSnapshot = RootSnapshot(sections.toMap(), observationId, generation, core)
