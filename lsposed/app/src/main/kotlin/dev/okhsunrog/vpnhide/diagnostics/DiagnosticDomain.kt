package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ConfigCoordinatorView
import dev.okhsunrog.vpnhide.ObservationClock
import dev.okhsunrog.vpnhide.ObservationState
import dev.okhsunrog.vpnhide.ProjectedStateFlow
import dev.okhsunrog.vpnhide.RootSnapshot
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/** What this process knows about itself that the measurement context needs. */
internal data class DiagnosticInputs(
    val selfPackage: String,
    val selfNeedsRestart: Boolean,
)

/**
 * What a capture got out of the suite. A capture never renders the legacy
 * projection: it records the identified attempt (or the reason it never got one)
 * in the bundle, so a bundle can say "the run was interrupted" instead of
 * silently carrying no report.
 */
internal sealed interface DiagnosticCaptureOutcome {
    /** The capture's own run reached a terminal attempt; [result] carries it with whatever evidence exists. */
    data class Ran(
        val result: DiagnosticRunResult,
    ) : DiagnosticCaptureOutcome

    /** No run was admitted (another suite owns the lane, or the probe resource is quarantined). */
    data class NotAdmitted(
        val reason: dev.okhsunrog.vpnhide.TransitionFailure,
    ) : DiagnosticCaptureOutcome
}

/**
 * The process-owned diagnostic domain, wired from the observations it folds.
 *
 * Diagnostics answer one question: *did the hooks work for this app process in a
 * measured run?* Each run is an identified, immutable attempt executed by
 * [DiagnosticRunCoordinator] on [scope], so leaving a screen or recreating the
 * Activity never cancels or restarts a suite. Every consumer renders one
 * immutable [snapshot], carrying accepted inputs, [presentation] and [situation].
 * Run transitions publish it before releasing their handles; bridge and export
 * barriers also wait for already-published observation events to be accepted.
 *
 * A suite starts in exactly three ways. [run] is the startup intent: the first
 * suite of the process, a join or a read afterwards. [retry] is the explicit run
 * the user asked for: always a new one. Every other automatic run is owed by the
 * presentation itself ([owedConfirmation]): when this app is eligible and the key
 * of its measurable world is covered by no measurement, no attempt and no earlier
 * request, one confirmation is requested for that key, and the presentation says
 * so ([DiagnosticPresentation.confirmationPending]) until it is. Recomposition,
 * timer ticks and re-reads that reveal no new key rerun nothing.
 *
 * Config operations reach the suite through [configOperation]: a request depends
 * on every accepted operation that can change this process's own measurement and
 * waits for them; the first mutating dispatch of such an operation interrupts an
 * active run and advances the change epoch.
 *
 * Everything the domain reads is a constructor input, so a test drives it with
 * plain state flows and a fake [DiagnosticRunIo]; the production wiring lives in
 * [DiagnosticsCache].
 */
internal class DiagnosticDomain(
    scope: CoroutineScope,
    io: DiagnosticRunIo,
    private val routing: StateFlow<ObservationState<AppVpnStateSnapshot>>,
    private val snapshots: StateFlow<ObservationState<RootSnapshot>>,
    private val config: StateFlow<ConfigCoordinatorView>,
    private val inputs: StateFlow<DiagnosticInputs?>,
    private val processIdentity: String,
    clock: () -> Long = ObservationClock::now,
    private val wallClock: () -> Long = System::currentTimeMillis,
    wait: suspend (Long) -> Unit = { delay(it) },
    private val log: (String) -> Unit = {},
) {
    private val lock = Any()
    private val owner = DiagnosticSnapshotOwner(lock, processIdentity, clock, wallClock)
    val snapshot: StateFlow<DiagnosticSnapshot> = owner.snapshot
    val impact: DiagnosticImpactState get() = snapshot.value.sources.impact
    val presentation: StateFlow<DiagnosticPresentation> = ProjectedStateFlow(snapshot) { it.presentation }
    val situation: StateFlow<Situation> = ProjectedStateFlow(snapshot) { it.situation }

    private val coordinator =
        DiagnosticRunCoordinator(scope, io, lock = lock, onPublished = { view -> owner.update { it.copy(runs = view) } })

    init {
        owner.watch(scope, routing) { state, value -> state.copy(routing = value) }
        owner.watch(scope, snapshots) { state, value -> state.copy(root = value) }
        owner.watch(scope, config) { state, value -> state.copy(config = value) }
        owner.watch(scope, inputs) { state, value -> state.copy(inputs = value) }
        owner.watchGrace(scope, wait)
        scope.launch { confirmMeasurements(presentation, ::requestConfirmation, clock, wait) }
    }

    /** No fresh I/O: await acceptance of a requested root observation, then return one whole revision. */
    suspend fun awaitSnapshot(root: RootSnapshot? = null): DiagnosticSnapshot =
        snapshot.first { candidate ->
            synchronized(lock) {
                candidate === snapshot.value &&
                    (root == null || candidate.includesRoot(root)) &&
                    sourcesAccepted(candidate.sources)
            }
        }

    // A barrier for already-published source events, not a second assembly from
    // independent .value reads. If a collector lags, only its owner publication
    // can satisfy the barrier. Superseding events may be conflated normally.
    private fun sourcesAccepted(sources: DiagnosticSnapshotSources): Boolean =
        sources.routing == routing.value && sources.root == snapshots.value &&
            sources.config == config.value && sources.inputs == inputs.value

    /** Startup intent: starts the first suite of the process; later calls join or read what exists. */
    fun run(): DiagnosticRunHandle? = synchronized(lock) { coordinator.ensure(request(automatic = true)) }

    /**
     * Explicit re-check: always a new run. The user asked for a fresh verdict and the
     * hero says "Running the hiding checks…" while it runs, so reusing a completed
     * suite would promise a measurement and deliver the old one. A run already in
     * flight is joined by the coordinator; nothing here reruns a suite at rest (I16).
     */
    fun retry() {
        synchronized(lock) { coordinator.request(request(automatic = false)) }
    }

    /**
     * Suspend until a terminal attempt is available — the active run's own result,
     * the latest finished attempt, or the startup suite when nothing ran yet — and
     * return the shared [snapshot]. A terminal blocked or failed attempt is
     * returned as is; retry belongs to an explicit trigger. The publication
     * precedes handle completion, and [awaitSnapshot] accepts queued source events.
     */
    suspend fun awaitTerminal(root: RootSnapshot? = null): DiagnosticSnapshot {
        run()?.await()
        // Run publication and the packet are synchronous; no asynchronous UI projection to await.
        return awaitSnapshot(root)
    }

    /**
     * A capture's own fresh, identified run. [captureId] is part of the request
     * identity, so a capture can never join a suite whose probes began before its
     * logging and counter baseline (§9); it is admitted as a pending run instead
     * and waits for the active one to settle. The terminal attempt is returned as
     * is — blocked, interrupted and failed included — because a capture records
     * the outcome rather than retrying it.
     */
    suspend fun captureRun(captureId: Long): DiagnosticCaptureOutcome =
        when (val admission = synchronized(lock) { coordinator.request(request(automatic = false).copy(captureId = captureId)) }) {
            is DiagnosticAdmission.Accepted -> DiagnosticCaptureOutcome.Ran(admission.handle.await())
            is DiagnosticAdmission.Rejected -> DiagnosticCaptureOutcome.NotAdmitted(admission.reason)
        }

    /** Config-operation lifecycle from the coordinator's observer; effects go to the run coordinator in order. */
    fun configOperation(event: DiagnosticImpactEvent) {
        synchronized(lock) {
            val transition = reduceDiagnosticImpact(impact, event)
            owner.stage { it.copy(impact = transition.state) }
            transition.effects.forEach { effect ->
                when (effect) {
                    is DiagnosticImpactEffect.DelayRuns -> coordinator.operationAccepted(effect.id)
                    DiagnosticImpactEffect.InterruptRuns -> coordinator.contextChanged(known = true)
                    is DiagnosticImpactEffect.SettleRuns -> coordinator.operationSettled(effect.id, effect.failure)
                }
            }
            owner.publish()
        }
    }

    /**
     * One automatic confirmation for [key]: admitted, or joined when a suite with
     * the same plan is already in flight. Not admitted only while a capture's own
     * run holds the probes, and the owner asks again on the next presentation.
     */
    private fun requestConfirmation(key: MeasurementKey): Boolean =
        synchronized(lock) {
            // The settle timer may have captured a key superseded by another accepted event.
            if (owedConfirmation(presentation.value, snapshot.value.sources.claimed) != key) return@synchronized false
            val admission = coordinator.request(request(automatic = true))
            log("confirmation for ${key.routing}: $admission")
            val accepted = admission is DiagnosticAdmission.Accepted
            if (accepted) owner.update { it.copy(claimed = key) }
            accepted
        }

    private fun request(automatic: Boolean): DiagnosticRequest = diagnosticRequest(automatic).copy(dependencies = impact.relevant)
}
