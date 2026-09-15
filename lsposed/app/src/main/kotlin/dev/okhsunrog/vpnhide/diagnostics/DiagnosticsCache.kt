package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import dev.okhsunrog.vpnhide.ContextObservationInputs
import dev.okhsunrog.vpnhide.ObservationRuntime
import dev.okhsunrog.vpnhide.ProjectedStateFlow
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.StateFlow

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
 * - [State.Running] — a run is checking eligibility or probing its core phase.
 * - [State.Blocked] — the latest attempt found the suite not eligible (VPN off,
 *   this app split-tunnelled out, or a pending self-restart); carries the
 *   [DiagnosticGate] so the banner explains which.
 * - [State.Failed] — the latest attempt could not measure: execution failure,
 *   deadline, cancellation or a context change during the run. Distinct from a
 *   VPN-off gate so an active-VPN user is not told their VPN is off.
 * - [State.Ready] — evidence exists; [State.Ready.complete] is false while the
 *   slow Java phase of the active run is still filling in.
 *
 * [run] is the automatic intent: it starts a suite only until one has actually
 * probed, and a blocked attempt does not consume it. [retry] keeps the existing
 * policy — a completed suite is reused, anything else is requested again as a
 * new run. Neither observation refreshes nor recomposition rerun a completed suite.
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

    private val coordinator by lazy { DiagnosticRunCoordinator(ObservationRuntime.scope, AppDiagnosticRunIo(inputs = { inputs })) }

    /** The identified run state: active run, latest attempt, latest complete measurement and their evidence. */
    val runs: StateFlow<DiagnosticRunView> get() = coordinator.view

    val state: StateFlow<State> by lazy { ProjectedStateFlow(coordinator.view, ::projectDiagnosticState) }

    /** Automatic suite request: idempotent, consumed by the first suite that actually probes. */
    fun run(
        @Suppress("UNUSED_PARAMETER") scope: CoroutineScope,
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        updateInputs(context, selfNeedsRestart)
        coordinator.request(diagnosticRequest(automatic = true))
    }

    /** Explicit retry from the VPN-off / failed banners and Dashboard refresh: a new run unless the last one completed. */
    fun retry(
        @Suppress("UNUSED_PARAMETER") scope: CoroutineScope,
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        updateInputs(context, selfNeedsRestart)
        if (diagnosticRetryAllowed(coordinator.view.value.core)) coordinator.request(diagnosticRequest())
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
        val handle = coordinator.ensure(diagnosticRequest(automatic = true)) ?: return state.value
        val result = handle.await()
        return projectDiagnosticAttempt(result.attempt, result.results)
    }

    private fun updateInputs(
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        restartPending = restartPending || selfNeedsRestart
        inputs = ContextObservationInputs(context.applicationContext, restartPending)
    }
}
