package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.startup.StartupTrace
import kotlinx.coroutines.flow.StateFlow
import uniffi.vpnhide_core_ffi.CoreProbeFrameKind
import uniffi.vpnhide_core_ffi.coreParseProbeSections
import uniffi.vpnhide_core_ffi.coreValidateRootSnapshotSections
import java.util.concurrent.CopyOnWriteArrayList
import uniffi.vpnhide_core_ffi.CoreDiagnosticProbe as CoreProbe

internal const val ROOT_SNAPSHOT_TIMEOUT_MS = 225_000L

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

internal class RootSnapshotException(
    message: String,
    val kind: SelfTargetFailureKind = SelfTargetFailureKind.IncompleteData,
) : RuntimeException(message)

/**
 * Single in-process source for root-owned/system state. Dashboard and
 * Protection derive different UI models from the same cached snapshot, so
 * each projection carries the source observation identity. Retained screen values
 * may differ while a dependent refresh is still running.
 */
internal object RootSnapshotCache : StateCache<RootSnapshot>(
    traceName = "root_snapshot",
    logTag = LogTags.APP,
    // Covers CoreRoot's 220s call envelope (200s collector / 210s RPC), including KPM input.
    timeoutMillis = ROOT_SNAPSHOT_TIMEOUT_MS,
) {
    val snapshot: StateFlow<RootSnapshot?> get() = value
    private val dependents = CopyOnWriteArrayList<() -> Unit>()
    private val inventoryDependents = CopyOnWriteArrayList<() -> Unit>()

    val dependency =
        object : ObservationDependency {
            override val collectionBudgetMillis: Long = ROOT_SNAPSHOT_TIMEOUT_MS

            override fun subscribe(invalidate: () -> Unit) {
                dependents.add(invalidate)
            }

            override fun refresh() {
                requestRefresh()
            }
        }
    val inventoryDependency =
        object : ObservationDependency {
            override val collectionBudgetMillis: Long = ROOT_SNAPSHOT_TIMEOUT_MS

            override fun subscribe(invalidate: () -> Unit) {
                inventoryDependents.add(invalidate)
            }

            override fun refresh() {
                requestRefresh()
            }
        }

    suspend fun getOrLoad(): RootSnapshot = awaitValue()

    suspend fun refresh(notBefore: Long = Long.MIN_VALUE): RootSnapshot = awaitValue(refresh = true, notBefore = notBefore)

    override suspend fun load(request: ObservationRequest): RootSnapshot =
        joinRootSnapshot(loadRootShellSnapshot(), CoreSessions.updates.value, request.id, request.generation)

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
internal fun loadRootShellSnapshot(recordMetric: (String, Long) -> Unit = StartupTrace::metric): Map<String, String> {
    val diagnostics = CoreRoot.collectBlocking(listOf(CoreProbe.RootSnapshot(packages = true)))
    startupProbeFailure(diagnostics)?.let { (kind, detail) -> throw RootSnapshotException(detail, kind) }
    val result = diagnostics.results(1).first()
    val sections = parseRootShellSnapshot(result.text, recordMetric).toMutableMap()
    validateRootSnapshotSections(sections)
    return sections
}

internal fun validateRootSnapshotSections(sections: Map<String, String>) {
    val validation = coreValidateRootSnapshotSections(sections.keys.toList())
    validation.error?.let { throw RootSnapshotException("root snapshot framing invalid: ${it.name}") }
    val missing = validation.missing
    if (missing.isNotEmpty()) {
        throw RootSnapshotException("root snapshot incomplete, missing sections: ${missing.joinToString()}")
    }
}

internal fun parseRootShellSnapshot(
    raw: String,
    recordMetric: (String, Long) -> Unit = StartupTrace::metric,
): Map<String, String> {
    val parsed = coreParseProbeSections(CoreProbeFrameKind.ROOT, raw)
    parsed.error?.let { throw RootSnapshotException("root snapshot framing invalid: ${it.name}") }
    parsed.timings.forEach { recordMetric("root_shell_${it.name}", it.durationMs) }
    return parsed.complete.associate { it.name to it.body }
}

/** Seeded startup reads and new probes join through this same typed provenance boundary. */
internal fun joinRootSnapshot(
    sections: Map<String, String>,
    core: CoreSessionUpdate,
    observationId: Long = 0,
    generation: Long = 0,
): RootSnapshot = RootSnapshot(sections.toMap(), observationId, generation, core)
