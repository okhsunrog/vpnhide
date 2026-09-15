package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import android.os.Process
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.ConfigOperationObserver
import dev.okhsunrog.vpnhide.ConfigOperationResult
import dev.okhsunrog.vpnhide.ConfigOperationSpec
import dev.okhsunrog.vpnhide.ConfigPhase
import dev.okhsunrog.vpnhide.ContextObservationInputs
import dev.okhsunrog.vpnhide.ObservationRuntime
import dev.okhsunrog.vpnhide.ProjectedStateFlow
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.TransitionFailure
import dev.okhsunrog.vpnhide.currentObservationValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn

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
        val reason: TransitionFailure,
    ) : DiagnosticCaptureOutcome
}

/**
 * Facade over the process-owned diagnostic run coordinator.
 *
 * Diagnostics answer one question: *did the hooks work for this app process in a
 * measured run?* Each run is an identified, immutable attempt executed by
 * [DiagnosticRunCoordinator] on the process scope, so leaving a screen or
 * recreating the Activity never cancels or restarts a suite (a caller's scope is
 * accepted for source compatibility only). [state] is a synchronous projection
 * of the coordinator's view onto the legacy vocabulary every consumer renders:
 *
 * - [State.NotRun] — no attempt has finished yet and none is active.
 * - [State.Running] — a run is waiting for a relevant config operation, checking
 *   eligibility or probing its core phase.
 * - [State.Blocked] — the latest attempt found the suite not eligible (VPN off,
 *   this app split-tunnelled out, or a pending self-restart); carries the
 *   [DiagnosticGate] so the banner explains which.
 * - [State.Failed] — the latest attempt could not measure: execution failure,
 *   deadline, cancellation, a config operation that failed or stayed unresolved,
 *   or a context change during the run. Distinct from a VPN-off gate so an
 *   active-VPN user is not told their VPN is off.
 * - [State.Ready] — evidence exists; [State.Ready.complete] is false while the
 *   slow Java phase of the active run is still filling in.
 *
 * [run] is the automatic intent: it starts a suite only until one has actually
 * probed, and a blocked attempt does not consume it. [retry] keeps the existing
 * policy — a completed suite is reused, anything else is requested again as a
 * new run. Neither observation refreshes nor recomposition rerun a completed suite.
 * [captureRun] is the debug export's entry: an explicit, capture-identified run
 * that never joins an existing suite and whose terminal attempt is reported as is.
 *
 * Config operations reach the suite through [configOperation]: a request depends
 * on every accepted operation that can change this process's own measurement
 * (its roles and hooks, global optional features, whole replacements) and waits
 * for them; the first mutating dispatch of such an operation interrupts an active
 * run and advances the change epoch. Relevance is decided again on the prepared
 * write set, so a mutation that only carries a transform still counts once its
 * candidate is known. Other apps' edits, debug logging and the startup runtime
 * reconcile neither delay nor interrupt a suite.
 */
internal object DiagnosticsCache {
    sealed interface State {
        data object NotRun : State

        data object Running : State

        // Never [DiagnosticGate.ROUTED] — that outcome is a measured [Ready].
        data class Blocked(
            val gate: DiagnosticGate,
        ) : State {
            init {
                require(gate != DiagnosticGate.ROUTED) { "Blocked gate must not be ROUTED" }
            }
        }

        data object Failed : State

        data class Ready(
            val results: CheckResults,
            val complete: Boolean,
        ) : State
    }

    @Volatile private var inputs: ContextObservationInputs? = null

    // Whether this app's own hooks need a restart to apply (it was just added as a
    // target). Process-constant, so it is sticky-OR: once any caller reports true,
    // a caller that does not know it (the agent bridge) can safely pass false.
    @Volatile private var restartPending = false

    private val impactLock = Any()
    private val impactFlow = MutableStateFlow(DiagnosticImpactState())
    private val impact: DiagnosticImpactState get() = impactFlow.value

    private val coordinator by lazy {
        DiagnosticRunCoordinator(ObservationRuntime.scope, AppDiagnosticRunIo(inputs = { inputs }, impact = { impact }))
    }

    /** The identified run state: active run, latest attempt, latest complete measurement and their evidence. */
    val runs: StateFlow<DiagnosticRunView> get() = coordinator.view

    val state: StateFlow<State> by lazy { ProjectedStateFlow(coordinator.view, ::projectDiagnosticState) }

    /**
     * The shared projection every consumer should render: one value per change of
     * the run view, the routing observation, the root snapshot, the confirmed
     * config or the operation impact, so eligibility, the selected measurement and
     * its applicability always come from the same instant.
     */
    val presentation: StateFlow<DiagnosticPresentation> by lazy {
        combine(
            coordinator.view,
            RoutingGateCache.observation,
            RootSnapshotCache.snapshot,
            CanonicalConfigRepository.state,
            impactFlow,
        ) { view, routing, snapshot, config, impact ->
            val current = inputs
            val observation =
                if (routing.attempted) {
                    buildDiagnosticContextObservation(
                        selfNeedsRestart = current?.selfNeedsRestart ?: false,
                        routing = routing,
                        snapshot = snapshot,
                        config = config.confirmed,
                        selfPackage = current?.context?.packageName.orEmpty(),
                        processIdentity = processIdentity(),
                        now = System.currentTimeMillis(),
                        readiness = configReadiness(impact),
                        changeEpoch = impact.changeEpoch,
                        initialized = current != null,
                    )
                } else {
                    null
                }
            diagnosticPresentation(view, observation, impact.changeEpoch, uncertain = currentObservationValue(routing) == null)
        }.stateIn(
            ObservationRuntime.scope,
            SharingStarted.Eagerly,
            diagnosticPresentation(coordinator.view.value, null, 0, uncertain = true),
        )
    }

    private fun processIdentity(): String = "pid:${Process.myPid()};uid:${Process.myUid()}"

    /** Automatic suite request: idempotent, consumed by the first suite that actually probes. */
    fun run(
        @Suppress("UNUSED_PARAMETER") scope: CoroutineScope,
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        updateInputs(context, selfNeedsRestart)
        coordinator.request(request(automatic = true))
    }

    /** Explicit retry from the VPN-off / failed banners and Dashboard refresh: a new run unless the last one completed. */
    fun retry(
        @Suppress("UNUSED_PARAMETER") scope: CoroutineScope,
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        updateInputs(context, selfNeedsRestart)
        // A completed suite is reused unless a known change made its measurement inapplicable.
        val changed = presentation.value.applicability == MeasurementApplicability.Changed
        if (changed || diagnosticRetryAllowed(coordinator.view.value.core)) coordinator.request(request(automatic = false))
    }

    /**
     * Suspend until a terminal attempt is available: the active run's own result,
     * the latest finished attempt, or the automatic suite when nothing ran yet. A
     * terminal Blocked/Failed attempt is returned as is; retry belongs to an
     * explicit trigger, so a dependent derivation cannot form a refresh cycle.
     */
    suspend fun awaitTerminal(
        context: Context,
        selfNeedsRestart: Boolean,
    ): State {
        updateInputs(context, selfNeedsRestart)
        val handle = coordinator.ensure(request(automatic = true)) ?: return state.value
        val result = handle.await()
        return projectDiagnosticAttempt(result.attempt, result.results)
    }

    /**
     * A capture's own fresh, identified run. [captureId] is part of the request
     * identity, so a capture can never join a suite whose probes began before its
     * logging and counter baseline (§9); it is admitted as a pending run instead
     * and waits for the active one to settle. The terminal attempt is returned as
     * is — blocked, interrupted and failed included — because a capture records
     * the outcome rather than retrying it.
     */
    suspend fun captureRun(
        context: Context,
        selfNeedsRestart: Boolean,
        captureId: Long,
    ): DiagnosticCaptureOutcome {
        updateInputs(context, selfNeedsRestart)
        return when (val admission = coordinator.request(request(automatic = false).copy(captureId = captureId))) {
            is DiagnosticAdmission.Accepted -> DiagnosticCaptureOutcome.Ran(admission.handle.await())

            is DiagnosticAdmission.Rejected -> DiagnosticCaptureOutcome.NotAdmitted(admission.reason)

            // Only an automatic request can be ignored; a capture request never is.
            DiagnosticAdmission.Ignored -> DiagnosticCaptureOutcome.NotAdmitted(TransitionFailure.Busy)
        }
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

    private fun request(automatic: Boolean): DiagnosticRequest = diagnosticRequest(automatic).copy(dependencies = impact.relevant)

    private fun updateInputs(
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        restartPending = restartPending || selfNeedsRestart
        inputs = ContextObservationInputs(context.applicationContext, restartPending)
    }
}

/** The config coordinator's observer: classifies each operation against this app's own measurement. */
internal class DiagnosticImpactObserver(
    private val selfPackage: () -> String,
) : ConfigOperationObserver {
    override fun accepted(
        id: Long,
        spec: ConfigOperationSpec,
    ) = DiagnosticsCache.configOperation(DiagnosticImpactEvent.Accepted(id, operationAffectsSelfMeasurement(spec, selfPackage())))

    override fun prepared(
        id: Long,
        spec: ConfigOperationSpec,
    ) = DiagnosticsCache.configOperation(DiagnosticImpactEvent.Prepared(id, operationAffectsSelfMeasurement(spec, selfPackage())))

    override fun dispatched(
        id: Long,
        phase: ConfigPhase,
    ) = DiagnosticsCache.configOperation(DiagnosticImpactEvent.Dispatched(id, phase))

    override fun settled(result: ConfigOperationResult) =
        DiagnosticsCache.configOperation(DiagnosticImpactEvent.Settled(result.id, result.failure))

    override fun recovered(result: ConfigOperationResult) =
        DiagnosticsCache.configOperation(DiagnosticImpactEvent.Recovered(result.id, result.failure))
}
