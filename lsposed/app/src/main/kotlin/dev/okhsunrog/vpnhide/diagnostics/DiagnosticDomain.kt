package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ConfigCoordinatorView
import dev.okhsunrog.vpnhide.ObservationClock
import dev.okhsunrog.vpnhide.ObservationState
import dev.okhsunrog.vpnhide.RootSnapshot
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.stateIn
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
 * projection, [presentation]: the screens collect it, the Dashboard state is
 * projected from it, the bridge reads it once it reflects a terminal attempt
 * ([awaitTerminal]), the bundle summarises it.
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
    routing: StateFlow<ObservationState<AppVpnStateSnapshot>>,
    snapshots: StateFlow<RootSnapshot?>,
    config: StateFlow<ConfigCoordinatorView>,
    inputs: StateFlow<DiagnosticInputs?>,
    private val processIdentity: String,
    clock: () -> Long = ObservationClock::now,
    private val wallClock: () -> Long = System::currentTimeMillis,
    wait: suspend (Long) -> Unit = { delay(it) },
    private val log: (String) -> Unit = {},
) {
    private val impactLock = Any()
    private val impactFlow = MutableStateFlow(DiagnosticImpactState())
    val impact: DiagnosticImpactState get() = impactFlow.value

    /** The key the confirmation owner last requested a run for; part of the presentation, so "pending" is one fact. */
    private val claimedKey = MutableStateFlow<MeasurementKey?>(null)

    private val coordinator = DiagnosticRunCoordinator(scope, io)

    /**
     * The shared projection every consumer should render: one value per change of
     * the run view, the routing observation, the root snapshot, the confirmed
     * config, the operation impact, the claimed key or the inputs, so eligibility,
     * the selected measurement, its applicability and the pending confirmation
     * always come from the same instant.
     */
    val presentation: StateFlow<DiagnosticPresentation> =
        combine(
            coordinator.view,
            routing,
            snapshots,
            config,
            combine(impactFlow, claimedKey, inputs) { impact, claimed, current -> Triple(impact, claimed, current) },
        ) { view, appVpnState, snapshot, configView, (impact, claimed, current) ->
            val gate = appVpnState.gateProjection()
            val observation =
                if (gate.attempted) {
                    buildDiagnosticContextObservation(
                        selfNeedsRestart = current?.selfNeedsRestart ?: false,
                        appVpn = AppVpnContext(gate, appVpnState.lastGood?.value?.identity),
                        snapshot = snapshot,
                        config = configView.confirmed,
                        selfPackage = current?.selfPackage.orEmpty(),
                        processIdentity = processIdentity,
                        now = wallClock(),
                        readiness = configReadiness(impact),
                        changeEpoch = impact.changeEpoch,
                        initialized = current != null,
                    )
                } else {
                    null
                }
            val knowledge = routingKnowledge(selfRoutingObservation(gate), clock())
            diagnosticPresentation(view, observation, impact.changeEpoch, knowledge, claimed)
        }.stateIn(
            scope,
            SharingStarted.Eagerly,
            diagnosticPresentation(
                coordinator.view.value,
                null,
                0,
                routingKnowledge(selfRoutingObservation(routing.value.gateProjection()), clock()),
            ),
        )

    /**
     * The one classification the surfaces render: [presentation] folded into a
     * [Situation], plus the single re-emission that ends a Background grace. It
     * words the presentation and never triggers a read or a run (I16).
     */
    val situation: StateFlow<Situation> =
        situationFlow(presentation, clock, wait).stateIn(scope, SharingStarted.Eagerly, situation(presentation.value, clock()))

    init {
        // The one owner of automatic confirmations, process-lived like the runs it requests.
        scope.launch { confirmMeasurements(presentation, ::requestConfirmation, clock, wait) }
    }

    /** Startup intent: starts the first suite of the process; later calls join or read what exists. */
    fun run(): DiagnosticRunHandle? = coordinator.ensure(request(automatic = true))

    /**
     * Explicit re-check: always a new run. The user asked for a fresh verdict and the
     * hero says "Running the hiding checks…" while it runs, so reusing a completed
     * suite would promise a measurement and deliver the old one. A run already in
     * flight is joined by the coordinator; nothing here reruns a suite at rest (I16).
     */
    fun retry() {
        coordinator.request(request(automatic = false))
    }

    /**
     * Suspend until a terminal attempt is available — the active run's own result,
     * the latest finished attempt, or the startup suite when nothing ran yet — and
     * return the shared [presentation] once it reflects that attempt. A terminal
     * blocked or failed attempt is returned as is; retry belongs to an explicit
     * trigger. Attempt ids are monotonic and runs finish in admission order, so a
     * presentation whose latest attempt id is at least the awaited run's id has
     * that run finished.
     */
    suspend fun awaitTerminal(): DiagnosticPresentation {
        val handle = run() ?: return presentation.value
        handle.await()
        return presentation.first { (it.lastAttempt?.id ?: 0) >= handle.id }
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
        when (val admission = coordinator.request(request(automatic = false).copy(captureId = captureId))) {
            is DiagnosticAdmission.Accepted -> DiagnosticCaptureOutcome.Ran(admission.handle.await())
            is DiagnosticAdmission.Rejected -> DiagnosticCaptureOutcome.NotAdmitted(admission.reason)
        }

    /** Config-operation lifecycle from the coordinator's observer; effects go to the run coordinator in order. */
    fun configOperation(event: DiagnosticImpactEvent) {
        val transition = synchronized(impactLock) { reduceDiagnosticImpact(impact, event).also { impactFlow.value = it.state } }
        transition.effects.forEach { effect ->
            when (effect) {
                is DiagnosticImpactEffect.DelayRuns -> coordinator.operationAccepted(effect.id)
                DiagnosticImpactEffect.InterruptRuns -> coordinator.contextChanged(known = true)
                is DiagnosticImpactEffect.SettleRuns -> coordinator.operationSettled(effect.id, effect.failure)
            }
        }
    }

    /**
     * One automatic confirmation for [key]: admitted, or joined when a suite with
     * the same plan is already in flight. Not admitted only while a capture's own
     * run holds the probes, and the owner asks again on the next presentation.
     */
    private fun requestConfirmation(key: MeasurementKey): Boolean {
        val admission = coordinator.request(request(automatic = true))
        log("confirmation for ${key.routing}: $admission")
        val accepted = admission is DiagnosticAdmission.Accepted
        if (accepted) claimedKey.value = key
        return accepted
    }

    private fun request(automatic: Boolean): DiagnosticRequest = diagnosticRequest(automatic).copy(dependencies = impact.relevant)
}
