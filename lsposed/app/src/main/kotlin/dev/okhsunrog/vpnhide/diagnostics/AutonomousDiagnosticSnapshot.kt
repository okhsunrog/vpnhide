package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ConfigCoordinatorView
import dev.okhsunrog.vpnhide.CoreSessionUpdate
import dev.okhsunrog.vpnhide.ObservationClock
import dev.okhsunrog.vpnhide.ObservationState
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.currentObservationValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

internal enum class AutonomousAvailability {
    Initializing,
    Offline,
    ProtocolSkew,
    ConfigUnjoined,
    NoRun,
    EpochMismatch,
    IncompletePublication,
    Published,
}

/** Accepted sources are provenance, never authority to launch another suite. */
internal data class AutonomousDiagnosticSources(
    val core: CoreSessionUpdate,
    val root: ObservationState<RootSnapshot>,
    val config: ConfigCoordinatorView,
)

internal data class AutonomousDiagnosticSnapshot(
    val revision: Long,
    val observedAt: Long,
    val sources: AutonomousDiagnosticSources,
    val availability: AutonomousAvailability,
    val run: AutonomousRunInfo?,
    val assessment: AutonomousAssessmentInfo?,
    val rootJoined: Boolean,
) {
    val rootSnapshot: RootSnapshot? get() = sources.root.lastGood?.value

    /** Freshness of this scoped report; it never means complete protection. */
    val current: Boolean get() = availability == AutonomousAvailability.Published && run?.stale == false

    fun includesRoot(root: RootSnapshot): Boolean =
        rootSnapshot?.let { it.generation >= root.generation && it.observationId >= root.observationId } == true
}

internal fun autonomousDiagnosticSnapshot(
    sources: AutonomousDiagnosticSources,
    revision: Long,
    now: Long,
): AutonomousDiagnosticSnapshot {
    val update = sources.core
    val state = update.state
    val run = state?.autonomousDiagnostics
    val availability =
        when {
            update.protocolSkew -> AutonomousAvailability.ProtocolSkew

            !update.current && state == null && update.sequence == 0L && update.failureDetail == null -> AutonomousAvailability.Initializing

            !update.current || state == null -> AutonomousAvailability.Offline

            !coreInputsJoined(
                update,
                sources.config,
            ) || sources.config.coreSequence != update.sequence -> AutonomousAvailability.ConfigUnjoined

            run == null -> AutonomousAvailability.NoRun

            run.coreEpoch.toLong() != state.epoch -> AutonomousAvailability.EpochMismatch

            state.autonomousAssessment == null -> AutonomousAvailability.IncompletePublication

            else -> AutonomousAvailability.Published
        }
    return AutonomousDiagnosticSnapshot(
        revision,
        now,
        sources,
        availability,
        run?.toDiagnosticInfo(),
        state?.autonomousAssessment?.toDiagnosticInfo(),
        currentRootJoined(sources),
    )
}

private fun currentRootJoined(sources: AutonomousDiagnosticSources): Boolean {
    val root = currentObservationValue(sources.root) ?: return false
    val update = sources.core
    val state = update.state ?: return false
    val rootState = root.core.state ?: return false
    // Compare only facts this heavy root projection actually consumed. Routing,
    // receipts and process telemetry have their own live Core publications.
    return update.current && root.core.current && rootState.epoch == state.epoch &&
        root.core.connectionGeneration == update.connectionGeneration &&
        rootState.configRevision == state.configRevision && coreInputsJoined(root, sources.config) &&
        rootState.canonical == state.canonical && rootState.backend == state.backend && rootState.nativeBoot == state.nativeBoot
}

/** One publication owner; construction, reads and input changes perform no RPC. */
internal class AutonomousDiagnosticSnapshotOwner(
    scope: CoroutineScope,
    private val core: StateFlow<CoreSessionUpdate>,
    private val root: StateFlow<ObservationState<RootSnapshot>>,
    private val config: StateFlow<ConfigCoordinatorView>,
    private val clock: () -> Long = ObservationClock::now,
) {
    private fun sources() = AutonomousDiagnosticSources(core.value, root.value, config.value)

    private val mutable = MutableStateFlow(autonomousDiagnosticSnapshot(sources(), 0, clock()))
    val snapshot: StateFlow<AutonomousDiagnosticSnapshot> = mutable.asStateFlow()

    init {
        scope.launch(start = CoroutineStart.UNDISPATCHED) {
            combine(core, root, config, ::AutonomousDiagnosticSources).collect { sources ->
                if (sources != mutable.value.sources) {
                    mutable.value = autonomousDiagnosticSnapshot(sources, mutable.value.revision + 1, clock())
                }
            }
        }
    }

    suspend fun awaitSnapshot(requestedRoot: RootSnapshot? = null): AutonomousDiagnosticSnapshot =
        snapshot.first { candidate ->
            candidate === snapshot.value && candidate.sources == sources() &&
                (!candidate.sources.core.current || requestedRoot == null || candidate.includesRoot(requestedRoot))
        }
}
