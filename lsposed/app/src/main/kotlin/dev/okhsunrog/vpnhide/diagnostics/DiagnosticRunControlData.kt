package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.Transition
import dev.okhsunrog.vpnhide.TransitionFailure

internal fun interruptRunForContext(
    state: DiagnosticRunState,
    known: Boolean,
    now: Long,
): Transition<DiagnosticRunState, DiagnosticRunEffect> {
    val active = state.active ?: return Transition(state)
    val reason = if (known) TransitionFailure.ContextChanged else TransitionFailure.ContextUnknown
    return when (active.stage) {
        DiagnosticStage.Waiting, DiagnosticStage.Draining -> Transition(state)
        DiagnosticStage.Checking -> finishDiagnosticRun(state, RunOutcome.NotStarted, reason, now)
        else -> beginDiagnosticDrain(state, RunOutcome.Interrupted, reason)
    }
}

/** A blocked Checking observation is a terminal NotStarted attempt; it neither probes nor consumes the automatic intent. */
internal fun blockDiagnosticRun(
    state: DiagnosticRunState,
    event: DiagnosticRunEvent.NotEligible,
    now: Long,
): Transition<DiagnosticRunState, DiagnosticRunEffect> {
    val active = state.active ?: return Transition(state)
    if (active.ticket != event.ticket || active.stage != DiagnosticStage.Checking) return Transition(state)
    return finishDiagnosticRun(state, RunOutcome.NotStarted, failure = null, now = now, eligibility = event.eligibility)
}

internal fun beginDiagnosticDrain(
    state: DiagnosticRunState,
    outcome: RunOutcome,
    reason: TransitionFailure,
): Transition<DiagnosticRunState, DiagnosticRunEffect> =
    dispatchDiagnosticStage(
        state.copy(active = requireNotNull(state.active).copy(ending = outcome, failure = reason)),
        DiagnosticStage.Draining,
    )

internal fun failDiagnosticRun(
    state: DiagnosticRunState,
    event: DiagnosticRunEvent.Failed,
    now: Long,
): Transition<DiagnosticRunState, DiagnosticRunEffect> {
    val active = state.active ?: return Transition(state)
    if (active.ticket != event.ticket) return Transition(state)
    return when (active.stage) {
        DiagnosticStage.Waiting -> Transition(state)
        DiagnosticStage.Checking -> finishDiagnosticRun(state, RunOutcome.NotStarted, event.reason, now)
        DiagnosticStage.Draining -> drainDiagnosticRun(state, DiagnosticRunEvent.Drained(event.ticket, false), now)
        DiagnosticStage.Verifying -> finishDiagnosticRun(state, RunOutcome.Interrupted, TransitionFailure.ContextUnknown, now)
        else -> beginDiagnosticDrain(state, RunOutcome.Failed, event.reason)
    }
}

internal fun drainDiagnosticRun(
    state: DiagnosticRunState,
    event: DiagnosticRunEvent.Drained,
    now: Long,
): Transition<DiagnosticRunState, DiagnosticRunEffect> {
    val active = state.active ?: return Transition(state)
    if (active.stage != DiagnosticStage.Draining || active.ticket != event.ticket) return Transition(state)
    return finishDiagnosticRun(
        state.copy(quarantined = !event.quiescent, quarantineTicket = if (event.quiescent) null else event.ticket),
        active.ending,
        active.failure,
        now,
    )
}

internal fun stopRequestedRun(
    state: DiagnosticRunState,
    id: Long,
    reason: TransitionFailure,
    now: Long,
): Transition<DiagnosticRunState, DiagnosticRunEffect> {
    if (state.pending?.id == id) {
        val outcome = if (reason == TransitionFailure.Cancelled) RunOutcome.Interrupted else RunOutcome.NotStarted
        return Transition(
            state.copy(pending = null),
            listOf(DiagnosticRunEffect.Completed(DiagnosticAttempt(id, outcome, reason))),
        )
    }
    val active = state.active ?: return Transition(state)
    if (active.id != id) return Transition(state)
    return when (active.stage) {
        DiagnosticStage.Waiting, DiagnosticStage.Checking -> {
            val outcome = if (reason == TransitionFailure.Cancelled) RunOutcome.Interrupted else RunOutcome.NotStarted
            finishDiagnosticRun(state, outcome, reason, now)
        }

        DiagnosticStage.Draining -> {
            Transition(state)
        }

        else -> {
            beginDiagnosticDrain(state, RunOutcome.Interrupted, reason)
        }
    }
}

internal fun settleRunDependency(
    state: DiagnosticRunState,
    event: DiagnosticRunEvent.OperationSettled,
    now: Long,
): Transition<DiagnosticRunState, DiagnosticRunEffect> {
    val pending = state.pending
    var next = state
    val effects = mutableListOf<DiagnosticRunEffect>()
    if (pending != null && event.id in pending.request.dependencies) {
        if (event.failure != null) {
            next = state.copy(pending = null)
            effects += DiagnosticRunEffect.Completed(DiagnosticAttempt(pending.id, RunOutcome.NotStarted, event.failure))
        } else {
            next = state.copy(pending = removeRunDependency(pending, event.id))
        }
    }
    val active = next.active
    if (active == null || active.stage != DiagnosticStage.Waiting ||
        event.id !in active.request.dependencies
    ) {
        return Transition(next, effects)
    }
    val result =
        if (event.failure != null) {
            finishDiagnosticRun(next, RunOutcome.NotStarted, event.failure, now)
        } else {
            val updated = removeRunDependency(active, event.id)
            if (updated.request.dependencies.isEmpty()) {
                dispatchDiagnosticStage(
                    next.copy(active = updated),
                    DiagnosticStage.Checking,
                )
            } else {
                Transition(next.copy(active = updated))
            }
        }
    return result.copy(effects = effects + result.effects)
}

private fun removeRunDependency(
    run: ActiveDiagnosticRun,
    id: Long,
): ActiveDiagnosticRun = run.copy(request = run.request.copy(dependencies = run.request.dependencies - id))

/** Acceptance delays admission; dispatch later sends ContextChanged to interrupt already-running probes. */
internal fun addRunDependency(
    state: DiagnosticRunState,
    id: Long,
): DiagnosticRunState {
    fun waiting(run: ActiveDiagnosticRun?): ActiveDiagnosticRun? =
        run?.takeIf { it.stage in setOf(DiagnosticStage.Waiting, DiagnosticStage.Checking) }?.let {
            it.copy(stage = DiagnosticStage.Waiting, request = it.request.copy(dependencies = it.request.dependencies + id))
        } ?: run
    return state.copy(active = waiting(state.active), pending = waiting(state.pending))
}
