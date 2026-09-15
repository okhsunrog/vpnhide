package dev.okhsunrog.vpnhide

internal fun startConfigRecovery(
    state: ConfigOperationState,
    manual: Boolean = false,
): Transition<ConfigOperationState, ConfigOperationEffect> {
    val active = requireNotNull(state.active)
    val attempt =
        when {
            manual -> 2
            active.stage == OperationStage.Reconciling -> active.recoveryAttempt + 1
            else -> 0
        }
    val ticket = EffectTicket(active.request.id, state.nextEffect)
    return Transition(
        state.copy(
            active = active.copy(stage = OperationStage.Reconciling, ticket = ticket, recoveryAttempt = attempt),
            nextEffect = state.nextEffect + 1,
        ),
        listOf(ConfigOperationEffect.Reconcile(ticket)),
    )
}

internal fun recoverConfigOperation(
    state: ConfigOperationState,
    event: ConfigOperationEvent.RecoveryFinished,
): Transition<ConfigOperationState, ConfigOperationEffect> {
    val active = state.active ?: return Transition(state)
    if (active.stage != OperationStage.Reconciling || active.ticket != event.ticket) return Transition(state)
    val evidence = event.evidence
    if (evidence != null && validRecoveryOutcomes(active.outcomes, evidence.outcomes)) {
        val failed = evidence.outcomes.values.any { it != PhaseOutcome.Confirmed }
        return finishConfigOperation(
            state.copy(confirmed = canonicalConfigSnapshot(evidence.config), active = active.copy(outcomes = evidence.outcomes.toMap())),
            if (failed) TransitionFailure.ExecutionFailed else null,
        )
    }
    if (active.recoveryAttempt == 0) return startConfigRecovery(state)
    val result = configOperationResult(state, active, TransitionFailure.ApplicationUnknown)
    val effects =
        buildList {
            if (!active.resultDelivered) add(ConfigOperationEffect.Completed(result))
            state.queue.forEach { queued ->
                add(
                    ConfigOperationEffect.Completed(
                        ConfigOperationResult(
                            queued.id,
                            queued.spec.phases.associateWith { PhaseOutcome.NotAttempted },
                            TransitionFailure.MutationPaused,
                        ),
                    ),
                )
            }
        }
    return Transition(state.copy(active = active.copy(stage = OperationStage.Held, resultDelivered = true), queue = emptyList()), effects)
}

/** Do not reopen the lane on partial evidence or contradictory acknowledgement. */
internal fun validRecoveryOutcomes(
    previous: Map<ConfigPhase, PhaseOutcome>,
    recovered: Map<ConfigPhase, PhaseOutcome>,
): Boolean =
    previous.keys == recovered.keys &&
        previous.all { (phase, old) ->
            when (old) {
                PhaseOutcome.Unknown, PhaseOutcome.Running -> recovered[phase] in setOf(PhaseOutcome.Confirmed, PhaseOutcome.FailedKnown)
                else -> recovered[phase] == old
            }
        }
