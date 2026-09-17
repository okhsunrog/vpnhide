package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.EffectTicket
import dev.okhsunrog.vpnhide.Transition
import dev.okhsunrog.vpnhide.TransitionFailure

internal enum class DiagnosticStage { Waiting, Checking, Core, Slow, Verifying, Draining }

internal enum class RunOutcome { NotStarted, Completed, Interrupted, Failed }

internal data class DiagnosticRequest(
    val plan: List<ProbePlanEntry>,
    val captureId: Long? = null,
    val dependencies: Set<Long> = emptySet(),
    val automatic: Boolean = false,
) {
    init {
        require(plan.isNotEmpty() && plan.all { it.id.isNotBlank() })
        require(plan.map { it.id }.distinct().size == plan.size)
    }
}

internal data class DiagnosticAttempt(
    val id: Long,
    val outcome: RunOutcome,
    val failure: TransitionFailure? = null,
    val measurement: DiagnosticMeasurement? = null,
    /** The blocking eligibility of a NotStarted attempt; null for every other outcome. */
    val eligibility: DiagnosticEligibility? = null,
) {
    /**
     * The run never started because of a current condition (VPN off, self excluded,
     * restart pending, a change applying). That is not a failure of the suite: the
     * condition is named by the eligibility while it holds, and is stale history
     * once it clears, so no surface should word it as "the check failed" (I13).
     */
    val blocked: Boolean
        get() = outcome == RunOutcome.NotStarted && failure == null && eligibility != null
}

internal data class ActiveDiagnosticRun(
    val id: Long,
    val request: DiagnosticRequest,
    val stage: DiagnosticStage = DiagnosticStage.Waiting,
    val ticket: EffectTicket = EffectTicket(id, 0),
    val context: MeasurementContext? = null,
    val outcomes: Map<String, CheckOutcome> = emptyMap(),
    val ending: RunOutcome = RunOutcome.Interrupted,
    val failure: TransitionFailure? = null,
    val startedAt: Long? = null,
    val endContext: MeasurementContext? = null,
)

/** Pure core behind DiagnosticsCache. Runtime ownership, deadlines and evidence capture are adapters. */
internal data class DiagnosticRunState(
    val active: ActiveDiagnosticRun? = null,
    val pending: ActiveDiagnosticRun? = null,
    val lastAttempt: DiagnosticAttempt? = null,
    val lastComplete: DiagnosticMeasurement? = null,
    val nextId: Long = 1,
    val nextEffect: Long = 1,
    val quarantined: Boolean = false,
    val quarantineTicket: EffectTicket? = null,
)

internal sealed interface DiagnosticRunEvent {
    data class Request(
        val request: DiagnosticRequest,
    ) : DiagnosticRunEvent

    data class ContextReady(
        val ticket: EffectTicket,
        val context: MeasurementContext,
    ) : DiagnosticRunEvent

    /** The fresh Checking observation found the suite blocked: no probe starts and the reason is retained. */
    data class NotEligible(
        val ticket: EffectTicket,
        val eligibility: DiagnosticEligibility,
    ) : DiagnosticRunEvent

    data class Failed(
        val ticket: EffectTicket,
        val reason: TransitionFailure,
    ) : DiagnosticRunEvent

    data class ProbesFinished(
        val ticket: EffectTicket,
        val outcomes: Map<String, CheckOutcome>,
    ) : DiagnosticRunEvent

    data class ContextChanged(
        val known: Boolean = true,
    ) : DiagnosticRunEvent

    data class OperationSettled(
        val id: Long,
        val failure: TransitionFailure? = null,
    ) : DiagnosticRunEvent

    data class OperationAccepted(
        val id: Long,
    ) : DiagnosticRunEvent

    data class Cancel(
        val id: Long,
    ) : DiagnosticRunEvent

    data class Expired(
        val id: Long,
    ) : DiagnosticRunEvent

    data class DrainExpired(
        val ticket: EffectTicket,
    ) : DiagnosticRunEvent

    data class Drained(
        val ticket: EffectTicket,
        val quiescent: Boolean,
    ) : DiagnosticRunEvent

    data class ResourceRecovered(
        val ticket: EffectTicket,
    ) : DiagnosticRunEvent
}

internal sealed interface DiagnosticRunEffect {
    data class Accepted(
        val id: Long,
        val joined: Boolean = false,
    ) : DiagnosticRunEffect

    data class Rejected(
        val reason: TransitionFailure,
    ) : DiagnosticRunEffect

    data class ArmDeadline(
        val id: Long,
    ) : DiagnosticRunEffect

    data class ArmDrainDeadline(
        val ticket: EffectTicket,
    ) : DiagnosticRunEffect

    data class Observe(
        val ticket: EffectTicket,
    ) : DiagnosticRunEffect

    data class Probe(
        val ticket: EffectTicket,
        val stage: DiagnosticStage,
        val plan: List<ProbePlanEntry>,
    ) : DiagnosticRunEffect

    data class Drain(
        val ticket: EffectTicket,
    ) : DiagnosticRunEffect

    data class Completed(
        val attempt: DiagnosticAttempt,
    ) : DiagnosticRunEffect
}

internal fun reduceDiagnosticRun(
    state: DiagnosticRunState,
    event: DiagnosticRunEvent,
    now: Long,
): Transition<DiagnosticRunState, DiagnosticRunEffect> =
    when (event) {
        is DiagnosticRunEvent.Request -> {
            requestDiagnosticRun(state, event.request)
        }

        is DiagnosticRunEvent.ContextReady -> {
            receiveRunContext(state, event, now)
        }

        is DiagnosticRunEvent.NotEligible -> {
            blockDiagnosticRun(state, event, now)
        }

        is DiagnosticRunEvent.ProbesFinished -> {
            receiveRunProbes(state, event)
        }

        is DiagnosticRunEvent.Failed -> {
            failDiagnosticRun(state, event, now)
        }

        is DiagnosticRunEvent.ContextChanged -> {
            interruptRunForContext(state, event.known, now)
        }

        is DiagnosticRunEvent.OperationSettled -> {
            settleRunDependency(state, event, now)
        }

        is DiagnosticRunEvent.OperationAccepted -> {
            Transition(addRunDependency(state, event.id))
        }

        is DiagnosticRunEvent.Cancel -> {
            stopRequestedRun(state, event.id, TransitionFailure.Cancelled, now)
        }

        is DiagnosticRunEvent.Expired -> {
            stopRequestedRun(state, event.id, TransitionFailure.DeadlineExceeded, now)
        }

        is DiagnosticRunEvent.DrainExpired -> {
            drainDiagnosticRun(state, DiagnosticRunEvent.Drained(event.ticket, false), now)
        }

        is DiagnosticRunEvent.Drained -> {
            drainDiagnosticRun(state, event, now)
        }

        is DiagnosticRunEvent.ResourceRecovered -> {
            if (state.quarantined && state.quarantineTicket == event.ticket) {
                Transition(state.copy(quarantined = false, quarantineTicket = null))
            } else {
                Transition(state)
            }
        }
    }

/** Identity is plan and capture; dependencies are admission state (an active run already carries every accepted one). */
private fun matchingRun(
    run: ActiveDiagnosticRun?,
    request: DiagnosticRequest,
): Boolean =
    run != null && run.stage != DiagnosticStage.Draining &&
        (request.automatic || !run.request.automatic) &&
        run.request.copy(automatic = false, dependencies = emptySet()) == request.copy(automatic = false, dependencies = emptySet())

private fun requestDiagnosticRun(
    state: DiagnosticRunState,
    request: DiagnosticRequest,
): Transition<DiagnosticRunState, DiagnosticRunEffect> {
    if (state.quarantined) return Transition(state, listOf(DiagnosticRunEffect.Rejected(TransitionFailure.ResourceUnavailable)))
    val shared = listOf(state.active, state.pending).firstOrNull { matchingRun(it, request) }
    if (shared != null) return Transition(state, listOf(DiagnosticRunEffect.Accepted(shared.id, joined = true)))
    if (state.active != null && (state.pending != null || request.automatic)) {
        return Transition(state, listOf(DiagnosticRunEffect.Rejected(TransitionFailure.Busy)))
    }
    val run = ActiveDiagnosticRun(state.nextId, request.copy(plan = request.plan.toList(), dependencies = request.dependencies.toSet()))
    val next = state.copy(nextId = state.nextId + 1)
    val transition = if (state.active == null) admitDiagnosticRun(next, run) else Transition(next.copy(pending = run))
    return transition.copy(
        effects =
            listOf(DiagnosticRunEffect.Accepted(run.id), DiagnosticRunEffect.ArmDeadline(run.id)) + transition.effects,
    )
}

internal fun admitDiagnosticRun(
    state: DiagnosticRunState,
    run: ActiveDiagnosticRun,
): Transition<DiagnosticRunState, DiagnosticRunEffect> =
    if (run.request.dependencies.isNotEmpty()) {
        Transition(
            state.copy(active = run),
        )
    } else {
        dispatchDiagnosticStage(state.copy(active = run), DiagnosticStage.Checking)
    }

internal fun dispatchDiagnosticStage(
    state: DiagnosticRunState,
    stage: DiagnosticStage,
): Transition<DiagnosticRunState, DiagnosticRunEffect> {
    val active = requireNotNull(state.active)
    val ticket = EffectTicket(active.id, state.nextEffect)
    val effect =
        when (stage) {
            DiagnosticStage.Checking, DiagnosticStage.Verifying -> DiagnosticRunEffect.Observe(ticket)
            DiagnosticStage.Core, DiagnosticStage.Slow -> DiagnosticRunEffect.Probe(ticket, stage, active.request.plan)
            DiagnosticStage.Draining -> DiagnosticRunEffect.Drain(ticket)
            DiagnosticStage.Waiting -> error("Waiting has no I/O effect")
        }
    val effects = if (stage == DiagnosticStage.Draining) listOf(effect, DiagnosticRunEffect.ArmDrainDeadline(ticket)) else listOf(effect)
    return Transition(state.copy(active = active.copy(stage = stage, ticket = ticket), nextEffect = state.nextEffect + 1), effects)
}

private fun receiveRunContext(
    state: DiagnosticRunState,
    event: DiagnosticRunEvent.ContextReady,
    now: Long,
): Transition<DiagnosticRunState, DiagnosticRunEffect> {
    val active = state.active ?: return Transition(state)
    if (active.ticket != event.ticket) return Transition(state)
    return when (active.stage) {
        DiagnosticStage.Checking -> {
            dispatchDiagnosticStage(
                state.copy(active = active.copy(context = event.context, startedAt = now)),
                DiagnosticStage.Core,
            )
        }

        DiagnosticStage.Verifying -> {
            val stable = sameMeasurementConditions(requireNotNull(active.context), event.context)
            finishDiagnosticRun(
                state.copy(active = active.copy(endContext = event.context)),
                if (stable) RunOutcome.Completed else RunOutcome.Interrupted,
                if (stable) null else TransitionFailure.ContextChanged,
                now,
            )
        }

        else -> {
            Transition(state)
        }
    }
}

private fun receiveRunProbes(
    state: DiagnosticRunState,
    event: DiagnosticRunEvent.ProbesFinished,
): Transition<DiagnosticRunState, DiagnosticRunEffect> {
    val active = state.active ?: return Transition(state)
    if (active.ticket != event.ticket || active.stage !in setOf(DiagnosticStage.Core, DiagnosticStage.Slow)) return Transition(state)
    val planned =
        active.request.plan
            .map { it.id }
            .toSet()
    require(event.outcomes.keys.all { it in planned && it !in active.outcomes })
    val next = state.copy(active = active.copy(outcomes = active.outcomes + event.outcomes.toMap()))
    return dispatchDiagnosticStage(next, if (active.stage == DiagnosticStage.Core) DiagnosticStage.Slow else DiagnosticStage.Verifying)
}

internal fun finishDiagnosticRun(
    state: DiagnosticRunState,
    outcome: RunOutcome,
    failure: TransitionFailure?,
    now: Long,
    eligibility: DiagnosticEligibility? = null,
): Transition<DiagnosticRunState, DiagnosticRunEffect> {
    val active = requireNotNull(state.active)
    val measurement =
        active.context?.let {
            DiagnosticMeasurement(
                active.id,
                it,
                active.request.plan,
                active.outcomes,
                outcome == RunOutcome.Completed,
                outcome == RunOutcome.Interrupted,
                now,
                startedAt = requireNotNull(active.startedAt),
                endContext = active.endContext,
            )
        }
    val attempt = DiagnosticAttempt(active.id, outcome, failure, measurement, eligibility)
    val next =
        state.copy(
            active = null,
            pending = null,
            lastAttempt = attempt,
            lastComplete = if (outcome == RunOutcome.Completed) measurement else state.lastComplete,
        )
    val pending = state.pending
    val successor: Transition<DiagnosticRunState, DiagnosticRunEffect> =
        when {
            pending == null -> {
                Transition(next)
            }

            !state.quarantined -> {
                admitDiagnosticRun(next, pending)
            }

            else -> {
                Transition(
                    next,
                    listOf(
                        DiagnosticRunEffect.Completed(
                            DiagnosticAttempt(pending.id, RunOutcome.NotStarted, TransitionFailure.ResourceUnavailable),
                        ),
                    ),
                )
            }
        }
    return successor.copy(effects = listOf(DiagnosticRunEffect.Completed(attempt)) + successor.effects)
}
